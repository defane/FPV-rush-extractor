//! Turns a session's recordings into the flights the user browses. Port of the
//! Swift `FPVPipeline` module.
//!
//! This is the join the rest of the library is built around: the detector speaks
//! in seconds within one file, the library in flights that may span two of them.

use std::collections::HashSet;
use std::path::Path;

use fpv_core::{Clip, ClipEntry, ClipKind, DetectionRun, Flight, SourceFile};
use fpv_detect::{ClipDetecting, ClipStitcher, DetectedClip, DetectorSettings, FileClips};
use fpv_media::{FfmpegInvoking, ThumbnailGenerator};
use fpv_store::{LibraryStore, StoreError};
use uuid::Uuid;

/// Records universal codec eligibility and clean EOF frame preservation.
/// Spatial classifier thresholds remain those of the original detector.
pub const ALGORITHM_VERSION: &str = "signal-detect-2";

#[derive(Debug, Default)]
pub struct SegmentationReport {
    pub flights: usize,
    pub clips: usize,
    pub stitched: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
    /// Reduced recordings left alone to preserve their edited clips.
    pub whole_file: usize,
}

#[derive(Debug, Clone)]
pub enum SegmentationEvent {
    Detecting(SourceFile),
    Detected(SourceFile, usize),
    Stitched(String, String),
    Failed(SourceFile, String),
}

pub struct SegmentationService<'a> {
    detector: &'a dyn ClipDetecting,
    ffmpeg: &'a dyn FfmpegInvoking,
    store: &'a LibraryStore,
    settings: DetectorSettings,
}

impl<'a> SegmentationService<'a> {
    /// `ffmpeg` draws the poster frames; the detector decodes on its own.
    pub fn new(detector: &'a dyn ClipDetecting, ffmpeg: &'a dyn FfmpegInvoking,
               store: &'a LibraryStore, settings: DetectorSettings) -> Self {
        Self { detector, ffmpeg, store, settings }
    }

    pub fn segment(&self, session: Uuid, thumbnail_directory: &Path,
                   progress: impl FnMut(SegmentationEvent))
        -> Result<SegmentationReport, StoreError> {
        self.segment_files(session, thumbnail_directory, None, progress)
    }

    /// Import follow-up: analyze only the newly imported sources, preserving
    /// previously reviewed flights and their numbering.
    pub fn segment_imported(&self, session: Uuid, files: &[Uuid], thumbnail_directory: &Path,
                            progress: impl FnMut(SegmentationEvent))
        -> Result<SegmentationReport, StoreError> {
        self.segment_files(session, thumbnail_directory, Some(files), progress)
    }

    fn segment_files(&self, session: Uuid, thumbnail_directory: &Path, only: Option<&[Uuid]>,
                     mut progress: impl FnMut(SegmentationEvent))
        -> Result<SegmentationReport, StoreError> {
        let mut report = SegmentationReport::default();
        // Codec never determines whether a recording can contain snow. Reduced
        // pieces keep their saved edits rather than being silently re-segmented.
        let reduced = self.store.reduced_files(session)?;
        let (mut files, whole): (Vec<SourceFile>, Vec<SourceFile>) =
            self.store.source_files(session)?.into_iter()
                .partition(|file| file.media.needs_segmentation() && !reduced.contains(&file.id)
                    && only.is_none_or(|ids| ids.contains(&file.id)));
        report.whole_file = whole.len();
        // Card order, because a flight continuing into the next file only makes
        // sense along it.
        files.sort_by_key(|file| file.sequence_number.unwrap_or(0));

        let mut detected: Vec<(SourceFile, Vec<DetectedClip>)> = vec![];
        for file in files {
            progress(SegmentationEvent::Detecting(file.clone()));
            match self.detector.clips(Path::new(&file.imported_path)) {
                Ok(clips) => {
                    progress(SegmentationEvent::Detected(file.clone(), clips.len()));
                    detected.push((file, clips));
                }
                Err(error) => {
                    report.failed.push((file.imported_path.clone(), error.to_string()));
                    progress(SegmentationEvent::Failed(file, error.to_string()));
                }
            }
        }

        let links: HashSet<String> = ClipStitcher::links(&detected.iter()
            .map(|(file, clips)| FileClips {
                name: file.imported_path.clone(),
                sequence_number: file.sequence_number,
                clips: clips.clone(),
            })
            .collect::<Vec<_>>())
            .into_iter().map(|(first, _)| first).collect();

        let thumbnails = ThumbnailGenerator::new(self.ffmpeg);
        let mut made: HashSet<Uuid> = HashSet::new();
        let mut order = if only.is_some() {
            self.store.flights(session)?.iter()
                .filter(|(_, clips)| clips.iter().any(|c| !detected.iter().any(|(file, _)| file.id == c.source_file_id)))
                .map(|(f, _)| f.order_in_session).max().unwrap_or(0)
        } else { 0 };
        let mut carried: Option<Flight> = None;
        for (index, (file, detected_clips)) in detected.iter().enumerate() {
            let mut flights: Vec<Flight> = vec![];
            let mut clips: Vec<Clip> = vec![];
            for (position, detected_clip) in detected_clips.iter().enumerate() {
                // The first clip of this file continues the previous file's last
                // one when the DVR cut the recording between them.
                let continued = if position == 0 { carried.take() } else { None };
                let continues = continued.is_some();
                let flight = continued.unwrap_or_else(|| {
                    order += 1;
                    let flight = Flight::new(session, order);
                    flights.push(flight.clone());
                    flight
                });
                made.insert(flight.id);
                clips.push(clip(detected_clip, file, flight.id, continues));
                carried = None;
            }
            // Does this file hand a flight on to the next one?
            if links.contains(&file.imported_path)
                && let Some(last) = clips.last()
            {
                carried = Some(flights.iter().find(|f| f.id == last.flight_id).cloned()
                    .unwrap_or(Flight { id: last.flight_id, ..Flight::new(session, order) }));
                let next = detected[index + 1].0.imported_path.clone();
                report.stitched.push((file.imported_path.clone(), next.clone()));
                progress(SegmentationEvent::Stitched(file.imported_path.clone(), next));
            }

            for clip in &mut clips {
                // A poster is a convenience: its failure costs a thumbnail.
                clip.thumbnail_path = thumbnails
                    .generate_for_clip(clip, file, thumbnail_directory, 480).ok()
                    .map(|path| path.to_string_lossy().into_owned());
            }
            let run = DetectionRun::new(file.id, ALGORITHM_VERSION, self.settings.parameters());
            self.store.replace_clips(file.id, &clips, &flights, &run)?;
            report.flights += flights.len();
            report.clips += clips.len();
        }

        if only.is_none() { self.renumber_others(session, &made, order)?; }
        Ok(report)
    }

    /// Numbers every flight this run did not make — action-cam takes, and the
    /// flights of a file the detector failed on — after the ones it did, in
    /// their existing order. Detection numbers its flights from 1, so without
    /// this a re-segmentation that finds one more flight collides with the
    /// first action-cam take.
    fn renumber_others(&self, session: Uuid, made: &HashSet<Uuid>, after: i64)
        -> Result<(), StoreError> {
        let mut next = after;
        for (flight, _) in self.store.flights(session)? {
            if made.contains(&flight.id) {
                continue;
            }
            next += 1;
            if flight.order_in_session != next {
                self.store.update_flight(&Flight { order_in_session: next, ..flight })?;
            }
        }
        Ok(())
    }
}

/// Seconds to frame indices, through the file's own rate — never a nominal
/// one, because no two recordings on a card share theirs.
fn clip(detected: &DetectedClip, file: &SourceFile, flight: Uuid, continues: bool) -> Clip {
    let rate = file.media.frame_rate;
    Clip {
        kind: ClipKind::Flight,
        // Continuation wins: the DVR cut this flight, it did not start here —
        // whatever the OSD says in the first seconds.
        entry: if continues { ClipEntry::Continuation } else { detected.entry },
        exit: detected.exit,
        confidence: detected.confidence,
        ..Clip::new(flight, file.id, rate.frame_index(detected.start),
            if detected.at_file_end && detected.exit == fpv_core::ClipExit::Eof {
                file.media.frame_count.unwrap_or_else(|| rate.frame_index(file.media.duration))
            } else { rate.frame_index(detected.end) })
    }
}

#[cfg(test)]
mod tests;
