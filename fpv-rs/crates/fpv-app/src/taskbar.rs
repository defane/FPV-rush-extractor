//! The task bar's words: how many run, how long each took, how far they are.

/// "No task running", "1 task running", "3 tasks running", or — none running
/// — "1 failed" until the finished ones are cleared.
pub fn summary(running: usize, failed: usize) -> String {
    match (running, failed) {
        (0, 0) => "No task running".into(),
        (0, failed) => format!("{failed} failed"),
        (1, _) => "1 task running".into(),
        (running, _) => format!("{running} tasks running"),
    }
}

/// "12 s", "2 min 05 s", "1 h 03 min".
pub fn elapsed(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0) as u64
    } else {
        0
    };
    match seconds {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min {:02} s", s / 60, s % 60),
        s => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

/// The running tasks' mean progress, from those that know their total.
pub fn overall(progress: impl IntoIterator<Item = (usize, usize)>) -> Option<f32> {
    let fractions: Vec<f32> = progress
        .into_iter()
        .filter(|&(_, total)| total > 0)
        .map(|(done, total)| (done as f32 / total as f32).min(1.0))
        .collect();
    (!fractions.is_empty()).then(|| fractions.iter().sum::<f32>() / fractions.len() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_many_run_or_failed() {
        assert_eq!(summary(0, 0), "No task running");
        assert_eq!(summary(0, 2), "2 failed");
        assert_eq!(summary(1, 2), "1 task running");
        assert_eq!(summary(3, 0), "3 tasks running");
    }

    #[test]
    fn says_how_long_a_task_took() {
        assert_eq!(elapsed(12.7), "12 s");
        assert_eq!(elapsed(125.0), "2 min 05 s");
        assert_eq!(elapsed(3780.0), "1 h 03 min");
        assert_eq!(elapsed(f64::NAN), "0 s");
    }

    #[test]
    fn averages_the_progress_of_what_knows_its_total() {
        assert_eq!(overall([]), None);
        assert_eq!(overall([(3, 0)]), None);
        assert_eq!(overall([(1, 2), (3, 4), (5, 0)]), Some(0.625));
    }
}
