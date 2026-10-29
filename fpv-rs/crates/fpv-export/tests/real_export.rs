//! Lossless cuts through the real ffmpeg, at the DVR's own odd rates: a cut
//! of N frames must hold exactly N frames. Rounding a duration up let the next
//! frame in — 1597 frames of PICT0031 at 24.486 fps came out 1598 (found by
//! the reduction's frame check, 2026-10-23).

use std::process::Command;

use fpv_core::{Clip, FrameRate, MediaInfo, SourceFile};
use fpv_export::ClipExporter;
use fpv_media::FfmpegRunner;
use uuid::Uuid;

fn frames(path: &std::path::Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v",
               "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
        .arg(path).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn cuts_exactly_the_frames_asked_at_the_dvrs_own_rate() {
    let folder = tempfile::tempdir().unwrap();
    let recording = folder.path().join("PICT0031.AVI");
    // PICT0031's rate, 1000000/40839 (24.486 fps): 1597 frames last 65.219883 s,
    // which rounds up to 65.2199 at four decimals.
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x120:rate=1000000/40839:duration=80",
               "-c:v", "mjpeg", "-q:v", "20", "-y"])
        .arg(&recording).status().unwrap();
    assert!(status.success());
    let rate = FrameRate::new(1_000_000, 40839);
    let file = SourceFile::new(Uuid::new_v4(), "card", recording.to_string_lossy(), Some(31),
                               MediaInfo::new(160, 120, rate, 80.0, "mjpeg"));
    let ffmpeg = FfmpegRunner::default();
    for (start, end) in [(0, 1597), (100, 1697), (300, 1897)] {
        let out = folder.path().join(format!("cut-{start}.avi"));
        ClipExporter::new(&ffmpeg)
            .export_clip(&Clip::new(Uuid::new_v4(), file.id, start, end), &file, &out)
            .unwrap();
        assert_eq!(frames(&out), (end - start).to_string(), "frames {start}..{end}");
    }
}

/// Signed 16-bit samples of a file's audio, from its start.
fn samples(path: &std::path::Path) -> Vec<u8> {
    Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path)
        .args(["-map", "0:a", "-f", "s16le", "-"]).output().unwrap().stdout
}

#[test]
fn keeps_the_sound_on_the_picture_when_cutting_mid_recording() {
    // The DVR's sound: PCM at 8 kHz, in packets of a tenth of a second or
    // more. Copied, a cut's sound started at the next packet but played from
    // the cut's first frame — 99 ms early on PICT0031 (measured, 2026-10-23).
    let folder = tempfile::tempdir().unwrap();
    let recording = folder.path().join("PICT0031.AVI");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x120:rate=1000000/40839:duration=110",
               "-f", "lavfi", "-i", "anoisesrc=r=8000:d=110:seed=7", "-c:v", "mjpeg", "-q:v", "20",
               "-c:a", "pcm_s16le", "-shortest", "-y"])
        .arg(&recording).status().unwrap();
    assert!(status.success());
    let rate = FrameRate::new(1_000_000, 40839);
    let media = MediaInfo { audio_codec: Some("pcm_s16le".into()), ..MediaInfo::new(160, 120, rate, 110.0, "mjpeg") };
    let file = SourceFile::new(Uuid::new_v4(), "card", recording.to_string_lossy(), Some(31), media);
    let out = folder.path().join("cut.avi");
    ClipExporter::new(&FfmpegRunner::default())
        .export_clip(&Clip::new(Uuid::new_v4(), file.id, 1721, 2436), &file, &out)
        .unwrap();
    assert_eq!(frames(&out), "715");

    // Where the cut's sound sits in the recording's: on the cut's first frame,
    // the very same samples (noise, so a match is unambiguous).
    let (cut, whole) = (samples(&out), samples(&recording));
    let at = whole.windows(4000).position(|w| w == &cut[..4000]).expect("the cut's samples are the recording's");
    let expected = (rate.seconds(1721) * 8000.0).round() as i64;
    assert!(((at / 2) as i64 - expected).abs() <= 1, "sound starts at sample {}, picture at {expected}", at / 2);
    assert_eq!(&whole[at..at + cut.len()], &cut[..]);
}

#[test]
fn mixed_recording_mute_preserves_sound_in_other_parts_and_fills_only_muted_parts_with_silence() {
    let folder = tempfile::tempdir().unwrap();
    let recording = folder.path().join("audio.avi");
    assert!(Command::new("ffmpeg").args(["-v", "error", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=25:duration=1", "-f", "lavfi", "-i", "sine=duration=1:sample_rate=48000",
        "-c:v", "mjpeg", "-c:a", "pcm_s16le", "-shortest", "-y"]).arg(&recording).status().unwrap().success());
    let mut media = MediaInfo::new(64, 48, FrameRate::new(25, 1), 1.0, "mjpeg");
    media.audio_codec = Some("pcm_s16le".into());
    let file = SourceFile::new(Uuid::new_v4(), "card", recording.to_string_lossy(), None, media);
    let original = std::fs::read(&recording).unwrap();
    let runner = FfmpegRunner::default();
    let exporter = ClipExporter::new(&runner);
    for mute_first in [true, false] {
        let mut first = file.clone(); first.muted = mute_first;
        let mut second = file.clone(); second.id = Uuid::new_v4(); second.muted = !mute_first;
        let parts = vec![(Clip::new(Uuid::new_v4(), first.id, 0, 25), first),
            (Clip::new(Uuid::new_v4(), second.id, 0, 25), second)];
        for reencode in [false, true] {
            let out = folder.path().join(format!("mixed-{mute_first}-{reencode}.avi"));
            if reencode { exporter.export_flight_reencoded(&parts, &out, None).unwrap(); }
            else { exporter.export_flight(&parts, &out).unwrap(); }
            assert_eq!(frames(&out), "50");
            let bytes = samples(&out);
            let audio: Vec<_> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
            assert!(audio.len() >= 48000 * 2 * 2);
            let (start, end) = if mute_first { (0, 48000 * 2) } else { (48000 * 2, 48000 * 2 * 2) };
            assert!(audio[start..end].iter().all(|s| *s == 0), "the muted chapter leaked audio");
            let audible = if mute_first { &audio[end..] } else { &audio[..start] };
            assert!(audible.iter().any(|s| s.abs() > 100), "the unmuted chapter lost its audio");
        }
    }
    assert_eq!(std::fs::read(recording).unwrap(), original);
}

#[test]
fn failed_container_export_leaves_no_empty_file_and_can_be_retried() {
    let folder = tempfile::tempdir().unwrap();
    let recording = folder.path().join("source.avi");
    assert!(Command::new("ffmpeg").args(["-v", "error", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=25:duration=1", "-f", "lavfi", "-i",
        "sine=duration=1:sample_rate=8000", "-c:v", "mjpeg", "-c:a", "pcm_s16le",
        "-shortest", "-y"]).arg(&recording).status().unwrap().success());
    let media = MediaInfo { audio_codec: Some("pcm_s16le".into()),
        ..MediaInfo::new(64, 48, FrameRate::new(25, 1), 1.0, "mjpeg") };
    let file = SourceFile::new(Uuid::new_v4(), "card", recording.to_string_lossy(), None, media);
    let original = std::fs::read(&recording).unwrap();
    let clip = Clip::new(Uuid::new_v4(), file.id, 0, 25);
    // PCM cannot be muxed in this container: ffmpeg creates an empty file
    // before returning the header error. The app must not leave it behind.
    let destination = folder.path().join("flight.webm");
    let runner = FfmpegRunner::default();
    let exporter = ClipExporter::new(&runner);
    assert!(exporter.export_clip(&clip, &file, &destination).is_err());
    assert!(!destination.exists(), "failed export left an empty final file");
    let destination = folder.path().join("flight.avi");
    exporter.export_clip(&clip, &file, &destination).unwrap();
    assert_eq!(frames(&destination), "25");
    assert!(exporter.export_clip(&clip, &file, &destination).is_err(), "never overwrite");
    assert_eq!(frames(&destination), "25");
    let past_end = Clip::new(Uuid::new_v4(), file.id, 250, 275);
    let empty = folder.path().join("empty.avi");
    assert!(exporter.export_clip(&past_end, &file, &empty).is_err(), "header-only video is not a success");
    assert!(!empty.exists());
    assert_eq!(std::fs::read(recording).unwrap(), original);
}

#[test]
fn h264_avi_exports_to_mp4_with_video_unchanged_and_incompatible_audio_converted() {
    let folder = tempfile::tempdir().unwrap();
    let recording = folder.path().join("EV300D.AVI");
    assert!(Command::new("ffmpeg").args(["-v", "error", "-f", "lavfi", "-i",
        "testsrc2=size=64x48:rate=25:duration=2", "-f", "lavfi", "-i",
        "sine=duration=2:sample_rate=8000", "-c:v", "libx264", "-pix_fmt", "yuv420p",
        "-c:a", "adpcm_ima_wav", "-shortest", "-y"]).arg(&recording).status().unwrap().success());
    let file = SourceFile::new(Uuid::new_v4(), "card", recording.to_string_lossy(), None,
        MediaInfo { audio_codec: Some("adpcm_ima_wav".into()),
            ..MediaInfo::new(64, 48, FrameRate::new(25, 1), 2.0, "h264") });
    let original = std::fs::read(&recording).unwrap();
    let clip = Clip::new(Uuid::new_v4(), file.id, 0, 50);
    let runner = FfmpegRunner::default();
    let exporter = ClipExporter::new(&runner);
    for muted in [false, true] {
        let destination = folder.path().join(format!("export-{muted}.mp4"));
        exporter.export_clip_with_mute(&clip, &file, &destination, muted).unwrap();
        assert_eq!(frames(&destination), "50");
        let audio = Command::new("ffprobe").args(["-v", "error", "-select_streams", "a:0",
            "-show_entries", "stream=codec_name", "-of", "csv=p=0"]).arg(&destination).output().unwrap();
        assert!(audio.status.success());
        assert_eq!(String::from_utf8_lossy(&audio.stdout).trim(), if muted { "" } else { "aac" });
        let video_hashes = |path: &std::path::Path| {
            let hash = |flag, mode| Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path)
                .args(["-map", "0:v:0", flag, mode, "-f", "framemd5", "-"]).output().unwrap();
            let mut output = hash("-fps_mode", "passthrough");
            // Match the proxy's compatibility path: FFmpeg 4 predates fps_mode,
            // while FFmpeg 9 removed vsync. Never resample frames for comparison.
            if !output.status.success() && String::from_utf8_lossy(&output.stderr)
                .contains("Unrecognized option 'fps_mode'") {
                output = hash("-vsync", "0");
            }
            assert!(output.status.success(), "frame hash failed for {}: {}", path.display(), String::from_utf8_lossy(&output.stderr));
            String::from_utf8_lossy(&output.stdout).lines().filter(|line| !line.starts_with('#'))
                .map(|line| line.rsplit(',').next().unwrap().trim().to_owned()).collect::<Vec<_>>()
        };
        assert_eq!(video_hashes(&destination), video_hashes(&recording));
    }
    assert_eq!(std::fs::read(recording).unwrap(), original);
}
