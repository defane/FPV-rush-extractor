//! Waiting for a flight to come down from the cloud drive before it plays.
//!
//! A File Provider such as Proton Drive holds a recording's first read until
//! the whole of it is down, so bytes say nothing: the wait counts recordings,
//! and only the ones already down say how long the rest will take.

/// The flight a click asked to play while its recordings were in the cloud,
/// and the task bringing them down.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudWait {
    pub task: u64,
    pub flight: String,
}

/// "Video 1 of 2": the recording being brought down, `done` of `total` being
/// whole on disk already.
pub fn wait_line(done: usize, total: usize) -> String {
    if total == 0 {
        return "Starting…".into();
    }
    format!("Video {} of {total}", (done + 1).min(total))
}

/// Time left at the pace of the recordings already down, `elapsed` seconds
/// after the download started; nothing before the first one is whole.
pub fn time_left(done: usize, total: usize, elapsed: f64) -> Option<String> {
    if done == 0 || done >= total || !elapsed.is_finite() || elapsed <= 0.0 {
        return None;
    }
    let seconds = elapsed / done as f64 * (total - done) as f64;
    Some(if seconds < 3.0 {
        "almost done".into()
    } else if seconds < 60.0 {
        format!("about {} s left", ((seconds / 5.0).round() * 5.0).max(5.0))
    } else {
        format!("about {} min left", (seconds / 60.0).round())
    })
}

impl crate::App {
    /// The wait itself: which video, how long, and the way out that leaves
    /// the download running and the flight unplayed.
    pub(crate) fn cloud_wait_window(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;
        let Some(wait) = self.cloud_wait.clone() else {
            return;
        };
        let Some(task) = self.tasks.iter().find(|task| task.id == wait.task) else {
            self.cloud_wait = None;
            return;
        };
        let (done, total) = (task.done, task.total);
        let elapsed = task.started.elapsed().as_secs_f64();
        let mut background = false;
        // Closed only by its button: a stray click must not drop the wait.
        egui::Modal::new(egui::Id::new("cloud-wait")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Downloading from the cloud");
            ui.separator();
            ui.label(format!(
                "{} is only in the cloud: it plays as soon as it is on this computer.",
                wait.flight
            ));
            let fraction = if total > 0 {
                done as f32 / total as f32
            } else {
                0.0
            };
            ui.add(egui::ProgressBar::new(fraction).animate(true));
            let mut line = wait_line(done, total);
            if let Some(left) = time_left(done, total, elapsed) {
                line = format!("{line} · {left}");
            }
            ui.label(line);
            ui.small(
                "The cloud drive does not say how far along a video is: each one arrives whole.",
            );
            ui.horizontal(|ui| {
                if ui.button("Continue in Background").clicked() {
                    background = true;
                }
            });
        });
        if background {
            self.detached.insert(wait.task);
            self.cloud_wait = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_videos_since_the_drive_hands_each_over_whole() {
        assert_eq!(wait_line(0, 0), "Starting…");
        assert_eq!(wait_line(0, 2), "Video 1 of 2");
        assert_eq!(wait_line(1, 2), "Video 2 of 2");
        // The last report comes once everything is down.
        assert_eq!(wait_line(2, 2), "Video 2 of 2");
    }

    #[test]
    fn says_how_long_only_once_a_video_is_down() {
        assert_eq!(time_left(0, 3, 40.0), None);
        // One video in 40 s, two to go.
        assert_eq!(time_left(1, 3, 40.0).as_deref(), Some("about 1 min left"));
        assert_eq!(time_left(2, 3, 24.0).as_deref(), Some("about 10 s left"));
        assert_eq!(time_left(3, 4, 3.0).as_deref(), Some("almost done"));
        assert_eq!(time_left(2, 2, 40.0), None);
        assert_eq!(time_left(1, 2, f64::NAN), None);
    }
}
