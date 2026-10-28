//! Hover strips made ahead of the pointer, in batches.
//!
//! The library has one owner thread and runs its jobs one after the other, so
//! a library of 4K takes (~2300 frames) made in one job would hold every other
//! job back for minutes. Each batch is its own job: whatever was asked for in
//! the meantime runs between two batches.

/// The frames of one hover strip, for the cards and the job alike: frames are
/// cached by index.
pub const STRIP: usize = 24;
/// Frames per job: one long take's strip.
pub const BATCH: usize = 24;

/// Which strips job runs, and whether another pass was asked for meanwhile.
#[derive(Debug, Default)]
pub struct Strips {
    running: bool,
    again: bool,
    /// The session on screen when it was last asked, made first.
    first: Option<String>,
    /// Frames made since the job started, for the task bar.
    pub made: usize,
    /// Flights already reported as failing, so each is reported once.
    reported: Vec<String>,
}

impl Strips {
    /// Asked for (app open, an import or a segmentation ended, the boundary
    /// editor saved). Returns the batch to start now, if none runs: requests
    /// during a run buy exactly one more pass.
    pub fn request(&mut self, first: Option<String>) -> Option<Option<String>> {
        self.first = first;
        if self.running {
            self.again = true;
            return None;
        }
        self.running = true;
        self.made = 0;
        Some(self.first.clone())
    }

    /// A batch ended: `made` frames, `more` left over, `failed` (flight,
    /// reason). Returns the next batch to start, if any, and the failures not
    /// reported before. A batch that made nothing ends the pass — the frames
    /// it left are those of recordings that cannot be read, or the budget is
    /// reached — so a broken recording never loops.
    pub fn finished(
        &mut self,
        made: usize,
        more: bool,
        failed: Vec<(String, String)>,
    ) -> (Option<Option<String>>, Vec<(String, String)>) {
        self.made += made;
        let new: Vec<_> = failed
            .into_iter()
            .filter(|(flight, _)| !self.reported.contains(flight))
            .collect();
        self.reported
            .extend(new.iter().map(|(flight, _)| flight.clone()));
        // Carrying on keeps a pending request for one more pass afterwards.
        let next = if (more && made > 0) || std::mem::take(&mut self.again) {
            Some(self.first.clone())
        } else {
            self.running = false;
            None
        };
        (next, new)
    }

    /// The batch failed outright: the pass ends, a later request starts anew.
    pub fn failed(&mut self) {
        self.running = false;
        self.again = false;
    }

    pub fn running(&self) -> bool {
        self.running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_the_library_in_batches_the_session_on_screen_first() {
        let mut strips = Strips::default();
        assert_eq!(
            strips.request(Some("alpes".into())),
            Some(Some("alpes".into()))
        );
        assert!(strips.running());
        assert_eq!(
            strips.finished(24, true, vec![]).0,
            Some(Some("alpes".into()))
        );
        assert_eq!(strips.finished(10, false, vec![]).0, None);
        assert_eq!(strips.made, 34);
        assert!(!strips.running());
    }

    #[test]
    fn never_runs_two_and_runs_one_more_pass_for_what_changed_meanwhile() {
        let mut strips = Strips::default();
        strips.request(None);
        // Two segmentations end while the first pass runs: one more pass, not two.
        assert_eq!(strips.request(Some("letz".into())), None);
        assert_eq!(strips.request(Some("letz".into())), None);
        assert_eq!(
            strips.finished(5, false, vec![]).0,
            Some(Some("letz".into()))
        );
        assert_eq!(strips.finished(3, false, vec![]).0, None);
        // Done: the next request starts afresh.
        assert_eq!(strips.request(None), Some(None));
        assert_eq!(strips.made, 0);
    }

    #[test]
    fn stops_when_a_batch_makes_nothing_and_reports_a_broken_flight_once() {
        let mut strips = Strips::default();
        strips.request(None);
        let broken = || vec![("Letz — Flight 2".to_owned(), "ffmpeg exited 1".to_owned())];
        let (next, new) = strips.finished(20, true, broken());
        assert_eq!((next, new.len()), (Some(None), 1));
        // Only the broken flight's frames are left: no endless loop, no second report.
        let (next, new) = strips.finished(0, true, broken());
        assert_eq!((next, new.len()), (None, 0));
        assert!(!strips.running());
    }

    #[test]
    fn a_failed_batch_ends_the_pass() {
        let mut strips = Strips::default();
        strips.request(None);
        strips.request(None);
        strips.failed();
        assert!(!strips.running());
        assert_eq!(strips.request(None), Some(None));
    }
}
