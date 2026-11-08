//! What the task bar shows of an import or a segmentation: one line per
//! event, and how many files are through out of how many. Kept here rather
//! than in the Tauri commands so the counting is tested — it has been wrong
//! once (a file counted as done when it started).

use std::path::Path;

use fpv_import::ImportEvent;
use fpv_pipeline::SegmentationEvent;

/// One step of a job: the line to show and the count so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub line: String,
    pub done: usize,
    /// 0 until the job knows how much there is.
    pub total: usize,
}

/// Counts an import's files: each one is done once imported, skipped or
/// failed. The total arrives with the first event, `Discovered`.
#[derive(Debug, Default)]
pub struct ImportProgress {
    done: usize,
    total: usize,
    imported: usize,
    skipped: usize,
    failed: usize,
}

impl ImportProgress {
    pub fn step(&mut self, event: ImportEvent) -> Step {
        let line = match event {
            ImportEvent::Discovered { count, sessions } => {
                self.total = count;
                format!("{count} recording(s) → {sessions} session(s)")
            }
            ImportEvent::Copying(path) => format!("Copying {}…", name_of(&path)),
            ImportEvent::Imported(file) => {
                self.done += 1;
                self.imported += 1;
                format!("Imported {} — {}, {:.1} s", name_of(Path::new(&file.imported_path)),
                        file.media.frame_rate, file.media.duration)
            }
            ImportEvent::Skipped(path) => {
                self.done += 1;
                self.skipped += 1;
                format!("Skipped {} (already in library)", name_of(&path))
            }
            ImportEvent::Failed(path, reason) => {
                self.done += 1;
                self.failed += 1;
                format!("FAILED {}: {reason}", name_of(&path))
            }
            ImportEvent::Flight { order, parts, .. } =>
                format!("Flight {order}: action-cam take, {parts} file(s)"),
        };
        let line = format!("{}/{} videos imported · {} skipped · {} failed — {line}",
            self.imported, self.total, self.skipped, self.failed);
        Step { line, done: self.done, total: self.total }
    }
}

/// Counts a segmentation's recordings. A file is done once detected or
/// failed — not when it starts, or the bar shows full while the last one is
/// still being read.
#[derive(Debug)]
pub struct SegmentProgress {
    done: usize,
    total: usize,
}

impl SegmentProgress {
    /// `total`: the session's recordings that segment (its DVR files).
    pub fn new(total: usize) -> Self {
        Self { done: 0, total }
    }

    pub fn step(&mut self, event: SegmentationEvent) -> Step {
        let total = self.total;
        let line = match event {
            SegmentationEvent::Detecting(file) => format!(
                "Detecting {} ({}/{total})…", name_of(Path::new(&file.imported_path)), self.done + 1),
            SegmentationEvent::Detected(file, clips) => {
                self.done += 1;
                format!("{}: {clips} clip(s) ({}/{total})", name_of(Path::new(&file.imported_path)), self.done)
            }
            SegmentationEvent::Stitched(a, b) => format!(
                "One flight spans {} and {}", name_of(Path::new(&a)), name_of(Path::new(&b))),
            SegmentationEvent::Failed(file, reason) => {
                self.done += 1;
                format!("FAILED {}: {reason}", name_of(Path::new(&file.imported_path)))
            }
        };
        Step { line, done: self.done, total }
    }
}

/// The line a strip-making job shows: which flight, and how far through the
/// library — a job over a whole library is long, and a bare flight name with a
/// bar barely moving reads as stuck.
pub fn strip_line(done: usize, total: usize, label: &str) -> String {
    if label.is_empty() { format!("{total} frame(s) done") } else { format!("{label} ({}/{total})", done + 1) }
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fpv_core::{FrameRate, MediaInfo, SourceFile};
    use uuid::Uuid;

    use super::*;

    fn recording(name: &str) -> SourceFile {
        let media = MediaInfo::new(640, 480, FrameRate::new(1_000_000, 40_061), 372.7, "mjpeg");
        SourceFile::new(Uuid::new_v4(), format!("/card/{name}"), format!("/library/Alpes/{name}"), None, media)
    }

    #[test]
    fn counts_a_segmented_file_once_it_is_through_not_when_it_starts() {
        let mut progress = SegmentProgress::new(2);
        let a = recording("PICT0003.AVI");
        let b = recording("PICT0004.AVI");

        let step = progress.step(SegmentationEvent::Detecting(a.clone()));
        assert_eq!((step.line.as_str(), step.done, step.total), ("Detecting PICT0003.AVI (1/2)…", 0, 2));
        let step = progress.step(SegmentationEvent::Detected(a, 9));
        assert_eq!((step.line.as_str(), step.done), ("PICT0003.AVI: 9 clip(s) (1/2)", 1));
        let step = progress.step(SegmentationEvent::Detecting(b.clone()));
        assert_eq!((step.line.as_str(), step.done), ("Detecting PICT0004.AVI (2/2)…", 1));
        // A failure is through too: the bar reaches the end.
        let step = progress.step(SegmentationEvent::Failed(b, "cannot decode".into()));
        assert_eq!((step.line.as_str(), step.done), ("FAILED PICT0004.AVI: cannot decode", 2));
    }

    #[test]
    fn says_how_far_through_the_library_a_strip_job_is() {
        assert_eq!(strip_line(51, 2304, "Test OSMO — Flight 3"), "Test OSMO — Flight 3 (52/2304)");
        assert_eq!(strip_line(2304, 2304, ""), "2304 frame(s) done");
    }

    #[test]
    fn a_stitch_moves_no_count() {
        let mut progress = SegmentProgress::new(2);
        let step = progress.step(SegmentationEvent::Stitched("/l/PICT0030.AVI".into(), "/l/PICT0031.AVI".into()));
        assert_eq!((step.line.as_str(), step.done), ("One flight spans PICT0030.AVI and PICT0031.AVI", 0));
    }

    #[test]
    fn counts_an_imported_skipped_or_failed_file_and_takes_the_total_from_discovery() {
        let mut progress = ImportProgress::default();
        let step = progress.step(ImportEvent::Discovered { count: 3, sessions: 1 });
        assert_eq!((step.line.as_str(), step.done, step.total), ("0/3 videos imported · 0 skipped · 0 failed — 3 recording(s) → 1 session(s)", 0, 3));
        let step = progress.step(ImportEvent::Copying(PathBuf::from("/card/PICT0003.AVI")));
        assert_eq!((step.line.as_str(), step.done), ("0/3 videos imported · 0 skipped · 0 failed — Copying PICT0003.AVI…", 0));
        let step = progress.step(ImportEvent::Imported(Box::new(recording("PICT0003.AVI"))));
        assert_eq!(step.done, 1);
        assert!(step.line.starts_with("1/3 videos imported · 0 skipped · 0 failed — Imported PICT0003.AVI — "), "{}", step.line);
        assert!(step.line.ends_with(", 372.7 s"), "{}", step.line);
        assert_eq!(progress.step(ImportEvent::Skipped(PathBuf::from("/card/PICT0004.AVI"))).done, 2);
        let step = progress.step(ImportEvent::Failed(PathBuf::from("/card/PICT0005.AVI"), "unreadable".into()));
        assert_eq!((step.line.as_str(), step.done, step.total), ("1/3 videos imported · 1 skipped · 1 failed — FAILED PICT0005.AVI: unreadable", 3, 3));
        let step = progress.step(ImportEvent::Flight { session: "Alpes".into(), order: 2, parts: 1 });
        assert_eq!((step.line.as_str(), step.done), ("1/3 videos imported · 1 skipped · 1 failed — Flight 2: action-cam take, 1 file(s)", 3));
    }
}
