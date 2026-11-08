//! Segmentation: one clip per video signal onset (CLAUDE.md 4.0). Reproduces
//! `spike/fpv_spike.py` exactly, as the Swift port before it did. That
//! equality is the acceptance test: the same 26 clips on the twelve reference
//! files, and the fixture metrics to 1e-4 of the Python ones.
//!
//! Floating-point parity depends on summation order, so every sum here walks
//! its values in the order the parity was established in. Do not "optimise" a
//! loop into a different traversal without re-running the parity checks.

mod detector;
mod file_detector;
mod metrics;
mod snow_split;
pub use snow_split::{SnowSplitEvidence, VERSION as SNOW_SPLIT_VERSION};

pub use detector::{ClipDetector, DetectedClip, DetectorSettings, FrameState};
pub use file_detector::{ClipDetecting, ClipStitcher, DetectError, DetectionResult,
                        FileClips, FileDetector};
pub use metrics::{FrameAnalyzer, FrameMetrics};
