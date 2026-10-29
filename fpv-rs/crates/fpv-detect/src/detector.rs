use std::collections::BTreeMap;

use fpv_core::{ClipEntry, ClipExit};

use crate::{FrameAnalyzer, FrameMetrics};

/// Everything the detector reads off a threshold. The defaults are the values
/// tuned against the 24 hand-labelled clips and confirmed by leave-one-file-out,
/// where 10 of 12 folds pick exactly these (CLAUDE.md 10). Changing one without
/// re-running that harness is a guess, not an improvement.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectorSettings {
    /// Analysis rate. Boundaries land on this grid: 0.25 s, about 6 frames.
    pub sample_rate: f64,
    pub black_luma: f64,
    /// Below both of these, with no picture structure and no vertical
    /// correlation, the frame is analog snow. The pair sits *inside* the
    /// degraded band: the line being drawn is unusable versus usable, which only
    /// ground truth can place.
    pub snow_block_variation: f64,
    pub snow_vertical_correlation: f64,
    /// The goggles' own screens: far more flat area than a camera, and softer
    /// vertical correlation. Both halves are needed.
    pub menu_flat_fraction: f64,
    pub menu_vertical_correlation: f64,
    /// Consecutive frames required before a state change is believed.
    pub hysteresis: usize,
    /// Dropouts shorter than this are absorbed into the surrounding clip. 3 s was
    /// the intuition; the labelled set overruled it.
    pub minimum_gap: f64,
    pub minimum_clip_duration: f64,
    /// Median coherence a clip needs to be a real feed rather than two
    /// transmitters beating. Judged per clip, never per frame — as a frame state
    /// the same metric made things markedly worse.
    pub minimum_clip_coherence: f64,
    pub coherence_window: f64,
}

impl Default for DetectorSettings {
    fn default() -> Self {
        Self {
            sample_rate: 4.0,
            black_luma: 22.0,
            snow_block_variation: 0.60,
            snow_vertical_correlation: 0.92,
            menu_flat_fraction: 0.10,
            menu_vertical_correlation: 0.90,
            hysteresis: 3,
            minimum_gap: 0.5,
            minimum_clip_duration: 5.0,
            minimum_clip_coherence: 0.50,
            coherence_window: 3.0,
        }
    }
}

impl DetectorSettings {
    /// The thresholds for a session's profile. `Standard` is `default()`,
    /// untouched. `Textured` was measured on the pilot's PICT0003 (Alps, one
    /// continuous six-minute flight by its OSD timer): its clean picture never
    /// drops under a vertical correlation of ~0.80 while its real snow stays
    /// under ~0.77, so the snow line moves to 0.78; and the reviewer wants its
    /// sub-second dropouts absorbed, so the gap grows to 1 s. That gives one
    /// clip, 0–372.5 s, where the standard profile gives nine. One file is
    /// thin ground — `spike/labels-alpes.csv` is where more of it goes.
    pub fn for_profile(profile: fpv_core::DetectionProfile) -> Self {
        match profile {
            fpv_core::DetectionProfile::Standard => Self::default(),
            fpv_core::DetectionProfile::Textured => Self {
                snow_vertical_correlation: 0.78,
                minimum_gap: 1.0,
                ..Self::default()
            },
        }
    }

    /// What a `DetectionRun` records, keyed as the Swift pipeline keys them.
    pub fn parameters(&self) -> BTreeMap<String, f64> {
        [
            ("sampleRate", self.sample_rate),
            ("blackLuma", self.black_luma),
            ("snowBlockVariation", self.snow_block_variation),
            ("snowVerticalCorrelation", self.snow_vertical_correlation),
            ("menuFlatFraction", self.menu_flat_fraction),
            ("menuVerticalCorrelation", self.menu_vertical_correlation),
            ("hysteresis", self.hysteresis as f64),
            ("minimumGap", self.minimum_gap),
            ("minimumClipDuration", self.minimum_clip_duration),
            ("minimumClipCoherence", self.minimum_clip_coherence),
            ("coherenceWindow", self.coherence_window),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameState {
    Black,
    NoSignal,
    Menu,
    Signal,
}

/// A clip as the detector sees it, in seconds within one file.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedClip {
    pub start: f64,
    pub end: f64,
    pub exit: ClipExit,
    /// How the clip opened. The detector reads no OSD (CLAUDE.md 4.0), so this
    /// is `Unknown` here; the pipeline marks a DVR cut's continuation.
    pub entry: ClipEntry,
    pub confidence: f64,
    pub coherence: f64,
    /// Set when the clip touches a file edge, which means it may be one half of a
    /// recording the DVR split. Such a clip is exempt from the minimum duration:
    /// its real length is unknown until the files either side are joined.
    pub at_file_start: bool,
    pub at_file_end: bool,
}

impl DetectedClip {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Segment {
    pub state: FrameState,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ClipDetector {
    pub settings: DetectorSettings,
}

impl ClipDetector {
    pub fn new(settings: DetectorSettings) -> Self {
        Self { settings }
    }

    // Per-frame

    pub fn classify(&self, m: &FrameMetrics) -> FrameState {
        let s = &self.settings;
        if m.luma < s.black_luma {
            FrameState::Black
        } else if m.block_variation < s.snow_block_variation
            && m.vertical_correlation < s.snow_vertical_correlation
        {
            FrameState::NoSignal
        } else if m.flat_fraction > s.menu_flat_fraction
            && m.vertical_correlation < s.menu_vertical_correlation
        {
            FrameState::Menu
        } else {
            FrameState::Signal
        }
    }

    /// Fills in each frame's scene correlation and windowed coherence. Both need
    /// the neighbours, so they cannot live in the per-frame analysis.
    pub fn add_sequence_metrics(&self, metrics: &mut [FrameMetrics], small_frames: &[Vec<f64>]) {
        for index in 0..metrics.len() {
            metrics[index].scene_correlation = if index == 0 {
                1.0
            } else {
                FrameAnalyzer::correlation(&small_frames[index - 1], &small_frames[index])
            };
        }
        let window = ((self.settings.coherence_window * self.settings.sample_rate).round() as usize)
            .max(1);
        let correlations: Vec<f64> = metrics.iter().map(|m| m.scene_correlation).collect();
        for (index, frame) in metrics.iter_mut().enumerate() {
            let lower = index.saturating_sub(window / 2);
            let upper = correlations.len().min(index + window / 2 + 1);
            frame.coherence = median(&correlations[lower..upper]);
        }
    }

    // State machine

    /// Debounces raw per-frame states into runs. A transition is dated at the
    /// first frame of the new state, not at the frame that confirms it, so a
    /// boundary does not lag by the whole hysteresis window.
    pub(crate) fn segments(&self, times: &[f64], states: &[FrameState]) -> Vec<Segment> {
        let mut segments = Vec::new();
        let mut current: Option<FrameState> = None;
        let mut current_start = 0.0;
        let mut pending: Option<FrameState> = None;
        let mut pending_count = 0;
        let mut pending_start = 0.0;

        for (&time, &state) in times.iter().zip(states) {
            let Some(now) = current else {
                current = Some(state);
                current_start = time;
                continue;
            };
            if state == now {
                pending = None;
                pending_count = 0;
                continue;
            }
            if pending == Some(state) {
                pending_count += 1;
            } else {
                pending = Some(state);
                pending_count = 1;
                pending_start = time;
            }
            if pending_count >= self.settings.hysteresis {
                segments.push(Segment { state: now, start: current_start, end: pending_start });
                current = Some(state);
                current_start = pending_start;
                pending = None;
                pending_count = 0;
            }
        }
        if let (Some(state), Some(&last)) = (current, times.last()) {
            segments.push(Segment { state, start: current_start, end: last });
        }
        segments
    }

    /// Signal runs, with short dropouts absorbed: losing the picture for a moment
    /// behind a tree is the same clip, not a new flight.
    pub(crate) fn signal_runs(&self, segments: &[Segment]) -> Vec<(f64, f64)> {
        let mut runs: Vec<(f64, f64)> = Vec::new();
        for segment in segments.iter().filter(|s| s.state == FrameState::Signal) {
            match runs.last_mut() {
                Some(last) if segment.start - last.1 < self.settings.minimum_gap => {
                    last.1 = segment.end;
                }
                _ => runs.push((segment.start, segment.end)),
            }
        }
        runs
    }

    // Clips

    pub fn detect(&self, times: &[f64], metrics: &[FrameMetrics]) -> Vec<DetectedClip> {
        let Some(&duration) = times.last() else { return vec![] };
        let states: Vec<FrameState> = metrics.iter().map(|m| self.classify(m)).collect();
        let segments = self.segments(times, &states);
        let edge = 1.0 / self.settings.sample_rate + 0.01;

        self.signal_runs(&segments)
            .into_iter()
            .map(|(start, end)| {
                let next = segments.iter().find(|s| s.start >= end && s.state != FrameState::Signal);
                let window: Vec<f64> = times.iter().zip(metrics)
                    .filter(|&(&t, _)| start <= t && t <= end)
                    .map(|(_, m)| m.coherence)
                    .collect();
                DetectedClip {
                    start, end,
                    exit: exit_for(next.map(|s| s.state)),
                    entry: ClipEntry::Unknown,
                    confidence: 0.9,
                    coherence: median(&window),
                    at_file_start: start <= edge,
                    at_file_end: end >= duration - edge,
                }
            })
            .filter(|clip| {
                clip.duration() >= self.settings.minimum_clip_duration
                    || clip.at_file_start || clip.at_file_end
            })
            // Two transmitters beating on neighbouring channels put a picture on
            // screen, but a differently broken one every frame. Whole clips go on
            // that, never individual frames.
            .filter(|clip| clip.coherence >= self.settings.minimum_clip_coherence)
            .collect()
    }
}

fn exit_for(state: Option<FrameState>) -> ClipExit {
    match state {
        Some(FrameState::NoSignal) => ClipExit::SignalLoss,
        Some(FrameState::Black) => ClipExit::Black,
        Some(FrameState::Menu) => ClipExit::Menu,
        Some(FrameState::Signal) | None => ClipExit::Eof,
    }
}

pub(crate) fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metric values standing for each state, taken from the measured
    // distributions rather than invented: real snow, a real picture, a real
    // goggle screen.
    fn snow() -> FrameMetrics { FrameMetrics::new(47.0, 1153.0, 0.10, 0.65, 0.0) }
    fn picture() -> FrameMetrics { FrameMetrics::new(157.0, 222.0, 1.64, 0.97, 0.0) }
    fn menu() -> FrameMetrics { FrameMetrics::new(87.0, 1287.0, 1.52, 0.84, 0.125) }
    fn black() -> FrameMetrics { FrameMetrics::new(3.0, 0.0, 0.0, 0.0, 1.0) }

    /// A 4 Hz sequence from runs of (metric, seconds), all at one coherence.
    fn sequence(runs: &[(FrameMetrics, f64)], coherence: f64) -> (Vec<f64>, Vec<FrameMetrics>) {
        let (mut times, mut metrics) = (vec![], vec![]);
        for &(value, seconds) in runs {
            for _ in 0..(seconds * 4.0) as usize {
                times.push(times.len() as f64 / 4.0);
                metrics.push(FrameMetrics { coherence, ..value });
            }
        }
        (times, metrics)
    }

    fn detect(runs: &[(FrameMetrics, f64)]) -> Vec<DetectedClip> {
        let (times, metrics) = sequence(runs, 1.0);
        ClipDetector::default().detect(&times, &metrics)
    }

    #[test]
    fn indoor_banded_snow_at_115_and_210_seconds_is_not_a_usable_signal() {
        // Measured grayscale metrics from the private Indoor regression video.
        // Both profiles already recognize this snow; codec-based bypass was the failure.
        let samples = [
            FrameMetrics::new(130.632959, 9562.774334, 0.053771, 0.732424, 0.0),
            FrameMetrics::new(129.065231, 9346.220556, 0.048923, 0.730113, 0.0),
        ];
        for profile in [fpv_core::DetectionProfile::Standard, fpv_core::DetectionProfile::Textured] {
            let detector = ClipDetector::new(DetectorSettings::for_profile(profile));
            for sample in &samples { assert_eq!(detector.classify(sample), FrameState::NoSignal); }
        }
    }

    #[test]
    fn keeps_the_standard_profile_exactly_the_tuned_default() {
        assert_eq!(DetectorSettings::for_profile(fpv_core::DetectionProfile::Standard),
                   DetectorSettings::default());
    }

    #[test]
    fn reads_a_textured_rock_face_as_picture_under_the_textured_profile() {
        // PICT0003 @36.5 s: a clean picture of a rock face, read as snow by the
        // standard thresholds.
        let rock = FrameMetrics::new(123.0, 900.0, 0.578, 0.871, 0.0);
        assert_eq!(ClipDetector::default().classify(&rock), FrameState::NoSignal);
        let textured = ClipDetector::new(DetectorSettings::for_profile(fpv_core::DetectionProfile::Textured));
        assert_eq!(textured.classify(&rock), FrameState::Signal);
        // Its real snow (@94.5 s) is still snow.
        assert_eq!(textured.classify(&FrameMetrics::new(80.0, 1500.0, 0.282, 0.677, 0.0)),
                   FrameState::NoSignal);
    }

    #[test]
    fn reads_each_state_off_the_measured_thresholds() {
        let detector = ClipDetector::default();
        assert_eq!(detector.classify(&snow()), FrameState::NoSignal);
        assert_eq!(detector.classify(&picture()), FrameState::Signal);
        assert_eq!(detector.classify(&menu()), FrameState::Menu);
        assert_eq!(detector.classify(&black()), FrameState::Black);
    }

    #[test]
    fn cuts_a_clip_where_picture_replaces_snow() {
        let clips = detect(&[(snow(), 10.0), (picture(), 30.0), (snow(), 10.0)]);
        assert_eq!(clips.len(), 1);
        assert_eq!((clips[0].start, clips[0].end), (10.0, 40.0));
        assert_eq!(clips[0].exit, ClipExit::SignalLoss);
    }

    #[test]
    fn dates_a_transition_at_its_first_frame_not_the_confirming_one() {
        // With hysteresis 3 the change is believed on the third frame, but the
        // boundary belongs at the first, or every clip starts 0.75 s late.
        let clips = detect(&[(snow(), 10.0), (picture(), 30.0)]);
        assert_eq!(clips[0].start, 10.0);
    }

    #[test]
    fn absorbs_a_dropout_too_short_to_be_a_new_flight() {
        let clips = detect(&[(picture(), 20.0), (snow(), 0.25), (picture(), 20.0), (snow(), 5.0)]);
        assert_eq!(clips.len(), 1);
        assert!(clips[0].duration() > 39.0);
    }

    #[test]
    fn splits_on_a_dropout_long_enough_to_matter() {
        let clips = detect(&[(picture(), 20.0), (snow(), 6.0), (picture(), 20.0), (snow(), 5.0)]);
        assert_eq!(clips.len(), 2);
    }

    #[test]
    fn keeps_a_short_clip_that_touches_a_file_edge() {
        // The 0.75 s tail of PICT0030 is the opening of the flight that fills
        // PICT0031. Below the minimum duration, and still a real clip.
        let clips = detect(&[(snow(), 20.0), (picture(), 0.75)]);
        assert_eq!(clips.len(), 1);
        assert!(clips[0].at_file_end);
        assert!(clips[0].duration() < 1.0);
    }

    #[test]
    fn discards_a_short_clip_in_the_middle_of_a_file() {
        assert!(detect(&[(snow(), 20.0), (picture(), 2.0), (snow(), 20.0)]).is_empty());
    }

    #[test]
    fn drops_a_whole_clip_that_is_another_pilots_interference() {
        // A picture is on screen throughout, but torn differently every frame.
        let (times, metrics) = sequence(&[(snow(), 10.0), (picture(), 30.0), (snow(), 10.0)], 0.15);
        assert!(ClipDetector::default().detect(&times, &metrics).is_empty());
    }

    #[test]
    fn ends_a_clip_on_the_goggle_menu() {
        let clips = detect(&[(picture(), 30.0), (menu(), 10.0)]);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].exit, ClipExit::Menu);
    }

    #[test]
    fn takes_the_median_of_an_even_window_as_the_mean_of_the_middle_two() {
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[]), 1.0);
    }
}
