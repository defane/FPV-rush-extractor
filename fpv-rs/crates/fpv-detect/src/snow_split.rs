//! Snow Split: positive local noise evidence, confirmed over time.
//!
//! Calibration corpus: EACHINE Flight 12 plus synthetic negatives. These
//! are not replacements for the separately tuned Standard/Textured classifiers.
use crate::{DetectedClip, FrameAnalyzer, FrameMetrics, FrameState};
use fpv_core::{ClipEntry, ClipExit};

pub const VERSION: &str = "snow-split-2";
const PATCHES: usize = FrameAnalyzer::BLOCK_ROWS * FrameAnalyzer::BLOCK_COLUMNS;
const MIN_HIGH_PASS: f64 = 0.5;
const MIN_VERTICAL_ENERGY: f64 = 16.0;
const MAX_FINE_CORRELATION: f64 = 0.85;
const MAX_COARSE_CORRELATION: f64 = 0.5;
const MAX_TEMPORAL_CORRELATION: f64 = 0.5;
const MIN_COVERAGE: f64 = 0.5;
const MIN_FROZEN_COVERAGE: f64 = 0.9;
const MIN_FROZEN_CORRELATION: f64 = 0.98;

/// Masks contain only patch votes, never image pixels. Unknown evidence is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SnowSplitEvidence {
    spatial: u64,
    temporal: u64,
    frozen: u64,
}
impl SnowSplitEvidence {
    pub fn spatial_coverage(self) -> f64 {
        self.spatial.count_ones() as f64 / PATCHES as f64
    }
    pub fn confirmed_coverage(self) -> f64 {
        (self.spatial & self.temporal).count_ones() as f64 / PATCHES as f64
    }
    pub fn frozen_coverage(self) -> f64 {
        self.frozen.count_ones() as f64 / PATCHES as f64
    }
    pub(crate) fn is_snow(self) -> bool {
        self.confirmed_coverage() > MIN_COVERAGE || self.frozen_coverage() >= MIN_FROZEN_COVERAGE
    }

    pub(crate) fn analyze(analyzer: &FrameAnalyzer, pixels: &[u8], small: &[f64]) -> Self {
        let mut evidence = Self::default();
        let w = analyzer.width;
        let pw = w / FrameAnalyzer::BLOCK_COLUMNS;
        let ph = analyzer.height / FrameAnalyzer::BLOCK_ROWS;
        assert_eq!(pixels.len(), w * analyzer.height);
        assert!(pw >= 2 && ph >= 2);
        for row in 0..FrameAnalyzer::BLOCK_ROWS {
            for col in 0..FrameAnalyzer::BLOCK_COLUMNS {
                let mut histogram = [0usize; 511];
                let mut upper = Vec::new();
                let mut lower = Vec::new();
                let mut energy = 0.0;
                let mut difference_sum = 0.0;
                for y in (row * ph..(row + 1) * ph - 1).step_by(2) {
                    for x in (col * pw..(col + 1) * pw - 1).step_by(2) {
                        let a = pixels[y * w + x] as i32;
                        let b = pixels[y * w + x + 1] as i32;
                        let c = pixels[(y + 1) * w + x] as i32;
                        let d = pixels[(y + 1) * w + x + 1] as i32;
                        histogram[(a - b - c + d).unsigned_abs() as usize] += 1;
                        upper.push(a as f64);
                        lower.push(c as f64);
                        energy += ((a - c) * (a - c)) as f64;
                        difference_sum += (a - c) as f64;
                    }
                }
                let count = upper.len();
                let mut cumulative = 0;
                let high_pass = histogram
                    .iter()
                    .position(|&n| {
                        cumulative += n;
                        cumulative > count / 2
                    })
                    .unwrap_or(0) as f64
                    / 2.0;
                let mean_difference = difference_sum / count as f64;
                let residual_energy = energy / count as f64 - mean_difference * mean_difference;
                if high_pass < MIN_HIGH_PASS || residual_energy < MIN_VERTICAL_ENERGY {
                    continue;
                }
                if FrameAnalyzer::correlation(&upper, &lower).abs() >= MAX_FINE_CORRELATION {
                    continue;
                }
                let coarse_width = FrameAnalyzer::SMALL_WIDTH / FrameAnalyzer::BLOCK_COLUMNS;
                let coarse_height = FrameAnalyzer::SMALL_HEIGHT / FrameAnalyzer::BLOCK_ROWS;
                let patch: Vec<_> = (row * coarse_height..(row + 1) * coarse_height)
                    .flat_map(|y| {
                        small[y * FrameAnalyzer::SMALL_WIDTH + col * coarse_width
                            ..y * FrameAnalyzer::SMALL_WIDTH + (col + 1) * coarse_width]
                            .iter()
                            .copied()
                    })
                    .collect();
                if FrameAnalyzer::correlation(
                    &patch[..patch.len() - coarse_width],
                    &patch[coarse_width..],
                )
                .abs()
                    >= MAX_COARSE_CORRELATION
                {
                    continue;
                }
                evidence.spatial |= 1 << (row * FrameAnalyzer::BLOCK_COLUMNS + col);
            }
        }
        evidence
    }
}

// Fine bandpass residuals expose changing grain even when a coarse, faint
// picture remains underneath it. Only the immediately preceding signature is
// retained, so this adds bounded memory rather than buffering full-size frames.
struct ScaleSignature {
    spatial: u64,
    bandpass: Vec<f64>,
}
#[derive(Default)]
pub(crate) struct SnowSplitSampler {
    previous: Option<ScaleSignature>,
    seen: usize,
}
impl SnowSplitSampler {
    fn signature(analyzer: &FrameAnalyzer, pixels: &[u8], small: &[f64]) -> ScaleSignature {
        const SIDE: usize = 8;
        let spatial = SnowSplitEvidence::analyze(analyzer, pixels, small).spatial;
        let mut bandpass = Vec::with_capacity(PATCHES * SIDE * SIDE);
        let pw = analyzer.width / FrameAnalyzer::BLOCK_COLUMNS;
        let ph = analyzer.height / FrameAnalyzer::BLOCK_ROWS;
        for row in 0..FrameAnalyzer::BLOCK_ROWS {
            for col in 0..FrameAnalyzer::BLOCK_COLUMNS {
                for y in 0..SIDE {
                    for x in 0..SIDE {
                        let y = row * ph + (y * ph / SIDE).min(ph - 2);
                        let x = col * pw + (x * pw / SIDE).min(pw - 2);
                        let at = y * analyzer.width + x;
                        bandpass.push(
                            (pixels[at] as f64
                                - pixels[at + 1] as f64
                                - pixels[at + analyzer.width] as f64
                                + pixels[at + analyzer.width + 1] as f64)
                                / 2.0,
                        );
                    }
                }
            }
        }
        ScaleSignature { spatial, bandpass }
    }
    pub(crate) fn analyze(
        &mut self,
        analyzer: &FrameAnalyzer,
        pixels: &[u8],
        small: &[f64],
        previous_frame: Option<&mut FrameMetrics>,
    ) -> SnowSplitEvidence {
        const SAMPLES: usize = 64;
        let current = Self::signature(analyzer, pixels, small);
        let mut evidence = SnowSplitEvidence {
            spatial: current.spatial,
            temporal: 0,
            frozen: 0,
        };
        let mut first_confirmed = 0;
        let mut frozen = 0;
        if let Some(previous) = &self.previous {
            for i in 0..PATCHES {
                let range = i * SAMPLES..(i + 1) * SAMPLES;
                let correlation = FrameAnalyzer::correlation(
                    &current.bandpass[range.clone()],
                    &previous.bandpass[range],
                )
                .abs();
                if correlation >= MIN_FROZEN_CORRELATION {
                    frozen |= current.spatial & previous.spatial & (1 << i);
                }
                if correlation < MAX_TEMPORAL_CORRELATION {
                    evidence.temporal |= current.spatial & (1 << i);
                    first_confirmed |= previous.spatial & (1 << i);
                }
            }
            if self.seen == 1
                && let Some(frame) = previous_frame
                && let Some(first) = frame.snow_split.as_mut()
            {
                first.temporal = first_confirmed;
                first.frozen = frozen;
            }
        }
        evidence.frozen = frozen;
        self.seen += 1;
        self.previous = Some(current);
        evidence
    }
}

pub(crate) fn parameters() -> Vec<(String, f64)> {
    [
        ("snowSplit", 1.0),
        ("snowSplitFrozenMinCoverage", MIN_FROZEN_COVERAGE),
        ("snowSplitFrozenMinCorrelation", MIN_FROZEN_CORRELATION),
        ("snowSplitMaxScaleCount", 1.0),
        ("snowSplitSignatureSide", 8.0),
        ("snowSplitMaxAnalysisWidth", 640.0),
        ("snowSplitMaxAnalysisHeight", 480.0),
        ("snowSplitMinHighPass", MIN_HIGH_PASS),
        ("snowSplitMinVerticalEnergy", MIN_VERTICAL_ENERGY),
        ("snowSplitMaxFineCorrelation", MAX_FINE_CORRELATION),
        ("snowSplitMaxCoarseCorrelation", MAX_COARSE_CORRELATION),
        ("snowSplitMaxTemporalCorrelation", MAX_TEMPORAL_CORRELATION),
        ("snowSplitMinCoverage", MIN_COVERAGE),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v))
    .collect()
}

/// Complement of confirmed snow runs. No black/menu, minimum-duration,
/// whole-clip coherence filter, or merging across confirmed snow is applied.
pub(crate) fn detect(
    times: &[f64],
    metrics: &[FrameMetrics],
    states: &[FrameState],
    rate: f64,
    confirmations: usize,
) -> Vec<DetectedClip> {
    let n = times.len().min(metrics.len()).min(states.len());
    if n == 0 {
        return vec![];
    }
    let end = times[n - 1] + 1.0 / rate;
    let mut clips = Vec::new();
    let mut cursor = times[0];
    let mut i = 0;
    let make = |start: f64, stop: f64, exit: ClipExit| {
        let values: Vec<_> = times[..n]
            .iter()
            .zip(&metrics[..n])
            .filter(|(t, _)| start <= **t && **t < stop)
            .map(|(_, m)| m.coherence)
            .collect();
        DetectedClip {
            start,
            end: stop,
            exit,
            entry: ClipEntry::Unknown,
            confidence: 0.9,
            coherence: crate::detector::median(&values),
            at_file_start: start == times[0],
            at_file_end: exit == ClipExit::Eof,
        }
    };
    while i < n {
        if states[i] != FrameState::NoSignal {
            i += 1;
            continue;
        }
        let first = i;
        while i < n && states[i] == FrameState::NoSignal {
            i += 1;
        }
        if i - first < confirmations.max(1) {
            continue;
        }
        let loss = times[first];
        if cursor < loss {
            clips.push(make(cursor, loss, ClipExit::SignalLoss));
        }
        cursor = if i < n { times[i] } else { end };
    }
    if cursor < end {
        clips.push(make(cursor, end, ClipExit::Eof));
    }
    clips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipDetector, DetectorSettings};
    use fpv_core::DetectionProfile;
    fn noise(seed: u32, overlay: bool) -> Vec<u8> {
        let mut state = seed;
        let mut pixels = vec![0; 640 * 480];
        for y in 0..480 {
            let mut previous = 128u16;
            for x in 0..640 {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let value = (state >> 24) as u16;
                pixels[y * 640 + x] = ((value + previous) / 2) as u8;
                previous = value;
            }
        }
        if overlay {
            for y in 0..120 {
                for x in 0..640 {
                    pixels[y * 640 + x] = if (x / 8 + y / 8) % 2 == 0 { 32 } else { 220 };
                }
            }
        }
        pixels
    }
    fn analyze(frames: &[Vec<u8>]) -> (ClipDetector, Vec<f64>, Vec<FrameMetrics>) {
        let analyzer = FrameAnalyzer::default();
        let detector =
            ClipDetector::new(DetectorSettings::for_profile(DetectionProfile::SnowSplit));
        let small: Vec<_> = frames.iter().map(|p| analyzer.downsample(p)).collect();
        let mut metrics = Vec::new();
        let mut sampler = SnowSplitSampler::default();
        for (pixels, small) in frames.iter().zip(&small) {
            let mut m = analyzer.metrics(pixels);
            m.snow_split = Some(sampler.analyze(&analyzer, pixels, small, metrics.last_mut()));
            metrics.push(m);
        }
        detector.add_sequence_metrics(&mut metrics, &small);
        (
            detector,
            (0..frames.len()).map(|i| i as f64 / 4.0).collect(),
            metrics,
        )
    }
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/tests/fixtures/{name}.gray",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }
    #[test]
    fn positive_spatial_noise_evidence_separates_reference_picture_menu_and_snow() {
        let analyzer = FrameAnalyzer::default();
        for (name, snow) in [("picture", false), ("menu", false), ("snow", true)] {
            let pixels = fixture(name);
            let e = SnowSplitEvidence::analyze(&analyzer, &pixels, &analyzer.downsample(&pixels));
            assert_eq!(e.spatial_coverage() >= MIN_COVERAGE, snow, "{name}: {e:?}");
        }
    }
    #[test]
    fn all_dynamic_snow_and_snow_with_osd_produce_no_clip() {
        for overlay in [false, true] {
            let frames: Vec<_> = (0..12).map(|i| noise(123 + i, overlay)).collect();
            let (d, t, m) = analyze(&frames);
            assert!(
                m.iter().all(|m| d.classify(m) == FrameState::NoSignal),
                "{m:?}"
            );
            assert!(d.detect(&t, &m).is_empty());
        }
    }
    #[test]
    fn confirmed_snow_keeps_even_one_sample_of_recovered_picture_or_menu() {
        let mut frames: Vec<_> = (0..4).map(|i| noise(23 + i, false)).collect();
        frames.push(fixture("picture"));
        frames.extend((4..8).map(|i| noise(23 + i, false)));
        frames.push(fixture("menu"));
        frames.extend((8..12).map(|i| noise(23 + i, false)));
        let (d, t, mut m) = analyze(&frames);
        for m in &mut m {
            m.coherence = -1.0;
        }
        let clips = d.detect(&t, &m);
        assert_eq!(
            clips
                .iter()
                .map(|c| (c.start, c.end, c.exit))
                .collect::<Vec<_>>(),
            [
                (1.0, 1.25, ClipExit::SignalLoss),
                (2.25, 2.5, ClipExit::SignalLoss)
            ]
        );
    }
    #[test]
    fn static_picture_black_menu_dim_grain_and_moving_fine_texture_remain_usable() {
        let mut frames = vec![fixture("picture"); 4];
        frames.extend(vec![vec![0; 640 * 480]; 4]);
        frames.extend(vec![fixture("menu"); 4]);
        for i in 0..4 {
            frames.push(noise(66 + i, false).iter().map(|&p| 16 + (p % 5)).collect());
        }
        for i in 0..4 {
            frames.push(
                (0..640 * 480)
                    .map(|p| {
                        if (p % 640 + p / 640 + i) % 2 == 0 {
                            32
                        } else {
                            220
                        }
                    })
                    .collect(),
            );
        }
        let (d, t, m) = analyze(&frames);
        let clips = d.detect(&t, &m);
        assert_eq!(clips.len(), 1, "{clips:?}");
        assert_eq!(
            (clips[0].start, clips[0].end, clips[0].exit),
            (0.0, 5.0, ClipExit::Eof)
        );
    }
    #[test]
    fn frozen_spatially_random_snow_is_excluded() {
        let frames = vec![noise(123, false); 12];
        let (d, t, m) = analyze(&frames);
        assert!(
            d.detect(&t, &m).is_empty(),
            "spatial coverage {}",
            m[0].snow_split.unwrap().spatial_coverage()
        );
    }
    #[test]
    fn isolated_noise_candidates_do_not_cut_and_usable_eof_is_not_lost() {
        let mut frames = vec![fixture("picture"); 4];
        frames.push(noise(22, false));
        frames.push(fixture("picture"));
        let (d, t, m) = analyze(&frames);
        let clips = d.detect(&t, &m);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].end, 1.5);
        assert!(clips[0].at_file_end);
    }
}
