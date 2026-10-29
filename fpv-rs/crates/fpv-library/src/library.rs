use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use fpv_core::{Clip, DetectionProfile, DetectionRun, FigureTag, Flight, FlightTimeline, MediaInfo, REDUCED_ALGORITHM,
               Session, SourceFile, uuid_string};
use fpv_detect::{DetectorSettings, FileDetector};
use fpv_export::{ClipExporter, ExportError};
use fpv_import::{FolderPattern, ImportEvent, LibraryImporter};
use fpv_media::{FfmpegRunner, FrameReader, MediaProbe, ThumbnailGenerator};
use fpv_pipeline::{SegmentationEvent, SegmentationReport, SegmentationService};
use fpv_proxy::{ProxyCache, ProxyGenerator};
use fpv_store::{LibraryStore, StoreError};
use uuid::Uuid;

use crate::Config;
use crate::reduce::{MARGIN_SECONDS, into_piece, kept_ranges};
use crate::views::{self, SessionView, TagView, file_name};

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Import(#[from] fpv_import::ImportError),
    #[error(transparent)]
    Export(#[from] ExportError),
    #[error(transparent)]
    Proxy(#[from] fpv_proxy::ProxyError),
    #[error(transparent)]
    Ffmpeg(#[from] fpv_media::FfmpegError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("not in the library: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    /// Left in the cloud rather than downloaded for a picture.
    #[error("{0} is only in the cloud: download it first")]
    InCloud(String),
}

pub type Result<T> = std::result::Result<T, LibraryError>;

/// Stored metadata for one clip's source; looking at Info never reads media
/// content or downloads a cloud placeholder.
#[derive(Debug, Clone)]
pub struct RecordingInfo {
    pub clip: Clip,
    pub source: SourceFile,
    pub available: bool,
    pub cloud: bool,
    pub size_bytes: Option<u64>,
}

/// How a flight is written out. `Lossless` refuses a flight whose recordings
/// disagree on their rate; the user then picks one of the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportMode {
    Lossless,
    /// One lossless file per recording.
    Parts,
    /// One file, each part resampled to the dominant rate (CLAUDE.md 2).
    Reencode,
}

/// Whether a strip frame may be the nearest keyframe rather than the exact
/// frame. Only on long-GOP footage, where the exact one costs a GOP of 4K
/// decoding (~1.7 s on 10-bit HEVC, the pilot's Osmo takes), and only when the
/// strip's frames are at least 2 s apart: on a short take several would land
/// on the same keyframe and the strip would repeat itself. The DVR's MJPEG is
/// all keyframes, so it is always exact and costs nothing extra.
fn rough_strip(media: &MediaInfo, spacing: f64) -> bool {
    !media.is_all_intra() && spacing >= 2.0
}

/// Whether a flight can play from its own files: every part from an action
/// cam. The DVR's MJPEG never can.
fn plays_directly<'m>(media: impl IntoIterator<Item = &'m MediaInfo>) -> bool {
    media.into_iter().all(|media| !media.needs_segmentation())
}

/// How the player opens a flight (`Library::playback`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Playback {
    /// From the recordings themselves, one segment after the other.
    Direct { segments: Vec<Segment> },
    /// From the flight's proxy.
    Proxy,
}

/// One recording's share of a flight, in that recording's own seconds.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    /// Where it begins on the flight's clock.
    pub offset: f64,
}

/// A source part for native playback, before display scaling or resampling.
#[derive(Debug, Clone)]
pub struct NativeSegment {
    pub segment: Segment,
    pub width: i64,
    pub height: i64,
    pub has_audio: bool,
    pub all_intra: bool,
}

/// The cache folder's contents by kind, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheUsage {
    pub proxies: u64,
    pub strips: u64,
    pub posters: u64,
    /// The boundary editor's source frames.
    pub previews: u64,
}

impl CacheUsage {
    pub fn total(&self) -> u64 {
        self.proxies + self.strips + self.posters + self.previews
    }
}

/// Marks a cached picture as just used, so the budget's trimming keeps it
/// over pictures nobody looked at for longer. Best effort: a picture whose
/// date cannot change is only trimmed sooner.
fn touch(path: &Path) {
    let _ = std::fs::File::options().write(true).open(path)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()));
}

/// Bytes under a folder, 0 if it is not there.
fn folder_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries.flatten().map(|entry| match entry.file_type() {
        Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
        Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
        Err(_) => 0,
    }).sum()
}

/// One hover-strip frame still to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripFrame {
    pub flight: String,
    pub index: usize,
    /// "Session — Flight 3", for the progress line.
    pub label: String,
}

/// What `prepare_strips` did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripsPrepared {
    pub made: usize,
    /// (flight, reason), once per flight.
    pub failed: Vec<(String, String)>,
    /// It stopped at the pictures' budget with strips still missing.
    pub at_budget: bool,
    /// Frames were left for a next batch (`prepare_strips_up_to`).
    pub more: bool,
}

/// One kind of what the cache folder holds, each in its own folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CacheKind {
    /// Playback previews, under the proxy budget.
    Proxies,
    /// Hover strips, posters and the editor's frames: the pictures, under
    /// their own budget.
    Strips,
    Posters,
    Previews,
}

impl CacheKind {
    pub const ALL: [CacheKind; 4] = [CacheKind::Proxies, CacheKind::Strips, CacheKind::Posters, CacheKind::Previews];

    fn folder(self) -> &'static str {
        match self {
            CacheKind::Proxies => "proxies",
            CacheKind::Strips => "strips",
            CacheKind::Posters => "posters",
            CacheKind::Previews => "preview",
        }
    }
}

/// What a batch export wrote, and what it could not.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkExport {
    pub written: Vec<PathBuf>,
    /// (flight, reason)
    pub failed: Vec<(String, String)>,
}

/// What moving a session's recordings into the import folder would do.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocationPreview {
    pub recordings: usize,
    pub bytes: u64,
    /// The folders they sit in now, and the ones they would move to.
    pub from: Vec<String>,
    pub to: Vec<String>,
}

/// What moving did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocationSummary {
    pub moved: usize,
    /// [recording, reason]: left where it was.
    pub failed: Vec<(String, String)>,
}

/// What freeing space on a cloud drive did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeUpSummary {
    pub freed: usize,
    pub bytes: u64,
    /// [recording, reason]: still on this disk.
    pub failed: Vec<(String, String)>,
}

/// How much of a recording a download reads at a time: progress moves on
/// after each.
const DOWNLOAD_CHUNK: usize = 8 << 20;

/// What downloading from the cloud drive did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSummary {
    pub downloaded: usize,
    pub bytes: u64,
    /// [recording, reason]: still only in the cloud.
    pub failed: Vec<(String, String)>,
}

/// How far a download from the cloud is: bytes read, and which recording.
/// A File Provider may hold a recording's first read until all of it is down,
/// so the bytes can stand still then jump; the recording count still moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadStep {
    pub done: usize,
    pub total: usize,
    /// 1-based, of `files`.
    pub file: usize,
    pub files: usize,
    pub line: String,
}

/// The file an export writes a flight to, and its extension: "<session> —
/// <title>", cleaned of path separators, `.avi` for the DVR, `.mp4` for an
/// action cam — the batch export's names, and the single export's default.
pub fn export_name(session: &str, title: &str, dvr: bool) -> (String, &'static str) {
    (fpv_import::folder_name(&format!("{session} — {title}")), if dvr { "avi" } else { "mp4" })
}

/// A session's details as the Edit Session dialog sends them back.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEdit {
    pub name: String,
    /// `YYYY-MM-DD`, or none.
    pub date: Option<String>,
    pub location: String,
    pub notes: String,
    /// The session's tags, which every flight of it carries.
    pub labels: Vec<String>,
}

/// What deleting a flight does to its recordings, for its confirmation and
/// its outcome alike.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightDeletion {
    /// Files that held nothing but this flight — pieces of a reduced DVR
    /// recording, an action-cam take and its chapters: to the trash.
    pub trashed: Vec<String>,
    /// Reduced pieces it shares with another flight: kept for that one.
    pub shared: Vec<String>,
    /// Whole DVR recordings it is part of: kept, only the flight goes — a
    /// re-segment would find it again.
    pub recordings: Vec<String>,
}

/// What reducing a session's DVR recordings to their flights would do.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReducePreview {
    /// Recordings that would be cut down to their flights.
    pub recordings: usize,
    /// Of those, recordings without a single flight: they go whole.
    pub emptied: usize,
    /// DVR recordings never segmented, left alone: nothing says where their
    /// flights are.
    pub unsegmented: usize,
    pub bytes_now: u64,
    /// Estimated from the frames kept: MJPEG frames are near enough alike in size.
    pub bytes_after: u64,
    /// Figure tags that lie wholly in the snow, lost with it.
    pub figures_lost: usize,
}

/// What a reduction did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReduceSummary {
    pub recordings: usize,
    pub pieces: usize,
    /// Bytes no longer in the import folder — in the trash until it is emptied.
    pub bytes_freed: u64,
    /// [recording, reason]: left as it was.
    pub failed: Vec<(String, String)>,
}

/// One DVR recording's reduction, planned before anything is touched.
struct ReducePlan {
    file: SourceFile,
    ranges: Vec<(i64, i64)>,
    frames: i64,
    clips: Vec<Clip>,
    tags: Vec<FigureTag>,
}

/// A folder of the import folder, as "Load from destination folder" lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub name: String,
    /// The session it loads into, read from its name.
    pub session: String,
    /// `YYYY-MM-DD`, when its name starts with one.
    pub date: Option<String>,
    pub recordings: usize,
    /// Recordings the library does not know: 0 is a folder already imported.
    /// Those deleted with their flight are not counted.
    pub new: usize,
    /// Recordings that left the library with a deleted flight and are still
    /// here: loaded again only when asked.
    pub deleted: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub flights: usize,
    pub deleted_from_card: usize,
    /// Card files that should have been deleted and were kept, and why.
    pub kept: Vec<String>,
}

/// One open library: its database and settings in the app's configuration
/// folder, its footage under `root`, its disposable derivatives in the cache
/// folder, and the tools to work on it.
pub struct Library {
    /// The import folder: the imported recordings, and nothing else.
    pub root: PathBuf,
    /// The configuration folder: `library.sqlite` and `config.json`.
    pub config_directory: PathBuf,
    /// The cache folder — `~/.cache/fpv_rush_extractor`: posters, playback
    /// proxies, hover strips and editor previews. All of it rebuilds on demand
    /// (`poster` remakes a missing poster), so it belongs where backups skip
    /// and cleaners may empty, not with the footage.
    pub cache_directory: PathBuf,
    pub store: LibraryStore,
    config: Config,
    tools: Option<PathBuf>,
    /// The cloud drive the import folder may sit in; the tests stand in their
    /// own, there being no File Provider on Linux.
    cloud: CloudDrive,
}

/// What the library asks of a cloud drive, as plain functions so the tests
/// can stand in for one.
#[derive(Clone, Copy)]
struct CloudDrive {
    /// Online Only: reading it downloads it (`fpv_media::only_in_cloud`).
    only_in_cloud: fn(&Path) -> bool,
    /// In a cloud drive's folder at all (`fpv_media::on_cloud_drive`).
    on_drive: fn(&Path) -> bool,
    /// Makes it Online Only again (`fpv_media::free_up_space`).
    free_up_space: fn(&Path) -> std::io::Result<()>,
}

impl Default for CloudDrive {
    fn default() -> Self {
        Self { only_in_cloud: fpv_media::only_in_cloud, on_drive: fpv_media::on_cloud_drive,
               free_up_space: fpv_media::free_up_space }
    }
}

impl Library {
    /// Opens the library configured in `config_directory`, creating the
    /// folder and a default `config.json` on first run.
    pub fn open(config_directory: &Path, cache_directory: &Path, tools: Option<PathBuf>)
        -> Result<Self> {
        let config = Config::load_or_create(config_directory)?;
        Self::open_with(config_directory, cache_directory, config, tools)
    }

    /// Opens with an explicit configuration (what `open` read, possibly
    /// overridden), then tidies what earlier runs left: stale posters and
    /// strips, and a proxy index out of step with the disk.
    ///
    /// A database left in the media folder by an earlier version is moved
    /// into the configuration folder first. Every path inside it is absolute,
    /// so it opens unchanged from there. Proxies, strips and previews an
    /// earlier version kept beside the footage are deleted: a cache, rebuilt
    /// under the cache folder on demand.
    pub fn open_with(config_directory: &Path, cache_directory: &Path, config: Config,
                     tools: Option<PathBuf>) -> Result<Self> {
        let database = config_directory.join("library.sqlite");
        let legacy = config.media_root.join("library.sqlite");
        if !database.exists() && legacy.is_file() {
            std::fs::create_dir_all(config_directory)?;
            std::fs::copy(&legacy, &database)?;
            std::fs::remove_file(&legacy)?;
            log::info!("moved the library database from {} to {}", legacy.display(),
                       database.display());
        }
        // Made up front: on a first run nothing exists yet, and the player's
        // loopback server needs the folder it serves from.
        std::fs::create_dir_all(&config.media_root)?;
        std::fs::create_dir_all(cache_directory)?;
        let store = LibraryStore::open(&database)?;
        let legacy_proxies = config.media_root.join("proxies");
        if legacy_proxies.is_dir() {
            let _ = std::fs::remove_dir_all(&legacy_proxies);
            store.delete_all_proxies()?;
            log::info!("dropped the proxies kept in {}", legacy_proxies.display());
        }
        for legacy in ["strips", "preview"] {
            let _ = std::fs::remove_dir_all(config.media_root.join("thumbnails").join(legacy));
        }
        let library = Self {
            root: config.media_root.clone(), config_directory: config_directory.to_path_buf(),
            cache_directory: cache_directory.to_path_buf(), store, config, tools,
            cloud: CloudDrive::default(),
        };
        library.move_posters_into_cache()?;
        library.sweep_orphaned_thumbnails()?;
        if let Err(error) = library.trim_pictures() {
            log::warn!("could not bring the pictures under their budget: {error}");
        }
        if let Err(error) = library.proxy_cache().reconcile() {
            log::warn!("could not reconcile the proxy cache: {error}");
        }
        Ok(library)
    }

    fn ffmpeg(&self) -> FfmpegRunner {
        FfmpegRunner::new(self.tools.clone())
    }

    fn posters(&self) -> PathBuf {
        self.cache_directory.join("posters")
    }

    /// Posters an earlier build kept beside the footage (`<import
    /// folder>/thumbnails`) move into the cache, and their clips are pointed at
    /// the new path; a `thumbnails` folder left empty goes. A poster already
    /// missing is left for `poster` to remake.
    fn move_posters_into_cache(&self) -> Result<()> {
        let posters = self.posters();
        let mut emptied = HashSet::new();
        for session in self.store.sessions()? {
            for (_, clips) in self.store.flights(session.id)? {
                for clip in clips {
                    let Some(old) = clip.thumbnail_path.as_deref().map(PathBuf::from) else { continue };
                    if old.starts_with(&posters) || !old.is_file() {
                        continue;
                    }
                    std::fs::create_dir_all(&posters)?;
                    let new = posters.join(old.file_name().unwrap_or_default());
                    if std::fs::rename(&old, &new).is_err() {
                        // Another disk: copy, then let go of the original.
                        std::fs::copy(&old, &new)?;
                        let _ = std::fs::remove_file(&old);
                    }
                    if let Some(parent) = old.parent() {
                        emptied.insert(parent.to_path_buf());
                    }
                    self.store.update_clip(&Clip {
                        thumbnail_path: Some(new.to_string_lossy().into_owned()), ..clip
                    })?;
                }
            }
        }
        emptied.insert(self.root.join("thumbnails"));
        for folder in emptied {
            // Only succeeds on an empty folder, which is the point.
            let _ = std::fs::remove_dir(folder);
        }
        Ok(())
    }

    /// Reads the camera out of every action-cam take imported before the
    /// library kept it, once: a take that names none is recorded as such. A
    /// recording missing from disk is left for a later run. Returns how many
    /// were named, so the UI refreshes only when something changed.
    pub fn identify_cameras(&self) -> Result<usize> {
        let ffmpeg = self.ffmpeg();
        let probe = MediaProbe::new(&ffmpeg);
        let mut named = 0;
        for file in self.store.unidentified_cameras()? {
            let path = Path::new(&file.imported_path);
            if !path.is_file() || self.only_in_cloud(&file) {
                continue;
            }
            let camera = match probe.probe(path) {
                Ok(media) => media.camera,
                Err(error) => {
                    log::warn!("could not read the camera of {}: {error}", path.display());
                    None
                }
            };
            named += usize::from(camera.is_some());
            self.store.set_camera(file.id, camera.as_deref())?;
        }
        Ok(named)
    }

    /// What the cache folder holds, by kind, in bytes.
    pub fn cache_usage(&self) -> Result<CacheUsage> {
        let size = |name: &str| folder_size(&self.cache_directory.join(name));
        Ok(CacheUsage { proxies: size("proxies"), strips: size("strips"), posters: size("posters"),
                        previews: size("preview") })
    }

    /// Empties the cache: playback previews (and their index), hover strips,
    /// posters and editor frames. All of it is made again on demand — posters
    /// when shown, strips by the background job, a preview when played — so
    /// nothing is lost but time. Only the folders the library made are
    /// touched, whatever else the cache folder holds. Returns what was freed.
    pub fn clear_cache(&self, kinds: &[CacheKind]) -> Result<CacheUsage> {
        let usage = self.cache_usage()?;
        let mut freed = CacheUsage::default();
        for &kind in kinds {
            match kind {
                CacheKind::Proxies => {
                    self.proxy_cache().purge_all()?;
                    freed.proxies = usage.proxies;
                }
                CacheKind::Strips => freed.strips = usage.strips,
                CacheKind::Posters => freed.posters = usage.posters,
                CacheKind::Previews => freed.previews = usage.previews,
            }
            match std::fs::remove_dir_all(self.cache_directory.join(kind.folder())) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        Ok(freed)
    }

    /// What a hover-strip frame weighs, on average over those on disk; 0
    /// before the first.
    fn strip_frame_size(&self) -> u64 {
        let Ok(entries) = std::fs::read_dir(self.cache_directory.join(CacheKind::Strips.folder())) else { return 0 };
        let sizes: Vec<u64> = entries.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).collect();
        if sizes.is_empty() { 0 } else { sizes.iter().sum::<u64>() / sizes.len() as u64 }
    }

    /// The pictures' bytes on disk: hover strips, posters, editor frames.
    fn picture_bytes(&self) -> u64 {
        [CacheKind::Strips, CacheKind::Posters, CacheKind::Previews].iter()
            .map(|kind| folder_size(&self.cache_directory.join(kind.folder()))).sum()
    }

    /// Brings the pictures back under their budget. The editor's frames go
    /// first — made for one look —, then hover strips, then posters, which
    /// every card on screen shows; within each, the one used longest ago
    /// (a cached picture is touched when used again). Whatever goes is made
    /// again when next wanted. Returns the bytes freed.
    pub fn trim_pictures(&self) -> Result<u64> {
        let budget = self.config.picture_budget_bytes();
        let mut files: Vec<(usize, std::time::SystemTime, u64, PathBuf)> = vec![];
        for (rank, kind) in [CacheKind::Previews, CacheKind::Strips, CacheKind::Posters].into_iter().enumerate() {
            let Ok(entries) = std::fs::read_dir(self.cache_directory.join(kind.folder())) else { continue };
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() && metadata.is_file() {
                    let used = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                    files.push((rank, used, metadata.len(), entry.path()));
                }
            }
        }
        let mut total: u64 = files.iter().map(|f| f.2).sum();
        files.sort_by_key(|f| (f.0, f.1));
        let mut freed = 0;
        for (_, _, size, path) in files {
            if total <= budget {
                break;
            }
            std::fs::remove_file(&path)?;
            total -= size;
            freed += size;
        }
        Ok(freed)
    }

    /// Sets both budgets and saves them to `config.json`, then brings the
    /// cache under them at once.
    pub fn set_cache_budgets(&mut self, proxies_gb: f64, pictures_gb: f64) -> Result<()> {
        for (what, gb) in [("playback previews", proxies_gb), ("pictures", pictures_gb)] {
            if !gb.is_finite() || gb < 0.1 {
                return Err(LibraryError::Invalid(format!("the {what}' budget must be at least 0.1 GB, not {gb}")));
            }
        }
        let config = Config { proxy_budget_gb: proxies_gb, picture_budget_gb: pictures_gb, ..self.config.clone() };
        config.save(&self.config_directory)?;
        self.config = config;
        // Settings is open, so no flight is playing: nothing to pin.
        self.proxy_cache().evict(&HashSet::new())?;
        self.trim_pictures()?;
        Ok(())
    }

    pub fn set_video_preferences(
        &mut self,
        mute_videos: bool,
        do_not_show_splash: bool,
    ) -> Result<()> {
        let config = Config {
            mute_videos,
            do_not_show_splash,
            ..self.config.clone()
        };
        config.save(&self.config_directory)?;
        self.config = config;
        Ok(())
    }

    pub(crate) fn proxy_cache(&self) -> ProxyCache<'_> {
        ProxyCache::new(self.cache_directory.join("proxies"), self.config.proxy_budget_bytes(),
                        &self.store)
    }

    // Reading

    /// Everything the browser shows, in one pass.
    pub fn snapshot(&self) -> Result<Vec<SessionView>> {
        let mut labels = self.store.session_labels()?;
        self.store.sessions()?.iter()
            .map(|session| {
                let files = self.store.source_files(session.id)?;
                let cloud: HashSet<Uuid> = files.iter()
                    .filter(|f| self.only_in_cloud(f)).map(|f| f.id).collect();
                let freeable: HashSet<Uuid> = files.iter()
                    .filter(|f| self.freeable(f)).map(|f| f.id).collect();
                Ok(views::session_view(session, files, self.store.flights(session.id)?,
                    views::SessionContext {
                        root: &self.root,
                        session_labels: labels.remove(&session.id).unwrap_or_default(),
                        flight_labels: self.store.flight_labels_in_session(session.id)?,
                        tags: &self.store.tags_in_session(session.id)?,
                        reduced: &self.store.reduced_files(session.id)?,
                        cloud: &cloud,
                        freeable: &freeable,
                    }))
            })
            .collect()
    }

    /// A recording the cloud drive holds online only: what merely looks at
    /// footage — posters, hover strips, the camera's name — leaves it there
    /// rather than download gigabytes nobody asked for.
    fn only_in_cloud(&self, file: &SourceFile) -> bool {
        (self.cloud.only_in_cloud)(Path::new(&file.imported_path))
    }

    /// On this disk, in a cloud drive's folder: its space can be freed.
    fn freeable(&self, file: &SourceFile) -> bool {
        (self.cloud.on_drive)(Path::new(&file.imported_path)) && !self.only_in_cloud(file)
    }

    fn in_cloud_error(file: &SourceFile) -> LibraryError {
        LibraryError::InCloud(file_name(&file.imported_path))
    }

    fn session(&self, id: &str) -> Result<Session> {
        let id = parse(id)?;
        self.store.sessions()?.into_iter().find(|s| s.id == id)
            .ok_or_else(|| LibraryError::NotFound(format!("session {id}")))
    }

    fn flight(&self, id: &str) -> Result<(Flight, Vec<Clip>, FlightTimeline)> {
        let (flight, clips) = self.store.flight(parse(id)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("flight {id}")))?;
        let parts = clips.iter()
            .map(|clip| Ok((clip.clone(), self.store.source_file(clip.source_file_id)?
                .ok_or_else(|| LibraryError::NotFound(format!("recording of clip {}", clip.id)))?)))
            .collect::<Result<Vec<_>>>()?;
        Ok((flight, clips, FlightTimeline::new(parts)))
    }

    // Import and segmentation

    /// Copies a folder of recordings into the library. `date` is `YYYY-MM-DD`,
    /// read as local midnight. With `delete_originals`, the card is freed
    /// conservatively: only files imported *in this run* go — never the
    /// skipped ones, whose bytes were not compared — and only once the library
    /// copy matches size for size.
    pub fn import(&self, folder: &Path, name: &str, date: Option<&str>, delete_originals: bool,
                  progress: impl FnMut(ImportEvent)) -> Result<ImportSummary> {
        self.import_with_audio(folder, name, date, delete_originals, false, progress)
    }

    /// Imports with a per-recording audio policy, preserving source bytes and known files.
    pub fn import_with_audio(&self, folder: &Path, name: &str, date: Option<&str>,
                  delete_originals: bool, no_sound: bool,
                  progress: impl FnMut(ImportEvent)) -> Result<ImportSummary> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LibraryError::Invalid("a session needs a name".into()));
        }
        let date = date.map(local_midnight).transpose()?;
        let ffmpeg = self.ffmpeg();
        let report = LibraryImporter::new(&ffmpeg, &self.store)
            .without_sound(no_sound)
            .posters_in(self.posters())
            .folders_named(FolderPattern::parse(&self.config.folder_pattern)
                .map_err(|e| LibraryError::Invalid(format!("folderPattern in config.json: {e}")))?)
            .import_card(folder, &self.root, name, date, progress)?;
        let mut summary = ImportSummary {
            imported: report.imported.len(), skipped: report.skipped.len(),
            failed: report.failed.len(), flights: report.flights.len(), ..Default::default()
        };
        if delete_originals {
            for file in &report.imported {
                let size = |path: &str| std::fs::metadata(path).ok().map(|m| m.len());
                let verified = size(&file.imported_path).is_some()
                    && size(&file.imported_path) == size(&file.original_path);
                if verified && std::fs::remove_file(&file.original_path).is_ok() {
                    summary.deleted_from_card += 1;
                } else {
                    summary.kept.push(format!("{} — could not verify the library copy",
                                              file_name(&file.original_path)));
                }
            }
        }
        Ok(summary)
    }

    /// The import folder's folders, for "Load from destination folder" to
    /// choose from: the session each would load into, its recordings, and how
    /// many of them the library does not know yet — none: already imported.
    /// Nothing is probed: a listing reads names only.
    pub fn import_folder_contents(&self) -> Result<Vec<FolderView>> {
        let ffmpeg = self.ffmpeg();
        let importer = LibraryImporter::new(&ffmpeg, &self.store);
        let known = self.store.imported_paths()?;
        let deleted = self.store.deleted_paths()?;
        self.session_folders_on_disk()?.into_iter().map(|folder| {
            let files = importer.discover(&folder)?;
            let name = file_name(&folder.to_string_lossy());
            let (day, session) = fpv_import::session_of_folder(&name);
            let unknown: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned())
                .filter(|path| !known.contains(path)).collect();
            let (deleted, new): (Vec<String>, Vec<String>) = unknown.into_iter().partition(|path| deleted.contains(path));
            Ok(FolderView {
                session, date: day.map(|d| d.format("%Y-%m-%d").to_string()),
                recordings: files.len(),
                new: new.len(),
                deleted: deleted.iter().map(|path| file_name(path)).collect(),
                name,
            })
        }).collect()
    }

    /// The folders directly in the import folder, hidden ones left out, by name.
    fn session_folders_on_disk(&self) -> Result<Vec<PathBuf>> {
        let mut folders: Vec<PathBuf> = std::fs::read_dir(&self.root)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.is_dir()
                && !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect();
        folders.sort();
        Ok(folders)
    }

    /// "Load from destination folder": registers the recordings of the chosen
    /// folders of the import folder that the library does not know, where they
    /// are — no copy. Folders are named, never given as paths: only the import
    /// folder's own are reachable. Each folder is a session, read back from
    /// its name (`session_of_folder`): the folders of one session and day (one
    /// per source) join one session, and a session of that name and day
    /// already in the library is joined. Folders holding nothing new create
    /// nothing. A recording deleted with its flight is left where it is,
    /// unless its folder is among `restore`: then it comes back too, into the
    /// session the folder loads into, beside the flights still there.
    pub fn load_import_folders(&self, chosen: &[String], restore: &[String],
                               mut progress: impl FnMut(ImportEvent)) -> Result<ImportSummary> {
        let ffmpeg = self.ffmpeg();
        let importer = LibraryImporter::new(&ffmpeg, &self.store).posters_in(self.posters());
        let folders = self.session_folders_on_disk()?;
        for name in chosen.iter().chain(restore) {
            if !folders.iter().any(|f| file_name(&f.to_string_lossy()) == *name) {
                return Err(LibraryError::NotFound(format!("folder {name} in the import folder")));
            }
        }
        let found: Vec<(PathBuf, Vec<_>)> = folders.into_iter()
            .filter(|folder| chosen.iter().chain(restore).any(|name| *name == file_name(&folder.to_string_lossy())))
            .map(|folder| Ok((folder.clone(), importer.discover(&folder)?)))
            .collect::<std::io::Result<_>>()?;
        let count = found.iter().map(|(_, files)| files.len()).sum();
        progress(ImportEvent::Discovered { count, sessions: found.len() });

        let known = self.store.imported_paths()?;
        let deleted = self.store.deleted_paths()?;
        let mut summary = ImportSummary::default();
        for (folder, mut files) in found {
            if !restore.contains(&file_name(&folder.to_string_lossy())) {
                files.retain(|file| {
                    let gone = deleted.contains(file.path.to_string_lossy().as_ref());
                    if gone {
                        summary.skipped += 1;
                        progress(ImportEvent::Skipped(file.path.clone()));
                    }
                    !gone
                });
            }
            if files.iter().all(|f| known.contains(f.path.to_string_lossy().as_ref())) {
                for file in files {
                    summary.skipped += 1;
                    progress(ImportEvent::Skipped(file.path));
                }
                continue;
            }
            let (day, name) = fpv_import::session_of_folder(&file_name(&folder.to_string_lossy()));
            let date = day.map(|d| local_midnight(&d.format("%Y-%m-%d").to_string())).transpose()?;
            let local_day = |date: Option<DateTime<Utc>>| date.map(|d| d.with_timezone(&Local).date_naive());
            let session = match self.store.sessions()?.into_iter()
                .find(|s| s.name == name && local_day(s.date) == day) {
                Some(session) => session,
                None => {
                    let session = Session::new(name, date);
                    self.store.save_session(&session)?;
                    session
                }
            };
            let report = importer.register_in_place(files, &session, &mut progress)?;
            let back: Vec<String> = report.imported.iter().map(|file| file.imported_path.clone())
                .filter(|path| deleted.contains(path)).collect();
            self.store.forget_deleted(&back)?;
            summary.imported += report.imported.len();
            summary.skipped += report.skipped.len();
            summary.failed += report.failed.len();
            summary.flights += report.flights.len();
            if report.imported.is_empty() && self.store.source_files(session.id)?.is_empty() {
                // Every new-looking file failed: no empty session left behind.
                self.store.delete_session(session.id)?;
            }
        }
        Ok(summary)
    }

    /// The session's DVR recordings a reduction would cut, and how. Skipped:
    /// action-cam takes, recordings already reduced, recordings never
    /// segmented (counted apart), and recordings whose flights fill them.
    fn reduce_plans(&self, session: Uuid) -> Result<(Vec<ReducePlan>, usize)> {
        let reduced = self.store.reduced_files(session)?;
        let clips: Vec<Clip> = self.store.flights(session)?.into_iter().flat_map(|(_, clips)| clips).collect();
        let mut plans = vec![];
        let mut unsegmented = 0;
        for file in self.store.source_files(session)? {
            if !file.media.needs_segmentation() || reduced.contains(&file.id) {
                continue;
            }
            if self.store.detection_runs(file.id)?.is_empty() {
                unsegmented += 1;
                continue;
            }
            let rate = file.media.frame_rate;
            let frames = views::last_frame(&file) + 1;
            let own: Vec<Clip> = clips.iter().filter(|c| c.source_file_id == file.id).cloned().collect();
            let spans: Vec<(i64, i64)> = own.iter().map(|c| (c.start_frame, c.end_frame)).collect();
            let ranges = kept_ranges(&spans, rate.frame_index(MARGIN_SECONDS), frames);
            if ranges == [(0, frames)] {
                continue; // flights wall to wall: nothing to gain
            }
            let tags = self.store.tags_of_source_file(file.id)?;
            plans.push(ReducePlan { file, ranges, frames, clips: own, tags });
        }
        Ok((plans, unsegmented))
    }

    /// What "Reduce…" would do to a session, for its confirmation.
    pub fn reduce_preview(&self, session: &str) -> Result<ReducePreview> {
        let session = self.session(session)?;
        let (plans, unsegmented) = self.reduce_plans(session.id)?;
        let mut preview = ReducePreview { unsegmented, ..Default::default() };
        for plan in &plans {
            let size = std::fs::metadata(&plan.file.imported_path).map(|m| m.len()).unwrap_or(0);
            let kept: i64 = plan.ranges.iter().map(|(a, b)| b - a).sum();
            preview.recordings += 1;
            preview.emptied += usize::from(plan.ranges.is_empty());
            preview.bytes_now += size;
            preview.bytes_after += (size as f64 * kept as f64 / plan.frames.max(1) as f64) as u64;
            preview.figures_lost += plan.tags.iter()
                .filter(|t| !plan.ranges.iter().any(|&r| into_piece(t.start_frame, t.end_frame, r).is_some()))
                .count();
        }
        Ok(preview)
    }

    /// "Reduce…": cuts each segmented DVR recording of the session down to its
    /// flights, each widened by `MARGIN_SECONDS`, losslessly (MJPEG is
    /// all-intra), and moves the recording to the trash. A recording is done
    /// whole or not at all: its pieces are cut beside it under hidden names
    /// and checked frame for frame, the library swaps to them in one
    /// transaction, and only then does the recording go. Its flights keep
    /// their titles, likes and tags; figure tags move with their frames, the
    /// ones in the snow go with it. A reduced piece is never segmented again.
    pub fn reduce_session(&self, session: &str, mut progress: impl FnMut(usize, usize, String))
        -> Result<ReduceSummary> {
        let session = self.session(session)?;
        let (plans, _) = self.reduce_plans(session.id)?;
        let mut summary = ReduceSummary::default();
        let total = plans.len();
        for (index, plan) in plans.into_iter().enumerate() {
            let name = file_name(&plan.file.imported_path);
            progress(index, total, format!("Reducing {name} ({}/{total})…", index + 1));
            match self.reduce_recording(&plan) {
                Ok((pieces, freed)) => {
                    summary.recordings += 1;
                    summary.pieces += pieces;
                    summary.bytes_freed += freed;
                }
                Err(error) => summary.failed.push((name, error.to_string())),
            }
        }
        progress(total, total, format!("{} recording(s) reduced", summary.recordings));
        Ok(summary)
    }

    fn reduce_recording(&self, plan: &ReducePlan) -> Result<(usize, u64)> {
        let original = Path::new(&plan.file.imported_path);
        let folder = original.parent().unwrap_or(Path::new("."));
        let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = original.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or("avi".into());
        let ffmpeg = self.ffmpeg();
        let exporter = ClipExporter::new(&ffmpeg);
        let probe = MediaProbe::new(&ffmpeg);
        let mut cut: Vec<(PathBuf, PathBuf, SourceFile)> = vec![];
        let clean = |cut: &[(PathBuf, PathBuf, SourceFile)]| for (partial, _, _) in cut {
            let _ = std::fs::remove_file(partial);
        };
        let reduction_source = SourceFile { muted: false, ..plan.file.clone() };
        for (n, &(start, end)) in plan.ranges.iter().enumerate() {
            let destination = free_name(folder, &format!("{stem}-{}", n + 1), &extension);
            let partial = folder.join(format!(".{}.reducing.{extension}",
                                              destination.file_name().unwrap().to_string_lossy()));
            let range = Clip::new(Uuid::nil(), plan.file.id, start, end);
            let checked = exporter.export_clip(&range, &reduction_source, &partial)
                .map_err(|e| LibraryError::Invalid(e.to_string()))
                .and_then(|_| probe.probe(&partial).map_err(|e| LibraryError::Invalid(e.to_string())))
                .and_then(|media| {
                    // A piece must be exactly its frames, at exactly the
                    // recording's rate, or every clip and tag moved onto it
                    // would sit off its footage.
                    if media.frame_count != Some(end - start) || media.frame_rate != plan.file.media.frame_rate {
                        Err(LibraryError::Invalid(format!(
                            "the piece of frames {start}-{end} came out {:?} frames at {}",
                            media.frame_count, media.frame_rate)))
                    } else {
                        Ok(media)
                    }
                });
            match checked {
                Ok(media) => {
                    let mut piece = SourceFile::new(plan.file.session_id, plan.file.original_path.clone(),
                                                    destination.to_string_lossy(), plan.file.sequence_number,
                                                    MediaInfo { camera: None, ..media });
                    piece.muted = plan.file.muted;
                    // The checksum of the recording it was cut from, on its
                    // first piece: importing the same card again must not
                    // bring the snow back as a new recording.
                    if n == 0 {
                        piece.checksum = plan.file.checksum.clone();
                    }
                    cut.push((partial, destination, piece));
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&partial);
                    clean(&cut);
                    return Err(error);
                }
            }
        }
        // Every piece is whole: they take their names, then the library swaps.
        let mut placed: Vec<PathBuf> = vec![];
        for (partial, destination, _) in &cut {
            if let Err(error) = std::fs::rename(partial, destination) {
                for path in &placed {
                    let _ = std::fs::remove_file(path);
                }
                clean(&cut);
                return Err(error.into());
            }
            placed.push(destination.clone());
        }
        let piece_of = |start: i64, end: i64| plan.ranges.iter().enumerate()
            .find_map(|(i, &range)| into_piece(start, end, range).map(|(a, b)| (i, a, b)));
        let clips: Vec<Clip> = plan.clips.iter().filter_map(|clip| {
            let (i, start, end) = piece_of(clip.start_frame, clip.end_frame)?;
            Some(Clip { source_file_id: cut[i].2.id, start_frame: start, end_frame: end, ..clip.clone() })
        }).collect();
        let tags: Vec<FigureTag> = plan.tags.iter().filter_map(|tag| {
            let (i, start, end) = piece_of(tag.start_frame, tag.end_frame)?;
            Some(FigureTag { source_file_id: cut[i].2.id, start_frame: start, end_frame: end, ..tag.clone() })
        }).collect();
        let runs: Vec<DetectionRun> = cut.iter().zip(&plan.ranges).map(|((_, _, piece), &(start, end))| {
            DetectionRun::new(piece.id, REDUCED_ALGORITHM, std::collections::BTreeMap::from([
                ("marginSeconds".to_owned(), MARGIN_SECONDS),
                ("fromFrame".to_owned(), start as f64),
                ("toFrame".to_owned(), end as f64),
            ]))
        }).collect();
        let pieces: Vec<SourceFile> = cut.iter().map(|(_, _, piece)| piece.clone()).collect();
        if let Err(error) = self.store.apply_reduction(plan.file.id, &pieces, &clips, &tags, &runs) {
            for path in &placed {
                let _ = std::fs::remove_file(path);
            }
            return Err(error.into());
        }
        let before = std::fs::metadata(original).map(|m| m.len()).unwrap_or(0);
        let after: u64 = placed.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        if let Err(error) = trash::delete(original) {
            // The library already holds the pieces; the recording only takes room.
            log::warn!("could not move {} to the trash: {error}", original.display());
            return Ok((pieces.len(), 0));
        }
        Ok((pieces.len(), before.saturating_sub(after)))
    }

    /// Runs the detector over a session's DVR recordings and files the flights.
    /// Re-running is cheap and reversible by design (CLAUDE.md 3.2).
    pub fn segment(&self, session: &str, progress: impl FnMut(SegmentationEvent))
        -> Result<SegmentationReport> {
        let session = self.session(session)?;
        // The session's own thresholds; the run records them, so a clip set
        // stays explainable whichever profile produced it.
        let settings = DetectorSettings::for_profile(session.detection_profile);
        let detector = FileDetector::new(settings.clone(), FrameReader::new(self.tools.clone()));
        let ffmpeg = self.ffmpeg();
        let report = SegmentationService::new(&detector, &ffmpeg, &self.store, settings)
            .segment(session.id, &self.posters(), progress)?;
        self.sweep_orphaned_thumbnails()?;
        Ok(report)
    }

    // Editing

    /// Renames a session. Its media folder keeps the import-time name: every
    /// stored path stays valid, and the folder name was only a convenience.
    pub fn rename_session(&self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(LibraryError::Invalid("a session needs a name".into()));
        }
        let session = self.session(id)?;
        Ok(self.store.save_session(&Session { name: name.into(), ..session })?)
    }

    /// Removes a session and moves its imported recordings to the trash —
    /// recoverable, unlike the rows. The card originals were never touched.
    /// Removes the session from the library. Its recordings go to the trash,
    /// or with `keep_recordings` stay where they are — re-importing the card
    /// later takes them back without copying them again.
    pub fn delete_session(&self, id: &str, keep_recordings: bool) -> Result<()> {
        let session = self.session(id)?;
        // The media folders, recovered from the files rather than recomputed
        // from a name the user may have changed since.
        let folders: HashSet<PathBuf> = self.store.source_files(session.id)?.iter()
            .filter_map(|f| Path::new(&f.imported_path).parent().map(Path::to_path_buf))
            .collect();
        self.store.delete_session(session.id)?;
        for folder in folders.iter().filter(|f| !keep_recordings && f.exists()) {
            if let Err(error) = trash::delete(folder) {
                log::warn!("could not move {} to the trash: {error}", folder.display());
            }
        }
        self.sweep_orphaned_thumbnails()
    }

    /// Which detector thresholds the session's DVR footage is segmented with.
    /// Nothing is re-detected here: the next `segment` applies it.
    /// Saves what the Edit Session dialog changed: name, flying day, place,
    /// notes and tags. Text is trimmed, and an emptied place or note is none.
    /// A tag given twice, in any case, is kept once as first written. The
    /// day only dates the session: folders already on disk keep their names.
    pub fn update_session(&self, id: &str, edit: &SessionEdit) -> Result<()> {
        let name = edit.name.trim();
        if name.is_empty() {
            return Err(LibraryError::Invalid("a session needs a name".into()));
        }
        let session = self.session(id)?;
        let date = match edit.date.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(day) => Some(local_midnight(day)?),
            None => None,
        };
        let text = |value: &str| Some(value.trim().to_owned()).filter(|v| !v.is_empty());
        let mut labels: Vec<String> = vec![];
        for label in edit.labels.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
            if !labels.iter().any(|known| known.eq_ignore_ascii_case(label)) {
                labels.push(label.to_owned());
            }
        }
        self.store.save_session(&Session {
            name: name.into(), date, location: text(&edit.location), notes: text(&edit.notes), ..session
        })?;
        Ok(self.store.set_session_labels(session.id, &labels)?)
    }

    pub fn set_detection_profile(&self, id: &str, profile: &str) -> Result<()> {
        let profile = DetectionProfile::from_raw(profile)
            .ok_or_else(|| LibraryError::Invalid(format!("no detection profile {profile}")))?;
        let session = self.session(id)?;
        Ok(self.store.save_session(&Session { detection_profile: profile, ..session })?)
    }

    pub fn playback_mute(&self, id: &str) -> Result<Option<bool>> {
        Ok(self.store.playback_mute(parse(id)?)?)
    }

    /// Playback only; never changes source audio or export policy.
    pub fn set_playback_mute(&self, id: &str, muted: bool) -> Result<()> {
        Ok(self.store.set_playback_mute(parse(id)?, muted)?)
    }

    /// Sets or clears a flight's title; empty falls back to "Flight N".
    pub fn set_flight_title(&self, id: &str, title: &str) -> Result<()> {
        let (flight, _, _) = self.flight(id)?;
        let title = Some(title.trim().to_owned()).filter(|t| !t.is_empty());
        Ok(self.store.update_flight(&Flight { title, ..flight })?)
    }

    /// The heart on a card: `rating` is 1 or nothing for now.
    pub fn toggle_like(&self, id: &str) -> Result<bool> {
        let (flight, _, _) = self.flight(id)?;
        let rating = if flight.rating.is_some() { None } else { Some(1) };
        self.store.update_flight(&Flight { rating, ..flight })?;
        Ok(rating.is_some())
    }

    /// Labels a whole flight — "freestyle", "gates" — trimmed; a label it has
    /// in any case is kept as first written.
    pub fn add_flight_label(&self, id: &str, label: &str) -> Result<()> {
        let label = label.trim();
        if label.is_empty() {
            return Err(LibraryError::Invalid("a tag needs a name".into()));
        }
        let (flight, _, _) = self.flight(id)?;
        Ok(self.store.add_flight_label(flight.id, label)?)
    }

    pub fn remove_flight_label(&self, id: &str, label: &str) -> Result<()> {
        let (flight, _, _) = self.flight(id)?;
        Ok(self.store.remove_flight_label(flight.id, label)?)
    }

    /// Every flight label in the library, most used first.
    pub fn flight_label_names(&self) -> Result<Vec<String>> {
        Ok(self.store.flight_label_names()?)
    }

    /// The session's recordings outside the current import folder, each with
    /// the path it would move to: the folder the pattern names for it today.
    fn relocation_plan(&self, session: &Session) -> Result<Vec<(SourceFile, PathBuf)>> {
        let pattern = FolderPattern::parse(&self.config.folder_pattern)
            .map_err(|e| LibraryError::Invalid(format!("folderPattern in config.json: {e}")))?;
        let mut plan: Vec<(SourceFile, PathBuf)> = vec![];
        for file in self.store.source_files(session.id)? {
            if views::outside(&self.root, &file.imported_path).is_none() {
                continue;
            }
            let path = Path::new(&file.imported_path);
            let source = fpv_import::source_label(&fpv_import::DiscoveredFile::new(path), &file.media);
            let folder = self.root.join(pattern.render(session.date, &session.name, source));
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let extension = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            // Never over a file already there, nor over one planned before.
            let mut destination = free_name(&folder, &stem, &extension);
            let mut n = 2;
            while plan.iter().any(|(_, d)| d == &destination) {
                destination = free_name(&folder, &format!("{stem} ({n})"), &extension);
                n += 1;
            }
            plan.push((file, destination));
        }
        Ok(plan)
    }

    /// What "Move to Import Folder…" would do, for its confirmation.
    pub fn relocation_preview(&self, session: &str) -> Result<RelocationPreview> {
        let session = self.session(session)?;
        let mut preview = RelocationPreview::default();
        for (file, destination) in self.relocation_plan(&session)? {
            preview.recordings += 1;
            preview.bytes += std::fs::metadata(&file.imported_path).map(|m| m.len()).unwrap_or(0);
            let from = Path::new(&file.imported_path).parent().unwrap_or(Path::new("")).to_string_lossy().into_owned();
            let to = destination.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned();
            if !preview.from.contains(&from) { preview.from.push(from); }
            if !preview.to.contains(&to) { preview.to.push(to); }
        }
        Ok(preview)
    }

    /// "Move to Import Folder…": brings a session's recordings left in an
    /// earlier import folder into the current one, each into the folder the
    /// pattern names for it today. A rename where the two share a disk, else
    /// a copy — mtime kept, size checked — then the original removed. The
    /// library follows each file as it lands, and a file it cannot record is
    /// put back; folders left empty behind are removed. Nothing is ever
    /// written over.
    pub fn move_session_into_import_folder(&self, session: &str, mut progress: impl FnMut(usize, usize, String))
        -> Result<RelocationSummary> {
        let session = self.session(session)?;
        let plan = self.relocation_plan(&session)?;
        let total = plan.len();
        let mut summary = RelocationSummary::default();
        let mut left: Vec<PathBuf> = vec![];
        for (index, (file, destination)) in plan.into_iter().enumerate() {
            let source = PathBuf::from(&file.imported_path);
            let name = file_name(&file.imported_path);
            progress(index, total, format!("Moving {name} ({}/{total})…", index + 1));
            match self.move_recording(&file, &source, &destination) {
                Ok(()) => {
                    summary.moved += 1;
                    if let Some(parent) = source.parent() && !left.iter().any(|p| p == parent) {
                        left.push(parent.to_path_buf());
                    }
                }
                Err(error) => summary.failed.push((name, error.to_string())),
            }
        }
        // Emptied folders go, and their parents if that empties them too —
        // never beyond the folder a recording sat in and its own parents.
        for folder in left {
            let mut folder = Some(folder.as_path());
            while let Some(dir) = folder {
                if std::fs::remove_dir(dir).is_err() {
                    break;
                }
                folder = dir.parent();
            }
        }
        progress(total, total, format!("{} recording(s) moved", summary.moved));
        Ok(summary)
    }

    fn move_recording(&self, file: &SourceFile, source: &Path, destination: &Path) -> Result<()> {
        if let Some(folder) = destination.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let renamed = std::fs::rename(source, destination).is_ok();
        if !renamed {
            // Another disk: copied whole, then checked, before the original goes.
            fpv_import::copy_preserving_mtime(source, destination)?;
            let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
            if size(source) != size(destination) {
                let _ = std::fs::remove_file(destination);
                return Err(LibraryError::Invalid("the copy came out a different size".into()));
            }
        }
        if let Err(error) = self.store.set_imported_path(file.id, &destination.to_string_lossy()) {
            if renamed {
                let _ = std::fs::rename(destination, source);
            } else {
                let _ = std::fs::remove_file(destination);
            }
            return Err(error.into());
        }
        if !renamed {
            std::fs::remove_file(source)?;
        }
        Ok(())
    }

    /// What deleting a flight would do to its files (`delete_flight`).
    pub fn flight_deletion(&self, id: &str) -> Result<FlightDeletion> {
        Ok(self.plan_flight_deletion(id)?.0)
    }

    fn plan_flight_deletion(&self, id: &str) -> Result<(FlightDeletion, Uuid, Vec<SourceFile>)> {
        let (flight, clips, _) = self.flight(id)?;
        let reduced = self.store.reduced_files(flight.session_id)?;
        let others: Vec<Clip> = self.store.flights(flight.session_id)?.into_iter()
            .filter(|(f, _)| f.id != flight.id)
            .flat_map(|(_, clips)| clips).collect();
        let mut plan = FlightDeletion::default();
        let mut going = vec![];
        for clip in &clips {
            let Some(file) = self.store.source_file(clip.source_file_id)? else { continue };
            let name = file_name(&file.imported_path);
            // A file is the flight's own when nothing else is cut from it: an
            // action-cam take, or a reduced piece no other flight lies in.
            if file.media.needs_segmentation() && !reduced.contains(&file.id) {
                plan.recordings.push(name);
            } else if others.iter().any(|c| c.source_file_id == file.id) {
                plan.shared.push(name);
            } else if !going.iter().any(|f: &SourceFile| f.id == file.id) {
                plan.trashed.push(name);
                going.push(file);
            }
        }
        Ok((plan, flight.id, going))
    }

    /// Deletes a flight from the library. A whole DVR recording stays — only
    /// the flight's pointer into it goes. A file that held nothing but this
    /// flight goes to the trash with it: the pieces a reduction left, an
    /// action-cam take and its chapters. A reduced piece shared with another
    /// flight stays for that one. The library changes in one transaction;
    /// the files follow.
    pub fn delete_flight(&self, id: &str) -> Result<FlightDeletion> {
        let (plan, flight, going) = self.plan_flight_deletion(id)?;
        self.store.delete_flight(flight, &going.iter().map(|f| f.id).collect::<Vec<_>>())?;
        for file in &going {
            if let Err(error) = trash::delete(&file.imported_path) {
                log::warn!("could not move {} to the trash: {error}", file.imported_path);
            }
        }
        Ok(plan)
    }

    /// Persists a hand-corrected boundary, clamped to the recording.
    pub fn update_clip(&self, id: &str, start_frame: i64, end_frame: i64) -> Result<()> {
        let clip = self.store.clip(parse(id)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("clip {id}")))?;
        let file = self.store.source_file(clip.source_file_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording of clip {id}")))?;
        let last = views::last_frame(&file).max(0);
        let start = start_frame.clamp(0, last);
        let end = end_frame.clamp(start + 1, last.saturating_add(1));
        Ok(self.store.update_clip(&Clip { start_frame: start, end_frame: end, ..clip })?)
    }

    // Pictures

    /// One frame of a flight's hover filmstrip, cached on disk under the
    /// flight's cache key, so scrubbing a card costs one ffmpeg run per frame
    /// *ever* — not per hover.
    pub fn filmstrip_frame(&self, flight: &str, index: usize, count: usize) -> Result<Thumbnail> {
        let (flight, clips, timeline) = self.flight(flight)?;
        let cached = self.strip_path(&flight, &clips, index);
        if cached.exists() {
            touch(&cached);
            return Ok(Thumbnail::cached(cached));
        }
        let fraction = (index as f64 + 0.5) / count.max(1) as f64;
        let position = timeline.position(fraction * timeline.duration())
            .ok_or_else(|| LibraryError::NotFound("footage in this flight".into()))?;
        if self.only_in_cloud(position.file) {
            return Err(Self::in_cloud_error(position.file));
        }
        let seconds = position.file.media.frame_rate.seconds(position.source_frame);
        let ffmpeg = self.ffmpeg();
        let generator = ThumbnailGenerator::new(&ffmpeg);
        let source = Path::new(&position.file.imported_path);
        let spacing = timeline.duration() / count.max(1) as f64;
        Ok(Thumbnail::made(if rough_strip(&position.file.media, spacing) {
            generator.generate_near(source, seconds, 320, &cached)?
        } else {
            generator.generate(source, seconds, 320, &cached)?
        }))
    }

    fn strip_path(&self, flight: &Flight, clips: &[Clip], index: usize) -> PathBuf {
        self.cache_directory.join("strips").join(format!("{}-{index}.jpg", Flight::cache_key(flight.id, clips)))
    }

    /// The hover-strip frames not on disk yet, across the whole library, the
    /// `first` session's flights first — the one on screen fills before the
    /// rest. Cheap: a file check per frame, no ffmpeg.
    /// Nothing once the pictures leave no room for another frame (weighed as
    /// the frames already made do): the job would make one only to take it
    /// back, at every launch.
    pub fn missing_strip_frames(&self, count: usize, first: Option<&str>) -> Result<Vec<StripFrame>> {
        if self.picture_bytes() + self.strip_frame_size() >= self.config.picture_budget_bytes() {
            return Ok(vec![]);
        }
        let mut sessions = self.store.sessions()?;
        if let Some(first) = first.map(parse).transpose()? {
            sessions.sort_by_key(|s| s.id != first);
        }
        let mut missing = vec![];
        for session in sessions {
            let cloud: HashSet<Uuid> = self.store.source_files(session.id)?.iter()
                .filter(|f| self.only_in_cloud(f)).map(|f| f.id).collect();
            for (flight, clips) in self.store.flights(session.id)? {
                // Its strip waits until the flight is downloaded.
                if clips.iter().any(|c| cloud.contains(&c.source_file_id)) {
                    continue;
                }
                let label = format!("{} — {}", session.name,
                                    flight.title.clone().unwrap_or_else(|| format!("Flight {}", flight.order_in_session)));
                for index in 0..count {
                    if !self.strip_path(&flight, &clips, index).exists() {
                        missing.push(StripFrame { flight: uuid_string(flight.id), index, label: label.clone() });
                    }
                }
            }
        }
        Ok(missing)
    }

    /// Makes every missing hover-strip frame ahead of the pointer, so a card
    /// scrubs at once even on a long 4K take. A flight that disappears
    /// meanwhile (re-segmented, deleted) is passed over; one whose recording
    /// cannot be read is reported once and its other frames skipped.
    pub fn prepare_strips(&self, count: usize, first: Option<&str>,
                          progress: impl FnMut(usize, usize, &str)) -> Result<StripsPrepared> {
        self.prepare_strips_up_to(count, first, usize::MAX, progress)
    }

    /// `prepare_strips`, at most `limit` frames of it: the native app runs one
    /// library job at a time, so the strips go in batches the other jobs can
    /// pass between. `more` says frames were left over.
    pub fn prepare_strips_up_to(&self, count: usize, first: Option<&str>, limit: usize,
                                mut progress: impl FnMut(usize, usize, &str)) -> Result<StripsPrepared> {
        let mut missing = self.missing_strip_frames(count, first)?;
        let mut prepared = StripsPrepared { more: missing.len() > limit, ..Default::default() };
        missing.truncate(limit);
        let mut broken: HashSet<String> = HashSet::new();
        let budget = self.config.picture_budget_bytes();
        let mut used = self.picture_bytes();
        // A frame that goes over is taken back: the job stops at the budget,
        // never a frame over it.
        for (done, frame) in missing.iter().enumerate() {
            progress(done, missing.len(), &frame.label);
            if broken.contains(&frame.flight) {
                continue;
            }
            match self.filmstrip_frame(&frame.flight, frame.index, count) {
                Ok(thumbnail) => {
                    let size = std::fs::metadata(&thumbnail.path).map_or(0, |m| m.len());
                    if thumbnail.made && used + size > budget {
                        let _ = std::fs::remove_file(&thumbnail.path);
                        prepared.at_budget = true;
                        break;
                    }
                    prepared.made += usize::from(thumbnail.made);
                    used += size;
                }
                Err(LibraryError::NotFound(_)) => { broken.insert(frame.flight.clone()); }
                Err(error) => {
                    broken.insert(frame.flight.clone());
                    prepared.failed.push((frame.label.clone(), error.to_string()));
                }
            }
        }
        progress(missing.len(), missing.len(), "");
        Ok(prepared)
    }

    /// Where a recording was copied to, for showing it in the file manager.
    pub fn recording_path(&self, file: &str) -> Result<PathBuf> {
        let file = self.store.source_file(parse(file)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording {file}")))?;
        Ok(PathBuf::from(file.imported_path))
    }

    /// Brings a session's recordings that are only in the cloud back onto this
    /// computer (`download`).
    pub fn download_session(&self, session: &str, progress: impl FnMut(DownloadStep))
        -> Result<DownloadSummary> {
        let session = self.session(session)?;
        self.download(self.store.source_files(session.id)?, DOWNLOAD_CHUNK, progress)
    }

    /// Brings a flight's recordings that are only in the cloud back onto this
    /// computer (`download`).
    pub fn download_flight(&self, flight: &str, progress: impl FnMut(DownloadStep))
        -> Result<DownloadSummary> {
        self.download(self.flight_files(flight)?, DOWNLOAD_CHUNK, progress)
    }

    /// A flight's recordings, each once, in its order.
    fn flight_files(&self, flight: &str) -> Result<Vec<SourceFile>> {
        let (_, clips, _) = self.flight(flight)?;
        let mut files: Vec<SourceFile> = vec![];
        for clip in &clips {
            if let Some(file) = self.store.source_file(clip.source_file_id)?
                && !files.iter().any(|f| f.id == file.id)
            {
                files.push(file);
            }
        }
        Ok(files)
    }

    /// Makes a session's recordings Online Only again (`free_up_space`).
    pub fn free_up_session(&self, session: &str, progress: impl FnMut(usize, usize, String))
        -> Result<FreeUpSummary> {
        let session = self.session(session)?;
        Ok(self.free_up_space(self.store.source_files(session.id)?, progress))
    }

    /// Makes a flight's recordings Online Only again (`free_up_space`).
    pub fn free_up_flight(&self, flight: &str, progress: impl FnMut(usize, usize, String))
        -> Result<FreeUpSummary> {
        Ok(self.free_up_space(self.flight_files(flight)?, progress))
    }

    /// Asks the cloud drive to take each of `files` that is on this disk back
    /// to Online Only. Only what sits in a cloud drive's folder is asked; a
    /// recording the drive keeps here anyway — refused, or not uploaded yet —
    /// is reported with its reason, and is never deleted by other means.
    fn free_up_space(&self, files: Vec<SourceFile>, mut progress: impl FnMut(usize, usize, String))
        -> FreeUpSummary {
        let here: Vec<SourceFile> = files.into_iter().filter(|f| self.freeable(f)).collect();
        let mut summary = FreeUpSummary::default();
        for (index, file) in here.iter().enumerate() {
            let name = file_name(&file.imported_path);
            progress(index, here.len(), format!("Freeing {name} ({}/{})…", index + 1, here.len()));
            let path = Path::new(&file.imported_path);
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            match (self.cloud.free_up_space)(path) {
                Ok(()) if self.only_in_cloud(file) => {
                    summary.freed += 1;
                    summary.bytes += size;
                }
                Ok(()) => summary.failed.push((name, "the cloud drive kept it on this disk".into())),
                Err(error) => summary.failed.push((name, error.to_string())),
            }
        }
        progress(here.len(), here.len(), format!("{} recording(s) made Online Only", summary.freed));
        summary
    }

    /// Reads every recording of `files` that is only in the cloud through to
    /// its end: the File Provider downloads a placeholder on its first read,
    /// so reading is downloading, and it needs nothing of the drive's own.
    /// Progress counts bytes, the placeholder's size being the recording's.
    /// A recording still in the cloud afterwards is reported, not trusted.
    fn download(&self, files: Vec<SourceFile>, chunk: usize, mut progress: impl FnMut(DownloadStep))
        -> Result<DownloadSummary> {
        let away: Vec<SourceFile> = files.into_iter().filter(|f| self.only_in_cloud(f)).collect();
        let size = |file: &SourceFile| std::fs::metadata(&file.imported_path).map_or(0, |m| m.len());
        let total: u64 = away.iter().map(size).sum();
        let mut summary = DownloadSummary::default();
        let mut buffer = vec![0u8; chunk];
        for (index, file) in away.iter().enumerate() {
            let name = file_name(&file.imported_path);
            let line = format!("Downloading {name} ({}/{})…", index + 1, away.len());
            let step = |done: u64| DownloadStep { done: done as usize, total: total as usize,
                                                  file: index + 1, files: away.len(), line: line.clone() };
            progress(step(summary.bytes));
            let read = (|| -> std::io::Result<u64> {
                let mut source = std::fs::File::open(&file.imported_path)?;
                let mut read = 0;
                loop {
                    let n = std::io::Read::read(&mut source, &mut buffer)?;
                    if n == 0 {
                        return Ok(read);
                    }
                    read += n as u64;
                    progress(step(summary.bytes + read));
                }
            })();
            match read {
                Ok(_) if self.only_in_cloud(file) =>
                    summary.failed.push((name, "the cloud drive did not bring it down".into())),
                Ok(read) => {
                    summary.downloaded += 1;
                    summary.bytes += read;
                }
                Err(error) => summary.failed.push((name, error.to_string())),
            }
        }
        progress(DownloadStep { done: total as usize, total: total as usize, file: away.len(), files: away.len(),
                                line: format!("{} recording(s) downloaded", summary.downloaded) });
        Ok(summary)
    }

    /// One recording per folder the session's recordings sit in, to show in
    /// the file manager: a session holding DVR and GoPro footage has a folder
    /// per source (`{source}` in the folder pattern), and each is opened.
    pub fn session_folders(&self, session: &str) -> Result<Vec<PathBuf>> {
        let session = self.session(session)?;
        let mut shown: Vec<PathBuf> = Vec::new();
        for file in self.store.source_files(session.id)? {
            let path = PathBuf::from(file.imported_path);
            if !shown.iter().any(|known| known.parent() == path.parent()) {
                shown.push(path);
            }
        }
        Ok(shown)
    }

    /// A thumbnail across the complete original source, not the flight's trim.
    pub fn source_strip_frame(&self, file: &str, index: usize, count: usize) -> Result<Thumbnail> {
        if count == 0 || index >= count {
            return Err(LibraryError::Invalid("invalid source filmstrip position".into()));
        }
        let source = self.store.source_file(parse(file)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording {file}")))?;
        let path = Path::new(&source.imported_path);
        if fpv_media::only_in_cloud(path) { return Err(Self::in_cloud_error(&source)); }
        let frames = source.media.frame_count.unwrap_or_else(||
            (source.media.duration * source.media.frame_rate.value()).round() as i64).max(1);
        let frame = if count == 1 { 0 } else {
            ((frames - 1) as f64 * index as f64 / (count - 1) as f64).round() as i64
        };
        let cached = self.cache_directory.join("preview").join(format!("{file}-strip-{count}-{index}.jpg"));
        if cached.exists() { return Ok(Thumbnail::cached(cached)); }
        let runner = self.ffmpeg();
        let generator = ThumbnailGenerator::new(&runner);
        let seconds = source.media.frame_rate.seconds(frame);
        let image = generator.generate(path, seconds, 160, &cached)?;
        Ok(Thumbnail::made(image))
    }

    /// The source frame under a boundary, for the editor — decoded from the
    /// recording itself, never the proxy, whose frames are resampled.
    pub fn preview_frame(&self, file: &str, frame: i64) -> Result<Thumbnail> {
        let file = self.store.source_file(parse(file)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording {file}")))?;
        let cached = self.cache_directory.join("preview")
            .join(format!("{}-{frame}.jpg", uuid_string(file.id)));
        if cached.exists() {
            return Ok(Thumbnail::cached(cached));
        }
        let ffmpeg = self.ffmpeg();
        Ok(Thumbnail::made(ThumbnailGenerator::new(&ffmpeg).generate(
            Path::new(&file.imported_path), file.media.frame_rate.seconds(frame), 640, &cached)?))
    }

    /// Deletes thumbnails nothing points at any more: re-segmenting replaces
    /// clips under new IDs, boundary edits re-key the strips, and deleting a
    /// session leaves both behind. Deleting too much costs nothing — a poster
    /// or a strip is remade the next time it is shown. Editor previews are a
    /// session's worth of scratch and go every time.
    fn sweep_orphaned_thumbnails(&self) -> Result<()> {
        let mut posters = HashSet::new();
        let mut strip_keys = HashSet::new();
        for session in self.store.sessions()? {
            for (flight, clips) in self.store.flights(session.id)? {
                posters.extend(clips.iter().filter_map(|c| c.thumbnail_path.as_deref().map(file_name)));
                strip_keys.insert(Flight::cache_key(flight.id, &clips));
            }
        }
        let sweep = |directory: PathBuf, keep: &dyn Fn(&str) -> bool| {
            let Ok(entries) = std::fs::read_dir(directory) else { return };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".jpg") && !keep(&name) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        };
        sweep(self.posters(), &|name| posters.contains(name));
        // A strip frame is "<cache key>-<index>.jpg".
        sweep(self.cache_directory.join("strips"), &|name| {
            name.rsplit_once('-').is_none_or(|(key, _)| strip_keys.contains(key))
        });
        let _ = std::fs::remove_dir_all(self.cache_directory.join("preview"));
        Ok(())
    }

    // Export

    /// Metadata and source-relative bounds for every part, in playback order.
    pub fn flight_info(&self, flight: &str) -> Result<Vec<RecordingInfo>> {
        let (_, _, timeline) = self.flight(flight)?;
        Ok(timeline.parts.into_iter().map(|(clip, source)| {
            let path = Path::new(&source.imported_path);
            let metadata = std::fs::metadata(path).ok().filter(|m| m.is_file());
            let cloud = fpv_media::only_in_cloud(path);
            RecordingInfo { clip, available: metadata.is_some() && !cloud,
                cloud, size_bytes: metadata.map(|m| m.len()), source }
        }).collect())
    }

    /// Writes a flight out. Returns the files written.
    pub fn export(&self, flight: &str, destination: &Path, mode: ExportMode) -> Result<Vec<PathBuf>> {
        let (_, _, timeline) = self.flight(flight)?;
        // Text-entry destinations in the native UI have no OS overwrite
        // prompt. Check every part before writing any of them.
        let destinations = if mode == ExportMode::Parts {
            let stem = destination.file_stem().unwrap_or_default().to_string_lossy();
            let extension = destination.extension().and_then(|s| s.to_str()).unwrap_or("avi");
            (1..=timeline.parts.len()).map(|index|
                destination.with_file_name(format!("{stem} - part{index}.{extension}"))).collect()
        } else { vec![destination.to_path_buf()] };
        if let Some(path) = destinations.iter().find(|path| path.exists()) {
            return Err(LibraryError::Invalid(format!("destination already exists: {}", path.display())));
        }
        let ffmpeg = self.ffmpeg();
        let exporter = ClipExporter::new(&ffmpeg);
        Ok(match mode {
            ExportMode::Lossless => vec![exporter.export_flight_with_mute(
                &timeline.parts,
                destination,
                self.config.mute_videos,
            )?],
            ExportMode::Reencode => vec![exporter.export_flight_reencoded_with_mute(
                &timeline.parts,
                destination,
                None,
                self.config.mute_videos,
            )?],
            ExportMode::Parts => {
                let stem = destination.file_stem().map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "flight".into());
                let extension = destination.extension().and_then(|e| e.to_str()).unwrap_or("avi");
                timeline.parts.iter().enumerate()
                    .map(|(index, (clip, file))| {
                        let path = destination.with_file_name(
                            format!("{stem} - part{}.{extension}", index + 1));
                        Ok(exporter.export_clip_with_mute(
                            clip,
                            file,
                            &path,
                            self.config.mute_videos,
                        )?)
                    })
                    .collect::<Result<_>>()?
            }
        })
    }

    /// Exports several flights into `folder`, one after the other, each under
    /// "<session> — <title>" (`.avi` for DVR footage, `.mp4` for action cams),
    /// never overwriting: a name already taken gets " (2)", " (3)"…
    ///
    /// A flight whose recordings disagree on their rate cannot be asked about
    /// one by one in a batch, so `on_mismatch` (`Parts` or `Reencode`) answers
    /// for all of them up front — `Reencode` for DVR footage only; an action
    /// cam's take always gets its lossless parts. A flight that fails is reported and the batch
    /// goes on; `progress` gets (flights done, total, the flight starting).
    pub fn export_flights(&self, flights: &[String], folder: &Path, on_mismatch: ExportMode,
                          mut progress: impl FnMut(usize, usize, &str)) -> Result<BulkExport> {
        std::fs::create_dir_all(folder)?;
        let sessions = self.store.sessions()?;
        let mut outcome = BulkExport::default();
        for (index, id) in flights.iter().enumerate() {
            let label = match self.flight(id) {
                Ok((flight, _, timeline)) => {
                    let session = sessions.iter().find(|s| s.id == flight.session_id)
                        .map(|s| s.name.as_str()).unwrap_or("Session");
                    let title = flight.title.clone()
                        .unwrap_or_else(|| format!("Flight {}", flight.order_in_session));
                    let dvr = timeline.parts.iter().any(|(_, f)| f.media.needs_segmentation());
                    let intra = timeline.parts.iter().all(|(_, f)| f.media.is_all_intra());
                    let (name, extension) = export_name(session, &title, dvr);
                    Some((name, extension, intra))
                }
                Err(_) => None,
            };
            let Some((name, extension, intra)) = label else {
                outcome.failed.push((id.clone(), "not in the library".into()));
                continue;
            };
            progress(index, flights.len(), &name);
            let destination = free_name(folder, &name, extension);
            let written = match self.export(id, &destination, ExportMode::Lossless) {
                // Re-encoding resamples MJPEG; an action cam's long-GOP take
                // cannot go that way, so it gets its lossless parts instead.
                Err(LibraryError::Export(ExportError::MismatchedFrameRates(..))) => {
                    let mode = if intra { on_mismatch } else { ExportMode::Parts };
                    self.export(id, &destination, mode)
                }
                other => other,
            };
            match written {
                Ok(files) => outcome.written.extend(files),
                Err(error) => outcome.failed.push((name, error.to_string())),
            }
        }
        progress(flights.len(), flights.len(), "");
        Ok(outcome)
    }

    /// The session's clips as ground-truth rows for the detector's tuning
    /// harness — corrections feed back as labelled data (CLAUDE.md 4.6). Real
    /// seconds: `fpv_spike` reads plain numbers as seconds. DVR files only;
    /// action-cam takes were never detected.
    pub fn export_labels(&self, session: &str, destination: &Path) -> Result<usize> {
        let session = self.session(session)?;
        let mut lines = vec![
            "# Ground truth rows exported from the FPV Rush library.".to_owned(),
            "# One row per clip: filename,start,end — times in real seconds.".into(),
            "#".into(),
            "# reviewed: no   <- flip to yes only after checking every boundary yourself".into(),
        ];
        let mut by_file: HashMap<Uuid, Vec<Clip>> = HashMap::new();
        for (_, clips) in self.store.flights(session.id)? {
            for clip in clips {
                by_file.entry(clip.source_file_id).or_default().push(clip);
            }
        }
        let mut rows = 0;
        for file in self.store.source_files(session.id)?.iter().filter(|f| f.media.needs_segmentation()) {
            let rate = file.media.frame_rate;
            let mut clips = by_file.remove(&file.id).unwrap_or_default();
            clips.sort_by_key(|c| c.start_frame);
            for clip in clips {
                lines.push(format!("{},{:.3},{:.3}", file_name(&file.imported_path),
                                   rate.seconds(clip.start_frame), rate.seconds(clip.end_frame)));
                rows += 1;
            }
        }
        std::fs::write(destination, lines.join("\n") + "\n")?;
        Ok(rows)
    }

    // Playback

    /// Full source timeline for non-destructive boundary previews, including
    /// frames outside the currently selected clip.
    pub fn native_source_playback(&self, file: &str) -> Result<NativeSegment> {
        let file = self.store.source_file(parse(file)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording {file}")))?;
        Ok(NativeSegment {
            segment: Segment { path: PathBuf::from(&file.imported_path), start: 0.0,
                end: file.media.duration, offset: 0.0 },
            width: file.media.width, height: file.media.height,
            has_audio: file.media.audio_codec.is_some() && !file.muted,
            all_intra: file.media.is_all_intra(),
        })
    }

    /// Native playback reads every codec directly, including DVR MJPEG. Each
    /// part uses its own source clock, even when adjacent file rates disagree.
    pub fn native_playback(&self, flight: &str) -> Result<Vec<NativeSegment>> {
        let (_, _, timeline) = self.flight(flight)?;
        Ok(timeline.parts.iter().map(|(clip, file)| {
            let rate = file.media.frame_rate;
            NativeSegment {
                segment: Segment {
                    path: PathBuf::from(&file.imported_path),
                    start: rate.seconds(clip.start_frame),
                    end: rate.seconds(clip.end_frame),
                    offset: timeline.flight_time(clip.start_frame, clip).unwrap_or_default(),
                },
                width: file.media.width,
                height: file.media.height,
                has_audio: file.media.audio_codec.is_some() && !file.muted,
                all_intra: file.media.is_all_intra(),
            }
        }).collect())
    }

    /// How the player opens a flight. An action-cam take plays from its own
    /// files — H.264 or HEVC in MP4, which the web view decodes itself (in
    /// hardware on a Mac): no transcode to wait for, full quality, exact
    /// frames, and no gigabytes in the cache. A GoPro take in chapters plays
    /// them one after the other. The DVR needs the proxy: no web view reads
    /// MJPEG-in-AVI, and a flight across two recordings at different rates
    /// only plays right once resampled and joined. If the web view refuses a
    /// file after all (HEVC on Linux without the GStreamer plugin), the player
    /// falls back to `proxy`.
    pub fn playback(&self, flight: &str) -> Result<Playback> {
        let (_, _, timeline) = self.flight(flight)?;
        if !plays_directly(timeline.parts.iter().map(|(_, file)| &file.media)) {
            return Ok(Playback::Proxy);
        }
        let segments = timeline.parts.iter().map(|(clip, file)| {
            let rate = file.media.frame_rate;
            Segment {
                path: PathBuf::from(&file.imported_path),
                start: rate.seconds(clip.start_frame),
                end: rate.seconds(clip.end_frame),
                offset: timeline.flight_time(clip.start_frame, clip).unwrap_or_default(),
            }
        }).collect();
        Ok(Playback::Direct { segments })
    }

    /// The flight's playback proxy: a cache hit, or a fresh transcode
    /// registered into the cache and evicted against the budget — never the
    /// key just written, which the player is about to open.
    pub fn proxy(&self, flight: &str) -> Result<PathBuf> {
        let (flight, clips, timeline) = self.flight(flight)?;
        let cache = self.proxy_cache();
        let key = Flight::cache_key(flight.id, &clips);
        if let Some(path) = cache.cached(&key)? {
            return Ok(path);
        }
        let incoming = cache.incoming_path(&key);
        let ffmpeg = self.ffmpeg();
        let generated = ProxyGenerator::new(&ffmpeg).generate(&timeline.parts, &incoming);
        if let Err(error) = generated {
            let _ = std::fs::remove_file(&incoming);
            return Err(error.into());
        }
        let path = cache.register(&key, &incoming)?;
        if let Err(error) = cache.evict(&HashSet::from([key])) {
            log::warn!("could not evict proxies: {error}");
        }
        Ok(path)
    }

    // Figure tags

    /// The tags in a flight, placed on its clock and sorted. A tag outside the
    /// flight — footage a re-segmentation no longer keeps — is not shown here,
    /// and not deleted either (CLAUDE.md 3.3).
    pub fn flight_tags(&self, flight: &str) -> Result<Vec<TagView>> {
        let (flight, _, timeline) = self.flight(flight)?;
        let mut tags: Vec<TagView> = self.store.tags_in_session(flight.session_id)?.iter()
            .filter_map(|tag| views::tag_view(tag, &timeline))
            .collect();
        tags.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(tags)
    }

    /// Tags a range marked on the player, in flight seconds. A range across the
    /// DVR's cut, or a GoPro chapter's, is clipped to the recording holding its
    /// start: a tag lives in one recording.
    pub fn add_tag(&self, flight: &str, label: &str, start: f64, end: f64) -> Result<TagView> {
        let label = label.trim();
        if label.is_empty() {
            return Err(LibraryError::Invalid("a figure needs a name".into()));
        }
        let (_, _, timeline) = self.flight(flight)?;
        let range = timeline.source_range(start, end)
            .ok_or_else(|| LibraryError::NotFound("footage in this flight".into()))?;
        let tag = FigureTag::new(range.file.id, range.start_frame, range.end_frame, label);
        self.store.save_tag(&tag)?;
        views::tag_view(&tag, &timeline)
            .ok_or_else(|| LibraryError::NotFound("the tag in its own flight".into()))
    }

    pub fn delete_tag(&self, id: &str) -> Result<()> {
        Ok(self.store.delete_tag(parse(id)?)?)
    }

    /// The autocomplete vocabulary: every label already used, most used first.
    pub fn tag_labels(&self) -> Result<Vec<String>> {
        Ok(self.store.tag_labels()?)
    }

    // Settings

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Names the folders of future imports from `pattern`, and saves it to
    /// `config.json`. Folders already on disk keep their names: every stored
    /// path is absolute.
    pub fn set_folder_pattern(&mut self, pattern: &str) -> Result<()> {
        let pattern = FolderPattern::parse(pattern).map_err(LibraryError::Invalid)?;
        let config = Config { folder_pattern: pattern.as_str().into(), ..self.config.clone() };
        config.save(&self.config_directory)?;
        self.config = config;
        Ok(())
    }

    /// What a folder would be called under `pattern`, for the settings to show
    /// while it is typed — on a sample dated session, for a DVR source.
    pub fn folder_preview(pattern: &str, session: &str) -> Result<String> {
        let pattern = FolderPattern::parse(pattern).map_err(LibraryError::Invalid)?;
        let day = NaiveDate::from_ymd_opt(2026, 10, 10).and_then(|d| d.and_hms_opt(12, 0, 0))
            .and_then(|d| Local.from_local_datetime(&d).earliest()).map(|d| d.to_utc());
        Ok(pattern.render(day, session, "FATSHARK"))
    }

    /// Points future imports at another folder, and saves it to `config.json`.
    ///
    /// Recordings already imported stay where they are: every stored path is
    /// absolute, so nothing has to move and nothing breaks. Posters and the
    /// other caches live in the cache folder and are not concerned.
    pub fn set_media_root(&mut self, root: &Path) -> Result<()> {
        if !root.is_absolute() {
            return Err(LibraryError::Invalid(format!("not an absolute folder: {}", root.display())));
        }
        std::fs::create_dir_all(root)?;
        // Writable, or every import would fail one file at a time.
        let probe = root.join(".fpv-rush-write-test");
        std::fs::write(&probe, b"")
            .and_then(|_| std::fs::remove_file(&probe))
            .map_err(|error| LibraryError::Invalid(format!("cannot write in {}: {error}", root.display())))?;
        if root == self.root {
            return Ok(());
        }
        let config = Config { media_root: root.to_path_buf(), ..self.config.clone() };
        config.save(&self.config_directory)?;
        self.config = config;
        self.root = root.to_path_buf();
        Ok(())
    }

    /// Every folder the UI loads pictures from: the cache, plus wherever a
    /// poster sits that could not be moved into it.
    pub fn image_directories(&self) -> Result<Vec<PathBuf>> {
        let mut directories = vec![self.cache_directory.clone()];
        for session in self.store.sessions()? {
            for (_, clips) in self.store.flights(session.id)? {
                for poster in clips.iter().filter_map(|c| c.thumbnail_path.as_deref()) {
                    if let Some(parent) = Path::new(poster).parent()
                        && !parent.starts_with(&self.cache_directory)
                        && !directories.iter().any(|d| d == parent)
                    {
                        directories.push(parent.to_path_buf());
                    }
                }
            }
        }
        Ok(directories)
    }

    /// A clip's poster, remade if the cache lost it. Never its first frame:
    /// analog reception fades in through sync bands (`poster_offset`).
    pub fn poster(&self, clip: &str) -> Result<Thumbnail> {
        let clip = self.store.clip(parse(clip)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("clip {clip}")))?;
        if let Some(path) = clip.thumbnail_path.as_deref().map(PathBuf::from)
            && path.is_file()
        {
            touch(&path);
            return Ok(Thumbnail::cached(path));
        }
        let file = self.store.source_file(clip.source_file_id)?
            .ok_or_else(|| LibraryError::NotFound(format!("recording of clip {}", clip.id)))?;
        if self.only_in_cloud(&file) {
            return Err(Self::in_cloud_error(&file));
        }
        let ffmpeg = self.ffmpeg();
        let path = ThumbnailGenerator::new(&ffmpeg).generate_for_clip(&clip, &file, &self.posters(), 480)?;
        self.store.update_clip(&Clip { thumbnail_path: Some(path.to_string_lossy().into_owned()), ..clip })?;
        Ok(Thumbnail::made(path))
    }

    /// Rebuilds a poster after the UI found that the cached image cannot be decoded.
    pub fn remake_poster(&self, clip_id: &str) -> Result<Thumbnail> {
        let clip = self.store.clip(parse(clip_id)?)?
            .ok_or_else(|| LibraryError::NotFound(format!("clip {clip_id}")))?;
        if let Some(path) = clip.thumbnail_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
        self.store.update_clip(&Clip {
            thumbnail_path: None,
            ..clip
        })?;
        self.poster(clip_id)
    }
}

/// A picture from the cache folder, and whether this call had to make it —
/// what the task bar records: a hover over strips already on disk is not work.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Thumbnail {
    pub path: PathBuf,
    pub made: bool,
}

impl Thumbnail {
    fn cached(path: PathBuf) -> Self { Self { path, made: false } }
    fn made(path: PathBuf) -> Self { Self { path, made: true } }
}

/// `<folder>/<stem>.<extension>`, or with " (2)", " (3)"… until it names
/// nothing on disk — a part file (" - part1") counts as taken too.
fn free_name(folder: &Path, stem: &str, extension: &str) -> PathBuf {
    let taken = |stem: &str| folder.join(format!("{stem}.{extension}")).exists()
        || folder.join(format!("{stem} - part1.{extension}")).exists();
    let mut candidate = stem.to_owned();
    let mut n = 2;
    while taken(&candidate) {
        candidate = format!("{stem} ({n})");
        n += 1;
    }
    folder.join(format!("{candidate}.{extension}"))
}

fn parse(id: &str) -> Result<Uuid> {
    Uuid::parse_str(id).map_err(|_| LibraryError::Invalid(format!("not an id: {id}")))
}

fn local_midnight(text: &str) -> Result<DateTime<Utc>> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
        .and_then(|day| Local.from_local_datetime(&day.and_hms_opt(0, 0, 0)?).earliest())
        .map(|local| local.to_utc())
        .ok_or_else(|| LibraryError::Invalid(format!("not a YYYY-MM-DD date: {text}")))
}

#[cfg(test)]
mod tests;
