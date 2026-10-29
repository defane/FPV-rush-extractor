use std::sync::Mutex;

use fpv_core::MediaInfo;
use fpv_media::{FfmpegTool, ProcessResult};
use uuid::Uuid;

use super::*;

#[derive(Default)]
struct RecordingFfmpeg {
    calls: Mutex<Vec<Vec<String>>>,
}

impl RecordingFfmpeg {
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl FfmpegInvoking for RecordingFfmpeg {
    fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
        if tool == FfmpegTool::Ffprobe {
            return Ok(ProcessResult { status: 0, stdout: b"1\n".to_vec(), stderr: String::new() });
        }
        self.calls.lock().unwrap().push(arguments.to_vec());
        if let Some(index) = arguments.iter().rposition(|a| a == "-y") {
            std::fs::write(&arguments[index + 1], b"v").unwrap();
        }
        Ok(ProcessResult { status: 0, stdout: vec![], stderr: String::new() })
    }
}

fn source(name: &str, rate: FrameRate, codec: &str) -> SourceFile {
    SourceFile::new(Uuid::new_v4(), format!("/card/{name}"), format!("/library/{name}"), Some(30),
                    MediaInfo::new(640, 480, rate, 600.0, codec))
}

fn part(file: &SourceFile, start: i64, end: i64) -> Part {
    (Clip::new(Uuid::new_v4(), file.id, start, end), file.clone())
}

fn value(arguments: &[String], flag: &str) -> f64 {
    let index = arguments.iter().position(|a| a == flag).unwrap();
    arguments[index + 1].parse().unwrap()
}

/// PICT0030 at 29.887 fps into PICT0031 at 24.486: 22% apart.
fn mismatched_pair() -> Vec<Part> {
    let first = source("PICT0030.AVI", FrameRate::new(1_000_000, 33459), "mjpeg");
    let second = source("PICT0031.AVI", FrameRate::new(1_000_000, 40839), "mjpeg");
    vec![part(&first, 4194, 4217), part(&second, 0, 1475)]
}

#[test]
fn cuts_losslessly_at_the_frames_own_timestamps() {
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    let rate = FrameRate::new(250_000, 10009); // 24.978 fps
    let file = source("PICT0032.AVI", rate, "mjpeg");
    let (clip, _) = part(&file, 8649, 11079);
    ClipExporter::new(&ffmpeg).export_clip(&clip, &file, &out.path().join("flight.avi")).unwrap();

    let arguments = &ffmpeg.calls()[0];
    assert!(arguments.windows(2).any(|w| w == ["-c", "copy"]));
    // Output-side seek: input-side lands early on these AVIs.
    let seek = arguments.iter().position(|a| a == "-ss").unwrap();
    let input = arguments.iter().position(|a| a == "-i").unwrap();
    assert!(input < seek);
    assert!((value(arguments, "-ss") - rate.seconds(8649)).abs() < 0.001);
    assert!((value(arguments, "-t") - 97.29).abs() < 0.01);
}

#[test]
fn muted_copy_export_keeps_video_and_drops_audio() {
    let rate = FrameRate::new(250_000, 10009);
    let mut file = source("PICT0032.AVI", rate, "mjpeg");
    file.media.audio_codec = Some("pcm_s16le".into());
    let (clip, _) = part(&file, 100, 500);
    let arguments = copy_arguments(&clip, &file, true).unwrap();
    assert!(arguments.windows(2).any(|w| w == ["-c:v", "copy"]));
    assert!(arguments.iter().any(|arg| arg == "-an"));
    assert!(!arguments.iter().any(|arg| arg == "-c:a" || arg == "-c"));
}

#[test]
fn writes_the_dvrs_pcm_sound_again_so_it_is_cut_on_the_frame() {
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    let mut file = source("PICT0031.AVI", FrameRate::new(1_000_000, 40839), "mjpeg");
    file.media.audio_codec = Some("pcm_s16le".into());
    let (clip, _) = part(&file, 1721, 2436);
    ClipExporter::new(&ffmpeg).export_clip(&clip, &file, &out.path().join("flight.avi")).unwrap();
    let arguments = &ffmpeg.calls()[0];
    assert!(arguments.windows(2).any(|w| w == ["-c:v", "copy"]));
    assert!(arguments.windows(2).any(|w| w == ["-c:a", "pcm_s16le"]));
    assert!(!arguments.windows(2).any(|w| w == ["-c", "copy"]));
}

#[test]
fn refuses_to_copy_a_long_gop_codec_from_the_middle_of_a_file() {
    let file = source("GH010042.MP4", FrameRate::new(60, 1), "h264");
    let (clip, _) = part(&file, 100, 200);
    let result = ClipExporter::new(&RecordingFfmpeg::default())
        .export_clip(&clip, &file, Path::new("/tmp/never.mp4"));
    assert!(matches!(result, Err(ExportError::NotAllIntra(codec)) if codec == "h264"));
}

#[test]
fn copies_a_whole_action_cam_take_across_its_chapters() {
    // Both chapters open on frame 0, a keyframe: a stream copy is exact.
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    let rate = FrameRate::new(60000, 1001);
    let parts = [part(&source("GH010042.MP4", rate, "h264"), 0, 28_000),
                 part(&source("GH020042.MP4", rate, "h264"), 0, 9_000)];
    ClipExporter::new(&ffmpeg).export_flight(&parts, &out.path().join("take.mp4")).unwrap();
    let calls = ffmpeg.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c.contains(&"copy".to_owned())));
    assert!(calls[0].last().unwrap().ends_with("part000.mp4"));
    // No -ss at all: on H.264 an output-side seek drops packets even at 0.
    assert!(calls[..2].iter().all(|c| !c.contains(&"-ss".to_owned())));
    assert!((value(&calls[0], "-t") - rate.seconds(28_000)).abs() < 0.001);
}

#[test]
fn concatenates_a_flight_split_across_two_recordings() {
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    let first = source("PICT0038.AVI", FrameRate::new(25000, 839), "mjpeg");
    let second = source("PICT0039.AVI", FrameRate::new(1_000_000, 33661), "mjpeg");
    ClipExporter::new(&ffmpeg)
        .export_flight(&[part(&first, 12000, 17926), part(&second, 0, 6000)],
                       &out.path().join("flight.avi"))
        .unwrap();
    // Two cuts and one concat, all stream copies, and no scratch left behind.
    let calls = ffmpeg.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls[2].contains(&"concat".to_owned()) && calls[2].contains(&"copy".to_owned()));
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
}

#[test]
fn refuses_to_concatenate_recordings_made_at_different_rates() {
    let result = ClipExporter::new(&RecordingFfmpeg::default())
        .export_flight(&mismatched_pair(), Path::new("/tmp/never.avi"));
    assert!(matches!(result, Err(ExportError::MismatchedFrameRates(..))));
}

#[test]
fn reencodes_a_mismatched_flight_when_asked_to() {
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    ClipExporter::new(&ffmpeg)
        .export_flight_reencoded(&mismatched_pair(), &out.path().join("flight.avi"), None)
        .unwrap();
    // Each part resampled on its own, then a stream-copy join.
    let calls = ffmpeg.calls();
    assert_eq!(calls.len(), 3);
    for cut in &calls[..2] {
        assert!(cut.contains(&"mjpeg".to_owned()));
        assert!(cut.windows(2).any(|w| w == ["-c:a", "copy"]));
        // The 0.77 s fragment bends to the 60 s continuation, never the reverse.
        assert!(cut.windows(2).any(|w| w == ["-vf", "fps=1000000/40839"]));
    }
    assert!(calls[2].contains(&"concat".to_owned()) && !calls[2].contains(&"mjpeg".to_owned()));
}

#[test]
fn reencoding_a_single_part_flight_stays_lossless() {
    let ffmpeg = RecordingFfmpeg::default();
    let out = tempfile::tempdir().unwrap();
    let file = source("PICT0034.AVI", FrameRate::new(3125, 126), "mjpeg");
    ClipExporter::new(&ffmpeg)
        .export_flight_reencoded(&[part(&file, 0, 1000)], &out.path().join("f.avi"), None)
        .unwrap();
    let calls = ffmpeg.calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].contains(&"mjpeg".to_owned()) && calls[0].contains(&"copy".to_owned()));
}

#[test]
fn imported_recording_mute_is_honored_without_a_global_mute_setting() {
    let ffmpeg = RecordingFfmpeg::default();
    let mut file = source("PICT0030.AVI", FrameRate::new(25, 1), "mjpeg");
    file.media.audio_codec = Some("pcm_s16le".into());
    file.muted = true;
    let out = tempfile::tempdir().unwrap();
    ClipExporter::new(&ffmpeg).export_clip(&part(&file, 0, 25).0, &file, &out.path().join("muted.avi")).unwrap();
    assert!(ffmpeg.calls()[0].contains(&"-an".to_owned()));
}

#[test]
fn successful_process_cannot_publish_empty_or_packetless_video() {
    struct EmptyExport { bytes: &'static [u8] }
    impl FfmpegInvoking for EmptyExport {
        fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
            if tool == FfmpegTool::Ffmpeg {
                std::fs::write(arguments.last().unwrap(), self.bytes).unwrap();
            }
            Ok(ProcessResult { status: 0, stdout: b"0\n".to_vec(), stderr: String::new() })
        }
    }
    for bytes in [b"".as_slice(), b"container header".as_slice()] {
        let out = tempfile::tempdir().unwrap();
        let destination = out.path().join("flight.avi");
        let file = source("PICT0030.AVI", FrameRate::new(25, 1), "mjpeg");
        let result = ClipExporter::new(&EmptyExport { bytes }).export_clip(
            &part(&file, 0, 25).0, &file, &destination);
        assert!(result.is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0, "scratch removed");
    }
}
