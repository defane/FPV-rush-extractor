use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone, Utc};
use filetime::FileTime;
use fpv_core::{Clip, ClipEntry, ClipExit, DetectionRun, Flight, MediaInfo, Session, SourceFile};
use fpv_media::{FfmpegInvoking, MediaProbe, ThumbnailGenerator};
use fpv_store::{LibraryStore, StoreError};

use crate::ordering::file_name;
use crate::{DEFAULT_GAP, DiscoveredFile, FolderPattern, GoProChapter, by_sequence, by_timestamp, sha256,
            sorted, source_label};

/// What the Swift app calls a recording, by extension. `.LRV` (GoPro's
/// low-res preview) and `.THM` (its thumbnail) are deliberately absent.
pub const MEDIA_EXTENSIONS: [&str; 3] = ["avi", "mov", "mp4"];

/// The `DetectionRun` recorded for an action-cam file: no detector ran, and the
/// run says so, so a clip set stays explainable.
pub const WHOLE_FILE_ALGORITHM: &str = "whole-file-1";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not read the card: {0}")]
    Card(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone)]
pub enum ImportEvent {
    Discovered { count: usize, sessions: usize },
    Copying(PathBuf),
    Imported(Box<SourceFile>),
    Skipped(PathBuf),
    Failed(PathBuf, String),
    /// An action-cam take saved as a flight, `parts` chapters long.
    Flight { session: String, order: i64, parts: usize },
}

#[derive(Debug, Default)]
pub struct ImportReport {
    /// Sessions that received files: created, or found by name and joined.
    pub sessions: Vec<Session>,
    pub imported: Vec<SourceFile>,
    pub skipped: Vec<PathBuf>,
    /// Kept rather than thrown: one unreadable file must not abandon a card
    /// halfway through, leaving the library in a state nobody can explain.
    pub failed: Vec<(PathBuf, String)>,
    /// Whole-file flights made for action-cam takes. DVR flights come later,
    /// from segmentation.
    pub flights: Vec<Flight>,
}

/// One file of a group, probed: the card file is probed before anything is
/// copied, because an action cam's session depends on its recorded time.
type Probed = (DiscoveredFile, MediaInfo);

/// Card in, library out: discovers recordings, groups them into sessions, copies
/// them and records what arrived.
///
/// The card is only ever read. Copying is unconditional and the duplication of
/// disk space is accepted (CLAUDE.md 9.3); what is not accepted is importing the
/// same recording twice, so a file already in the library is skipped rather than
/// copied again under a second name.
pub struct LibraryImporter<'a> {
    ffmpeg: &'a dyn FfmpegInvoking,
    store: &'a LibraryStore,
    posters: Option<PathBuf>,
    folders: FolderPattern,
    muted: bool,
}

impl<'a> LibraryImporter<'a> {
    pub fn new(ffmpeg: &'a dyn FfmpegInvoking, store: &'a LibraryStore) -> Self {
        Self { ffmpeg, store, posters: None, folders: FolderPattern::session_only(), muted: false }
    }

    /// Persist an audio policy only for recordings newly registered by this import.
    pub fn without_sound(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// How session folders are named in the import folder. Without it, the
    /// session's name alone, as the CLI lays a library out.
    pub fn folders_named(mut self, pattern: FolderPattern) -> Self {
        self.folders = pattern;
        self
    }

    /// Where action-cam takes get their poster frames. Without it, a
    /// `thumbnails` folder inside the library, as the CLI lays a library out;
    /// the app passes its cache folder.
    pub fn posters_in(mut self, directory: impl Into<PathBuf>) -> Self {
        self.posters = Some(directory.into());
        self
    }

    /// Every recording on a card, in card order. Subfolders are walked: a GoPro
    /// files its takes under `DCIM/100GOPRO`, a DJI under `DCIM/100MEDIA`.
    pub fn discover(&self, card: &Path) -> std::io::Result<Vec<DiscoveredFile>> {
        let mut found = vec![];
        walk(card, 0, &mut found)?;
        Ok(sorted(found.into_iter().map(DiscoveredFile::new).collect()))
    }

    /// - `name`: the session. A card that splits into several gets a numbered
    ///   suffix, so nothing is silently merged. A session already holding that
    ///   name is joined rather than duplicated: an action-cam card imported
    ///   under the name of the day's DVR session lands beside its flights.
    /// - `date`: the flying date for new sessions. `None` is honest for the
    ///   analog DVR, which has no clock (CLAUDE.md 5); an action-cam session
    ///   takes the time of its first take instead.
    pub fn import_card(&self, card: &Path, library: &Path, name: &str,
                       date: Option<DateTime<Utc>>, mut progress: impl FnMut(ImportEvent))
        -> Result<ImportReport, ImportError> {
        let mut report = ImportReport::default();
        let files = self.discover(card)?;
        let count = files.len();

        let probe = MediaProbe::new(self.ffmpeg);
        let mut segmented: Vec<Probed> = vec![];
        let mut whole: Vec<Probed> = vec![];
        for file in files {
            match probe.probe(&file.path) {
                Ok(media) if media.is_analog_dvr() => segmented.push((file, media)),
                Ok(media) => {
                    let media = MediaInfo { creation_date: cam_timestamp(&file, &media), ..media };
                    whole.push((file, media));
                }
                Err(error) => fail(&mut report, &mut progress, &file.path, error.to_string()),
            }
        }

        let mut groups: Vec<(bool, Vec<Probed>)> = vec![];
        if !segmented.is_empty() {
            let dvr = if segmented.iter().any(|(f, _)| f.sequence_number.is_some()) {
                by_sequence(segmented, |(f, _)| f.sequence_number)
            } else {
                by_timestamp(segmented, DEFAULT_GAP, |(f, _)| f.timestamp)
            };
            groups.extend(dvr.into_iter().map(|g| (false, g)));
        }
        if !whole.is_empty() {
            groups.extend(by_timestamp(whole, DEFAULT_GAP, |(_, m)| m.creation_date)
                .into_iter().map(|g| (true, g)));
        }
        progress(ImportEvent::Discovered { count, sessions: groups.len() });

        // Same footage under a different name — a reformatted card restarts the
        // PICT counter — is caught by content, against everything the library
        // already holds. A file that cannot be hashed is not skipped: the copy
        // will fail on its own and land in the report.
        let mut seen = self.store.checksums().unwrap_or_default();
        let known = self.store.imported_paths()?;
        let total = groups.len();
        for (index, (whole_file, group)) in groups.into_iter().enumerate() {
            let session_name = if total == 1 { name.to_owned() } else { format!("{name} ({})", index + 1) };
            // The session's date names its folders, so it is settled first: an
            // existing session keeps its own, a new one takes the given date or,
            // for action cams, the first take's time.
            let existing = self.store.sessions()?.into_iter().find(|s| s.name == session_name);
            let first_take = group.iter().filter_map(|(_, m)| m.creation_date).min();
            let session_date = match &existing {
                Some(session) => session.date,
                None => date.or(if whole_file { first_take } else { None }),
            };

            // Which of these are actually new. Answered before the session is
            // touched, so re-importing the same card is a no-op.
            let mut pending = vec![];
            for (file, media) in group {
                let folder = self.folders.render(session_date, &session_name, source_label(&file, &media));
                let destination = library.join(folder).join(file.name());
                let on_disk = destination.exists();
                if on_disk && known.contains(destination.to_string_lossy().as_ref()) {
                    report.skipped.push(file.path.clone());
                    progress(ImportEvent::Skipped(file.path));
                    continue;
                }
                // On disk but in no session: a session removed from the
                // library with its recordings kept. The copy is taken back as
                // it is, never re-copied over; a different file of that name
                // is never overwritten either.
                let adopt = on_disk && same_size(&file.path, &destination);
                if on_disk && !adopt {
                    fail(&mut report, &mut progress, &file.path,
                         format!("a different file is already at {}", destination.display()));
                    continue;
                }
                let checksum = sha256(&file.path).ok();
                if checksum.as_ref().is_some_and(|c| seen.contains(c)) {
                    report.skipped.push(file.path.clone());
                    progress(ImportEvent::Skipped(file.path));
                    continue;
                }
                if let Some(checksum) = &checksum {
                    seen.insert(checksum.clone());
                }
                pending.push((file, media, checksum, destination, adopt));
            }
            if pending.is_empty() {
                continue;
            }

            let session = match existing {
                Some(existing) => existing,
                None => {
                    let session = Session::new(session_name.clone(), session_date);
                    self.store.save_session(&session)?;
                    session
                }
            };
            report.sessions.push(session.clone());

            let mut saved: Vec<(SourceFile, Option<GoProChapter>)> = vec![];
            for (file, media, checksum, destination, adopt) in pending {
                let copied = if adopt {
                    Ok(destination)
                } else {
                    progress(ImportEvent::Copying(file.path.clone()));
                    copy_preserving_mtime(&file.path, &destination)
                };
                match copied {
                    Ok(destination) => {
                        let mut record = SourceFile::new(
                            session.id, file.path.to_string_lossy(), destination.to_string_lossy(),
                            file.sequence_number.or(file.gopro.map(|g| g.order_key())), media);
                        record.muted = self.muted;
                        record.checksum = checksum;
                        report.imported.push(record.clone());
                        progress(ImportEvent::Imported(Box::new(record.clone())));
                        saved.push((record, file.gopro));
                    }
                    Err(error) => fail(&mut report, &mut progress, &file.path, error.to_string()),
                }
            }
            let files: Vec<SourceFile> = saved.iter().map(|(f, _)| f.clone()).collect();
            self.store.save_source_files(&files)?;

            if whole_file {
                let posters = self.posters.clone().unwrap_or_else(|| library.join("thumbnails"));
                for flight in self.save_whole_file_flights(&session, &saved, &posters)? {
                    progress(ImportEvent::Flight {
                        session: session.name.clone(), order: flight.0.order_in_session,
                        parts: flight.1,
                    });
                    report.flights.push(flight.0);
                }
            }
        }
        Ok(report)
    }

    /// Registers recordings already in the import folder into `session`, where
    /// they are: no copy, nothing moved. For "Load from destination folder":
    /// a session removed with its recordings kept, a library rebuilt over an
    /// import folder. A recording the library knows, by path or by content,
    /// is skipped; an action-cam take becomes its whole-file flight, a DVR
    /// recording waits for segmentation.
    pub fn register_in_place(&self, files: Vec<DiscoveredFile>, session: &Session,
                             mut progress: impl FnMut(ImportEvent)) -> Result<ImportReport, ImportError> {
        let mut report = ImportReport::default();
        let known = self.store.imported_paths()?;
        let mut seen = self.store.checksums().unwrap_or_default();
        let probe = MediaProbe::new(self.ffmpeg);
        let mut saved: Vec<(SourceFile, Option<GoProChapter>)> = vec![];
        for file in files {
            if known.contains(file.path.to_string_lossy().as_ref()) {
                report.skipped.push(file.path.clone());
                progress(ImportEvent::Skipped(file.path));
                continue;
            }
            let media = match probe.probe(&file.path) {
                Ok(media) if media.is_analog_dvr() => media,
                Ok(media) => MediaInfo { creation_date: cam_timestamp(&file, &media), ..media },
                Err(error) => {
                    fail(&mut report, &mut progress, &file.path, error.to_string());
                    continue;
                }
            };
            let checksum = sha256(&file.path).ok();
            if checksum.as_ref().is_some_and(|c| !seen.insert(c.clone())) {
                report.skipped.push(file.path.clone());
                progress(ImportEvent::Skipped(file.path));
                continue;
            }
            let path = file.path.to_string_lossy().into_owned();
            let mut record = SourceFile::new(session.id, path.clone(), path,
                                             file.sequence_number.or(file.gopro.map(|g| g.order_key())), media);
            record.muted = self.muted;
            record.checksum = checksum;
            report.imported.push(record.clone());
            progress(ImportEvent::Imported(Box::new(record.clone())));
            saved.push((record, file.gopro));
        }
        if saved.is_empty() {
            return Ok(report);
        }
        let files: Vec<SourceFile> = saved.iter().map(|(f, _)| f.clone()).collect();
        self.store.save_source_files(&files)?;
        report.sessions.push(session.clone());
        let takes: Vec<_> = saved.into_iter().filter(|(f, _)| !f.media.is_analog_dvr()).collect();
        if !takes.is_empty() {
            let posters = self.posters.clone().unwrap_or_else(|| PathBuf::from("thumbnails"));
            for flight in self.save_whole_file_flights(session, &takes, &posters)? {
                progress(ImportEvent::Flight {
                    session: session.name.clone(), order: flight.0.order_in_session, parts: flight.1,
                });
                report.flights.push(flight.0);
            }
        }
        Ok(report)
    }

    /// One flight per action-cam take: the chapters of a GoPro recording join
    /// into one, exactly as a flight the DVR cut across two files does, so the
    /// player, the tags and export need nothing new. Appended after the
    /// session's existing flights — the DVR has no clock to interleave by.
    fn save_whole_file_flights(&self, session: &Session, files: &[(SourceFile, Option<GoProChapter>)],
                               posters: &Path) -> Result<Vec<(Flight, usize)>, StoreError> {
        let mut order = self.store.flights(session.id)?.iter()
            .map(|(f, _)| f.order_in_session).max().unwrap_or(0);
        let thumbnails = ThumbnailGenerator::new(self.ffmpeg);
        let mut flights = vec![];
        let mut index = 0;
        while index < files.len() {
            let recording = files[index].1.map(|g| g.recording);
            let mut end = index + 1;
            while recording.is_some() && end < files.len()
                && files[end].1.map(|g| g.recording) == recording {
                end += 1;
            }
            order += 1;
            let flight = Flight::new(session.id, order);
            for (part, (file, _)) in files[index..end].iter().enumerate() {
                let media = &file.media;
                let frames = media.frame_count.filter(|&n| n > 0)
                    .unwrap_or_else(|| media.frame_rate.frame_index(media.duration));
                let mut clip = Clip {
                    entry: if part == 0 { ClipEntry::Unknown } else { ClipEntry::Continuation },
                    exit: ClipExit::Eof,
                    // No detector judged these boundaries: the pilot pressed
                    // record and stop.
                    confidence: 1.0,
                    ..Clip::new(flight.id, file.id, 0, frames)
                };
                // A poster is a convenience: its failure costs a thumbnail, not
                // the take.
                match thumbnails.generate_for_clip(&clip, file, posters, 480) {
                    Ok(path) => clip.thumbnail_path = Some(path.to_string_lossy().into_owned()),
                    Err(error) => log::warn!("no poster for {}: {error}", file.imported_path),
                }
                let run = DetectionRun::new(file.id, WHOLE_FILE_ALGORITHM, BTreeMap::new());
                self.store.replace_clips(file.id, &[clip], std::slice::from_ref(&flight), &run)?;
            }
            flights.push((flight, end - index));
            index = end;
        }
        Ok(flights)
    }
}

fn same_size(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.len() == b.len(),
        _ => false,
    }
}

fn fail(report: &mut ImportReport, progress: &mut impl FnMut(ImportEvent), path: &Path,
        reason: String) {
    report.failed.push((path.to_path_buf(), reason.clone()));
    progress(ImportEvent::Failed(path.to_path_buf(), reason));
}

/// An action cam's recorded time, for session grouping and dating.
///
/// GoPro writes its local wall-clock time into `creation_time` and labels it
/// UTC — the MP4 header has no time zone field. Read as UTC, an evening take
/// in summer would date to the next day; read as local, it lands right.
fn cam_timestamp(file: &DiscoveredFile, media: &MediaInfo) -> Option<DateTime<Utc>> {
    match (media.creation_date, file.gopro) {
        (Some(stamp), Some(_)) => Local.from_local_datetime(&stamp.naive_utc()).earliest()
            .map(|local| local.to_utc()),
        (Some(stamp), None) => Some(stamp),
        (None, _) => file.timestamp,
    }
}

/// Copies through a temporary name and renames, so an interrupted copy never
/// leaves a truncated file under the real name — which the next import would
/// take for "already imported". The mtime is carried over (CLAUDE.md 2);
/// `std::fs::copy` alone does not.
pub fn copy_preserving_mtime(source: &Path, destination: &Path) -> std::io::Result<PathBuf> {
    let directory = destination.parent().expect("a destination inside a session folder");
    std::fs::create_dir_all(directory)?;
    let partial = directory.join(format!(".{}.partial", file_name(destination)));
    let result = std::fs::copy(source, &partial).and_then(|_| {
        let modified = FileTime::from_last_modification_time(&std::fs::metadata(source)?);
        filetime::set_file_mtime(&partial, modified)?;
        std::fs::rename(&partial, destination)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result.map(|_| destination.to_path_buf())
}

fn walk(directory: &Path, depth: usize, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if entry.file_type()?.is_dir() {
            if depth < 3 {
                walk(&path, depth + 1, found)?;
            }
        } else if path.extension().and_then(|e| e.to_str())
            .is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())) {
            found.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "importer_tests.rs"]
mod tests;
