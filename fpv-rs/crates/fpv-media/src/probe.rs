use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use fpv_core::{FrameRate, MediaInfo};
use serde::Deserialize;

use crate::{FfmpegError, FfmpegInvoking, FfmpegTool};

/// Reads one recording's real parameters. Nothing about this format may be
/// assumed: the frame rate differs in every file, and there is no creation date.
pub struct MediaProbe<'a> {
    ffmpeg: &'a dyn FfmpegInvoking,
}

#[derive(Deserialize)]
struct Output {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
    tags: Option<HashMap<String, String>>,
}

impl<'a> MediaProbe<'a> {
    pub fn new(ffmpeg: &'a dyn FfmpegInvoking) -> Self {
        Self { ffmpeg }
    }

    pub fn probe(&self, path: &Path) -> Result<MediaInfo, FfmpegError> {
        let arguments: Vec<String> = ["-v", "error", "-print_format", "json",
                                      "-show_streams", "-show_format"]
            .into_iter().map(String::from)
            .chain([path.to_string_lossy().into_owned()])
            .collect();
        let result = self.ffmpeg.run(FfmpegTool::Ffprobe, &arguments)?;
        if !result.succeeded() {
            return Err(FfmpegError::tool_failed(FfmpegTool::Ffprobe, &arguments, &result));
        }
        Self::parse(&result.stdout)
    }

    pub fn parse(json: &[u8]) -> Result<MediaInfo, FfmpegError> {
        let unreadable = |detail: String| FfmpegError::UnreadableOutput {
            tool: "ffprobe".into(), detail,
        };
        let output: Output = serde_json::from_slice(json).map_err(|e| unreadable(e.to_string()))?;
        let video = output.streams.iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
            .filter(|s| s.width.is_some() && s.height.is_some())
            .ok_or_else(|| unreadable("no video stream".into()))?;
        let rate = video.r_frame_rate.as_deref()
            .and_then(FrameRate::from_ffprobe)
            .ok_or_else(|| unreadable(format!(
                "unusable r_frame_rate {}", video.r_frame_rate.as_deref().unwrap_or("nil"))))?;
        let audio = output.streams.iter().find(|s| s.codec_type.as_deref() == Some("audio"));
        Ok(MediaInfo {
            width: video.width.unwrap_or_default(),
            height: video.height.unwrap_or_default(),
            frame_rate: rate,
            frame_count: video.nb_frames.as_deref().and_then(|n| n.parse().ok()),
            duration: output.format.duration.as_deref()
                .and_then(|d| d.parse().ok()).unwrap_or(0.0),
            video_codec: video.codec_name.clone().unwrap_or_else(|| "unknown".into()),
            audio_codec: audio.and_then(|a| a.codec_name.clone()),
            creation_date: output.format.tags.as_ref()
                .and_then(|tags| tags.get("creation_time"))
                // FFmpeg 9 combines duplicate MP4 header/mdta timestamps
                // into one semicolon-separated metadata value.
                .and_then(|text| text.split(';')
                    .find_map(|stamp| DateTime::parse_from_rfc3339(stamp.trim()).ok()))
                .map(|date| date.with_timezone(&Utc)),
            camera: output.format.tags.as_ref().and_then(camera_name),
        })
    }
}

/// The camera's make and model from a container's tags, where the camera
/// wrote any — the DVR's AVI never does. Tried in order: the QuickTime
/// make/model keys (Apple's `mdta`, the `©mak`/`©mod` atoms); GoPro's
/// firmware string, whose prefix names the generation (`H22.01.01.10.00` is a
/// HERO11); an `encoder` that is a camera's name (`DJI…`, `Insta360…`) rather
/// than a muxer's (`Lavf…`). Anything else is `None`, and the brand then comes
/// from the file name.
pub fn camera_name(tags: &HashMap<String, String>) -> Option<String> {
    let tag = |key: &str| tags.iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty());
    let make = tag("com.apple.quicktime.make").or_else(|| tag("make"));
    if let Some(model) = tag("com.apple.quicktime.model").or_else(|| tag("model")) {
        return Some(match make {
            Some(make) if !model.to_lowercase().starts_with(&make.to_lowercase()) =>
                format!("{make} {model}"),
            _ => model.to_string(),
        });
    }
    if let Some(model) = tag("firmware").and_then(gopro_generation) {
        return Some(model);
    }
    let encoder = tag("encoder")?;
    let named = encoder.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');
    for brand in ["DJI", "Insta360"] {
        if let Some(model) = encoder.strip_prefix(brand).map(str::trim_start)
            && named && !model.is_empty()
        {
            return Some(format!("{brand} {model}"));
        }
    }
    None
}

/// GoPro's firmware strings start with the camera's code: `HD5`…`HD9` for the
/// HERO5 to HERO9, then `H21` for the HERO10 and one a year since.
fn gopro_generation(firmware: &str) -> Option<String> {
    let code = firmware.split('.').next()?;
    let generation = match code {
        "HD4" => 4, "HD5" => 5, "HD6" => 6, "HD7" => 7, "HD8" => 8, "HD9" => 9,
        "H21" => 10, "H22" => 11, "H23" => 12, "H24" => 13,
        _ => return None,
    };
    Some(format!("GoPro HERO{generation}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessResult;

    /// Stands in for the bundled binaries so the parsing can be tested without
    /// media. This is the boundary CLAUDE.md 3.1 exists to make mockable.
    struct StubFfmpeg<'a> {
        status: i32,
        output: &'a str,
        error: &'static str,
    }

    impl FfmpegInvoking for StubFfmpeg<'_> {
        fn run(&self, _: FfmpegTool, _: &[String]) -> Result<ProcessResult, FfmpegError> {
            Ok(ProcessResult {
                status: self.status, stdout: self.output.as_bytes().to_vec(),
                stderr: self.error.into(),
            })
        }
    }

    /// Verbatim ffprobe output for PICT0030.AVI, the first file of the reference set.
    const PICT0030: &str = r#"
    {
      "streams": [
        { "codec_name": "mjpeg", "codec_type": "video", "width": 640, "height": 480,
          "r_frame_rate": "1000000/33459", "nb_frames": "4217" },
        { "codec_name": "pcm_s16le", "codec_type": "audio", "sample_rate": "8000",
          "r_frame_rate": "0/0", "nb_frames": "1128794" }
      ],
      "format": { "format_name": "avi", "duration": "141.096603" }
    }"#;

    fn probe(output: &str) -> Result<MediaInfo, FfmpegError> {
        let stub = StubFfmpeg { status: 0, output, error: "" };
        MediaProbe::new(&stub).probe(Path::new("/cards/PICT0030.AVI"))
    }

    #[test]
    fn reads_the_real_parameters_of_an_analog_dvr_file() {
        let info = probe(PICT0030).unwrap();
        assert_eq!((info.width, info.height), (640, 480));
        assert_eq!(info.video_codec, "mjpeg");
        assert_eq!(info.frame_count, Some(4217));
        assert!((info.duration - 141.096603).abs() < 0.0001);
        // The audio track is present on this hardware.
        assert_eq!(info.audio_codec.as_deref(), Some("pcm_s16le"));
        // No creation date anywhere in an AVI from this DVR.
        assert_eq!(info.creation_date, None);
    }

    #[test]
    fn keeps_the_frame_rate_as_a_rational_rather_than_rounding_it() {
        let info = probe(PICT0030).unwrap();
        assert_eq!(info.frame_rate, FrameRate::new(1_000_000, 33459));
        assert!((info.frame_rate.value() - 29.887).abs() < 0.001);
    }

    #[test]
    fn reports_the_argument_vector_when_ffprobe_fails() {
        let stub = StubFfmpeg { status: 1, output: "", error: "Invalid data found" };
        let error = MediaProbe::new(&stub).probe(Path::new("/cards/broken.AVI")).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Invalid data found"), "{text}");
        assert!(text.contains("ffprobe -v error -print_format json"), "{text}");
        assert!(text.contains("/cards/broken.AVI"), "{text}");
    }

    #[test]
    fn rejects_output_with_no_usable_frame_rate() {
        let json = r#"{ "streams": [ { "codec_name": "mjpeg", "codec_type": "video",
            "width": 640, "height": 480, "r_frame_rate": "0/0" } ],
            "format": { "duration": "1.0" } }"#;
        assert!(matches!(probe(json), Err(FfmpegError::UnreadableOutput { .. })));
    }

    #[test]
    fn reads_a_creation_date_where_a_container_has_one() {
        let json = r#"{ "streams": [ { "codec_name": "h264", "codec_type": "video",
            "width": 1920, "height": 1080, "r_frame_rate": "60/1" } ],
            "format": { "duration": "1.0",
                        "tags": { "creation_time": "2026-08-23T10:15:00.000000Z" } } }"#;
        let date = probe(json).unwrap().creation_date.unwrap();
        assert_eq!(date.to_rfc3339(), "2026-08-23T10:15:00+00:00");
        let duplicated = json.replace("2026-08-23T10:15:00.000000Z",
            "2026-08-23T10:15:00.000000Z;2026-08-23T10:15:00Z");
        assert_eq!(probe(&duplicated).unwrap().creation_date, Some(date));
    }

    fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn names_the_camera_from_its_make_and_model_tags() {
        assert_eq!(camera_name(&tags(&[("com.apple.quicktime.make", "Insta360"),
                                       ("com.apple.quicktime.model", "X3")])).as_deref(),
                   Some("Insta360 X3"));
        // A model that already carries the make is not prefixed twice.
        assert_eq!(camera_name(&tags(&[("make", "DJI"), ("model", "DJI Osmo Action 4")])).as_deref(),
                   Some("DJI Osmo Action 4"));
    }

    #[test]
    fn names_a_gopro_from_its_firmware_code() {
        assert_eq!(camera_name(&tags(&[("firmware", "H22.01.01.10.00")])).as_deref(),
                   Some("GoPro HERO11"));
        assert_eq!(camera_name(&tags(&[("firmware", "HD9.01.01.72.00")])).as_deref(),
                   Some("GoPro HERO9"));
        // A code nobody has mapped yet: the file name's brand will do.
        assert_eq!(camera_name(&tags(&[("firmware", "H99.01.00")])), None);
    }

    #[test]
    fn takes_an_encoder_for_a_camera_only_when_it_names_one() {
        assert_eq!(camera_name(&tags(&[("encoder", "DJI OsmoAction4")])).as_deref(),
                   Some("DJI OsmoAction4"));
        assert_eq!(camera_name(&tags(&[("encoder", "DJIMini3Pro")])).as_deref(),
                   Some("DJI Mini3Pro"));
        // A muxer, a handler-like string, a bare brand: not a camera.
        for encoder in ["Lavf61.7.100", "DJI.AVC", "DJI", "  "] {
            assert_eq!(camera_name(&tags(&[("encoder", encoder)])), None, "{encoder}");
        }
    }

    /// The pilot's DJI Osmo Nano take (2026-10-17, Alpes), ffprobe's output cut
    /// to the fields read: the camera names itself in `encoder`, and the file
    /// carries a 1280x720 MJPEG cover picture after the HEVC.
    const OSMO_NANO: &str = r#"
    {
      "streams": [
        { "codec_name": "hevc", "codec_type": "video", "width": 3840, "height": 2160,
          "r_frame_rate": "60000/1001", "nb_frames": "26457" },
        { "codec_name": "aac", "codec_type": "audio", "r_frame_rate": "0/0" },
        { "codec_type": "data", "r_frame_rate": "0/0" },
        { "codec_type": "data", "r_frame_rate": "0/0" },
        { "codec_name": "mjpeg", "codec_type": "video", "width": 1280, "height": 720,
          "r_frame_rate": "90000/1", "disposition": { "attached_pic": 1 } }
      ],
      "format": { "duration": "441.390950",
                  "tags": { "major_brand": "isom", "creation_time": "2026-10-17T09:22:04.000000Z",
                            "encoder": "DJI Osmo Nano" } }
    }"#;

    #[test]
    fn names_the_pilots_osmo_nano_and_reads_its_hevc_not_its_cover_picture() {
        let info = probe(OSMO_NANO).unwrap();
        assert_eq!(info.camera.as_deref(), Some("DJI Osmo Nano"));
        assert_eq!((info.video_codec.as_str(), info.width), ("hevc", 3840));
        assert!(!info.needs_segmentation(), "its MJPEG cover is not a DVR recording");
    }

    #[test]
    fn reads_the_camera_through_a_probe_and_none_from_the_dvr() {
        assert_eq!(probe(PICT0030).unwrap().camera, None);
        let json = r#"{ "streams": [ { "codec_name": "h264", "codec_type": "video",
            "width": 1920, "height": 1080, "r_frame_rate": "60/1" } ],
            "format": { "duration": "1.0", "tags": { "firmware": "H21.01.01.62.00" } } }"#;
        assert_eq!(probe(json).unwrap().camera.as_deref(), Some("GoPro HERO10"));
    }
}
