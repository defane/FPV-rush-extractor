use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::FrameRate;

/// What ffprobe can tell us about one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub width: i64,
    pub height: i64,
    pub frame_rate: FrameRate,
    pub frame_count: Option<i64>,
    /// Seconds.
    pub duration: f64,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    /// `None` for the analog DVR: the AVIs carry no creation date, and the file
    /// mtime is meaningless because the goggles have no clock.
    pub creation_date: Option<DateTime<Utc>>,
    /// The camera's make and model where the file names it ("GoPro HERO11
    /// Black"); `None` for the DVR and for cameras that write nothing usable.
    #[serde(default)]
    pub camera: Option<String>,
}

impl MediaInfo {
    /// The fields every recording has; the optional ones start empty.
    pub fn new(width: i64, height: i64, frame_rate: FrameRate, duration: f64,
               video_codec: impl Into<String>) -> Self {
        Self {
            width, height, frame_rate, frame_count: None, duration,
            video_codec: video_codec.into(), audio_codec: None, creation_date: None,
            camera: None,
        }
    }

    /// All-intra codecs can be cut frame-accurately with `ffmpeg -c copy`. MJPEG
    /// is; H.264 from the digital systems is not, and would need its leading
    /// partial GOP re-encoded or the cut snapped to a GOP boundary.
    pub fn is_all_intra(&self) -> bool {
        self.video_codec == "mjpeg"
    }

    /// Every imported recording can contain unusable video, regardless of codec.
    /// Eligibility is independent of source labels and lossless-cut capability.
    pub fn needs_segmentation(&self) -> bool {
        true
    }

    /// Legacy source-label hint, never a gate for running signal detection.
    pub fn is_analog_dvr(&self) -> bool {
        self.video_codec == "mjpeg"
    }

    /// The OSD grid: MAX7456 is 30 columns by 13 rows on NTSC, 16 on PAL.
    /// Detected from the frame height rather than hardcoded (CLAUDE.md 9.2).
    pub fn osd_rows(&self) -> i64 {
        if self.height > 512 { 16 } else { 13 }
    }

    pub fn osd_columns(&self) -> i64 {
        30
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_osd_grid_from_the_frame_height_not_a_constant() {
        let ntsc = MediaInfo::new(640, 480, FrameRate::new(30, 1), 1.0, "mjpeg");
        let pal = MediaInfo::new(720, 576, FrameRate::new(25, 1), 1.0, "mjpeg");
        assert_eq!(ntsc.osd_rows(), 13);
        assert_eq!(pal.osd_rows(), 16);
        assert!(ntsc.is_all_intra());
    }

    #[test]
    fn all_imported_video_codecs_are_eligible_for_signal_segmentation() {
        for codec in ["mjpeg", "h264", "hevc", "mpeg4", "prores"] {
            let recording = MediaInfo::new(720, 576, FrameRate::new(30, 1), 600.0, codec);
            assert!(recording.needs_segmentation(), "{codec} must reach the detector");
        }
    }

    #[test]
    fn segmentation_eligibility_is_independent_of_lossless_cut_capability() {
        let dvr = MediaInfo::new(640, 480, FrameRate::new(1_000_000, 40_061), 372.7, "mjpeg");
        assert!(dvr.needs_segmentation() && dvr.is_all_intra());
        // H.264/HEVC can contain recorded snow too, but precise cuts need
        // different export handling than the all-intra DVR codec.
        for codec in ["h264", "hevc"] {
            let take = MediaInfo::new(3840, 2160, FrameRate::new(60_000, 1001), 30.0, codec);
            assert!(take.needs_segmentation(), "{codec}");
            // Long-GOP: cut only from frame 0, never mid-stream.
            assert!(!take.is_all_intra(), "{codec}");
        }
    }
}
