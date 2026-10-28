use std::path::{Path, PathBuf};

use fpv_core::{Clip, FrameRate, SourceFile};
use fpv_media::{FfmpegError, FfmpegInvoking, Workspace, run_writing, seconds_argument,
                write_concat_list};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("no clips to build a proxy from")]
    NothingToGenerate,
    #[error(transparent)]
    Ffmpeg(#[from] FfmpegError),
    #[error("could not prepare the proxy: {0}")]
    Io(#[from] std::io::Error),
}

/// Taller than this, a source is scaled down for playback, and decoded by the
/// hardware where there is one. 720 lines, not 1080: measured on a 4K 10-bit
/// HEVC 59.94 fps take, 1080p60 took 14.5 s for 20 s of footage, 720p30 9.7 s,
/// and decoding alone 7.8 s. The DVR's 480 lines are never touched.
const MAX_PROXY_HEIGHT: i64 = 720;

/// Faster than this, a proxy plays at half the rate (or a quarter…): 30 fps is
/// enough to watch a flight and mark a figure — a tag goes back to the
/// recording's own frames through seconds, never through the proxy's — and
/// halves the frames to encode. The DVR records 24–30 fps and is unaffected.
const MAX_PROXY_RATE: f64 = 30.5;

/// The hardware decoder for sources taller than the proxy. Named on the Mac:
/// on the pilot's Osmo 4K 10-bit HEVC, 30 s of footage decoded in 78 s in
/// software and 10 s with VideoToolbox (measured on their Mac, 2026-10-20) —
/// too much to leave to `auto`'s choice. Elsewhere ffmpeg picks (VA-API,
/// CUDA…) or decodes in software when it finds none.
const HWACCEL: &str = if cfg!(target_os = "macos") { "videotoolbox" } else { "auto" };

fn proxy_rate(mut rate: FrameRate) -> FrameRate {
    while rate.value() > MAX_PROXY_RATE {
        rate = if rate.numerator % 2 == 0 { FrameRate::new(rate.numerator / 2, rate.denominator) }
               else { FrameRate::new(rate.numerator, rate.denominator * 2) };
    }
    rate
}

/// Optional headless playback derivative. The native GUI reads source files
/// directly. MPEG-4/AAC uses only built-in LGPL FFmpeg encoders.
pub struct ProxyGenerator<'a> {
    ffmpeg: &'a dyn FfmpegInvoking,
}

#[derive(Clone, Copy, PartialEq)]
enum Audio {
    Aac,
    Pcm,
}

impl<'a> ProxyGenerator<'a> {
    pub fn new(ffmpeg: &'a dyn FfmpegInvoking) -> Self {
        Self { ffmpeg }
    }

    /// Writes the flight's proxy to `destination` (an `.mp4`).
    ///
    /// Each part is cut and resampled on its own to the flight's dominant rate,
    /// then the homogeneous pieces are joined with a video copy — the same order
    /// as the re-encoding exporter, for the same measured reason: the concat
    /// demuxer times AVI packets at the first file's rate, and resampling after
    /// the join squeezed 60 s of PICT0031 into 49.
    ///
    /// The audio is encoded to AAC once, at the join, never per part. AAC
    /// opens every stream with a frame of encoder priming that MP4 hides behind
    /// an edit list, and the concat demuxer drops the second part's: joining
    /// per-part AAC delayed PICT0031's sound by 120 ms (measured). So the pieces
    /// carry PCM in a `.mov`, which also keeps the video timestamps exact — a
    /// `.mkv` rounds them to the millisecond and the joined rate drifts.
    pub fn generate(&self, parts: &[(Clip, SourceFile)], destination: &Path)
        -> Result<PathBuf, ProxyError> {
        if parts.is_empty() {
            return Err(ProxyError::NothingToGenerate);
        }
        let rate = proxy_rate(FrameRate::dominant(parts));
        let height = parts.iter().map(|(_, f)| f.media.height).max().unwrap_or(0);
        if let [part] = parts {
            return self.transcode(part, rate, height, Audio::Aac, destination);
        }

        let workspace = Workspace::beside(destination, "fpv-proxy")?;
        let mut pieces = vec![];
        for (index, part) in parts.iter().enumerate() {
            let piece = workspace.path().join(format!("part{index:03}.mov"));
            self.transcode(part, rate, height, Audio::Pcm, &piece)?;
            pieces.push(piece);
        }
        let list = workspace.path().join("parts.txt");
        write_concat_list(&pieces, &list)?;
        let mut arguments: Vec<String> = [
            "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i",
        ].map(String::from).to_vec();
        arguments.push(list.to_string_lossy().into_owned());
        arguments.extend(["-c:v", "copy", "-c:a", "aac", "-b:a", "128k",
                          "-movflags", "+faststart"].map(String::from));
        arguments.extend(["-y".into(), destination.to_string_lossy().into_owned()]);
        Ok(run_writing(self.ffmpeg, &arguments, destination)?)
    }

    /// One part to `output`. A source taller than the proxy is decoded by the
    /// hardware (`HWACCEL`) — decoding is most of the time on 4K, 7.8× faster
    /// on the pilot's Mac — and if the hardware takes the job and fails it, the
    /// part is made again in software: a GPU quirk must not cost the preview.
    fn transcode(&self, part: &(Clip, SourceFile), rate: FrameRate, height: i64, audio: Audio,
                 output: &Path) -> Result<PathBuf, ProxyError> {
        let attempt = |hardware: bool| {
            let mut arguments = Self::arguments(part, rate, height, audio, hardware);
            arguments.extend(["-y".into(), output.to_string_lossy().into_owned()]);
            match run_writing(self.ffmpeg, &arguments, output) {
                Err(FfmpegError::ToolFailed { ref message, .. })
                    if message.contains("Unrecognized option 'fps_mode'") => {
                    // FFmpeg 4 uses -vsync; FFmpeg 9 removed it. Keep the
                    // modern spelling unless the installed tool rejects it.
                    let index = arguments.iter().position(|arg| arg == "-fps_mode").unwrap();
                    arguments[index] = "-vsync".into();
                    arguments[index + 1] = "0".into();
                    run_writing(self.ffmpeg, &arguments, output)
                }
                result => result,
            }
        };
        if height <= MAX_PROXY_HEIGHT {
            return Ok(attempt(false)?);
        }
        match attempt(true) {
            Ok(path) => Ok(path),
            Err(error) => {
                log::warn!("hardware decoding failed, transcoding in software: {error}");
                Ok(attempt(false)?)
            }
        }
    }

    /// One part, cut at its own file's rate and resampled to `rate`; every
    /// part is scaled alike, to the flight's tallest source, so the pieces join.
    fn arguments((clip, file): &(Clip, SourceFile), rate: FrameRate, height: i64, audio: Audio,
                 hardware: bool) -> Vec<String> {
        let source = file.media.frame_rate;
        let start = source.seconds(clip.start_frame);
        let duration = source.seconds(clip.end_frame) - start;
        let mut filter = format!("fps={}/{}", rate.numerator, rate.denominator);
        if height > MAX_PROXY_HEIGHT {
            filter.push_str(&format!(",scale=-2:{MAX_PROXY_HEIGHT}"));
        }
        let mut arguments: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
        if hardware {
            arguments.extend(["-hwaccel".into(), HWACCEL.into()]);
        }
        arguments.extend([
            // Before -i, unlike the exporter. Its stream copy cannot cut
            // between index entries and landed early on these AVIs; a
            // transcode seeks to the entry before and decodes up to the exact
            // frame. After -i, ffmpeg decoded the whole file up to the flight
            // instead. Measured on the reference card (2026-10-23): the same
            // frames and samples to the bit, PICT0032 @7:33 10.1 s → 2.2 s,
            // @9:40 10.9 s → 0.7 s, PICT0031 @5:10 7.3 s → 1.9 s. The proxy's
            // clock still matches FlightTimeline's (tests/real_proxy.rs).
            "-ss".into(), seconds_argument(start),
            "-i".into(), file.imported_path.clone(),
            "-t".into(), seconds_argument(duration),
            // DVR footage stays 640x480 square pixels as recorded: no scale, no
            // SAR — conforming resamples every frame and softens the OSD's
            // 1-pixel outlines. yuv420p because MJPEG decodes to yuvj422p,
            // which hardware decoders do not play.
            "-vf".into(), filter,
            // MPEG-4 Part 2 limits the encoder timebase denominator to 65535;
            // DVR rates have denominators in the millions. A 60 kHz encoder
            // clock represents their timestamps without changing playback speed.
            "-c:v".into(), "mpeg4".into(), "-q:v".into(), "2".into(),
            "-enc_time_base".into(), "1:60000".into(),
            "-fps_mode".into(), "passthrough".into(),
            "-pix_fmt".into(), "yuv420p".into(),
        ]);
        arguments.extend(match audio {
            // The DVR records pcm_s16le, which MP4 will not carry: -c:a copy
            // fails here, with the reason only in stderr.
            Audio::Aac => ["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"].as_slice(),
            Audio::Pcm => ["-c:a", "pcm_s16le"].as_slice(),
        }.iter().map(|s| s.to_string()));
        arguments
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use fpv_core::MediaInfo;
    use fpv_media::{FfmpegTool, ProcessResult};
    use uuid::Uuid;

    use super::*;

    #[derive(Default)]
    struct RecordingFfmpeg {
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FfmpegInvoking for RecordingFfmpeg {
        fn run(&self, _: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
            self.calls.lock().unwrap().push(arguments.to_vec());
            if let Some(index) = arguments.iter().rposition(|a| a == "-y") {
                std::fs::write(&arguments[index + 1], b"v").unwrap();
            }
            Ok(ProcessResult { status: 0, stdout: vec![], stderr: String::new() })
        }
    }

    fn source(name: &str, rate: FrameRate, height: i64) -> SourceFile {
        SourceFile::new(Uuid::new_v4(), format!("/card/{name}"), format!("/library/{name}"),
                        Some(30), MediaInfo::new(height * 4 / 3, height, rate, 600.0, "mjpeg"))
    }

    fn has(arguments: &[String], text: &str) -> bool {
        arguments.iter().any(|a| a.contains(text))
    }

    #[test]
    fn transcodes_the_flight_the_lossless_exporter_refuses() {
        let ffmpeg = RecordingFfmpeg::default();
        let out = tempfile::tempdir().unwrap();
        let destination = out.path().join("flight.mp4");
        let first = source("PICT0030.AVI", FrameRate::new(1_000_000, 33459), 480);
        let second = source("PICT0031.AVI", FrameRate::new(1_000_000, 40839), 480);
        let parts = [(Clip::new(Uuid::new_v4(), first.id, 4194, 4217), first),
                     (Clip::new(Uuid::new_v4(), second.id, 0, 1475), second)];
        ProxyGenerator::new(&ffmpeg).generate(&parts, &destination).unwrap();

        let calls = ffmpeg.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3);
        for (cut, (clip, file)) in calls[..2].iter().zip(&parts) {
            assert!(has(cut, "mpeg4") && has(cut, "yuv420p") && !has(cut, "copy"));
            // PCM in the pieces; AAC only once, after the join.
            assert!(!has(cut, "aac") && has(cut, "pcm_s16le"));
            assert!(cut.last().unwrap().ends_with(".mov"));
            assert!(cut.windows(2).any(|w| w == ["-vf", "fps=1000000/40839"]));
            // Never scaled, never given a SAR.
            assert!(!has(cut, "scale") && !has(cut, "setsar"));
            // Seeking before the input: decoding up to the flight took most
            // of the time for flights late in a recording.
            let input = cut.iter().position(|a| a == "-i").unwrap();
            let seek = cut.iter().position(|a| a == "-ss").unwrap();
            assert!(seek < input && cut[input + 1] == file.imported_path);
            assert!(cut.windows(2).any(|w| w == ["-enc_time_base", "1:60000"]));
            assert!(cut.windows(2).any(|w| w == ["-fps_mode", "passthrough"]));
            let at: f64 = cut[seek + 1].parse().unwrap();
            assert!((at - file.media.frame_rate.seconds(clip.start_frame)).abs() < 0.001);
        }
        let join = &calls[2];
        assert!(has(join, "concat") && has(join, "copy") && has(join, "aac") && !has(join, "mpeg4"));
        assert_eq!(join.last().unwrap(), &destination.to_string_lossy());
    }

    #[test]
    fn transcodes_a_single_part_flight_in_one_pass() {
        let ffmpeg = RecordingFfmpeg::default();
        let out = tempfile::tempdir().unwrap();
        let file = source("PICT0034.AVI", FrameRate::new(3125, 126), 480);
        ProxyGenerator::new(&ffmpeg)
            .generate(&[(Clip::new(Uuid::new_v4(), file.id, 0, 1000), file)],
                      &out.path().join("flight.mp4"))
            .unwrap();
        let calls = ffmpeg.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert!(has(&calls[0], "mpeg4") && has(&calls[0], "fps=3125/126") && has(&calls[0], "aac"));
    }

    fn take(name: &str, rate: FrameRate, height: i64) -> Vec<String> {
        let ffmpeg = RecordingFfmpeg::default();
        let out = tempfile::tempdir().unwrap();
        let file = source(name, rate, height);
        ProxyGenerator::new(&ffmpeg)
            .generate(&[(Clip::new(Uuid::new_v4(), file.id, 0, 300), file)], &out.path().join("take.mp4"))
            .unwrap();
        ffmpeg.calls.lock().unwrap()[0].clone()
    }

    fn filter(call: &[String]) -> &str {
        &call[call.iter().position(|a| a == "-vf").unwrap() + 1]
    }

    #[test]
    fn plays_a_4k_action_cam_take_from_a_720p_proxy_at_half_its_rate() {
        // The pilot's Osmo takes: 4K, 59.94 fps. 720p30 is enough to watch and
        // tag (tags go back to source frames through seconds) and cuts a third
        // of the time; the rest is decoding, which the hardware does faster.
        let call = take("DJI_0042.MP4", FrameRate::new(60000, 1001), 2160);
        assert_eq!(filter(&call), "fps=30000/1001,scale=-2:720");
        let input = call.iter().position(|a| a == "-i").unwrap();
        let hwaccel = call.iter().position(|a| a == "-hwaccel").unwrap();
        assert!(hwaccel < input);
        // Named on the Mac, where it was measured; ffmpeg's pick elsewhere.
        let expected = if cfg!(target_os = "macos") { "videotoolbox" } else { "auto" };
        assert_eq!(call[hwaccel + 1], expected);
    }

    #[test]
    fn halves_a_high_rate_until_it_is_30_or_less_and_keeps_the_rest() {
        assert_eq!(filter(&take("GX010043.MP4", FrameRate::new(120, 1), 1080)), "fps=30/1,scale=-2:720");
        assert_eq!(filter(&take("GX010044.MP4", FrameRate::new(25, 1), 2160)), "fps=25/1,scale=-2:720");
        // 720 lines already: nothing to scale, nothing for the hardware to do.
        let call = take("GX010045.MP4", FrameRate::new(30000, 1001), 720);
        assert_eq!(filter(&call), "fps=30000/1001");
        assert!(!has(&call, "-hwaccel"));
    }

    #[test]
    fn supports_ffmpeg_before_fps_mode_was_added() {
        struct OlderFfmpeg(RecordingFfmpeg);
        impl FfmpegInvoking for OlderFfmpeg {
            fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
                if arguments.iter().any(|arg| arg == "-fps_mode") {
                    return Ok(ProcessResult { status: 1, stdout: vec![],
                        stderr: "Unrecognized option 'fps_mode'.".into() });
                }
                assert!(arguments.windows(2).any(|pair| pair == ["-vsync", "0"]));
                self.0.run(tool, arguments)
            }
        }
        let ffmpeg = OlderFfmpeg(RecordingFfmpeg::default());
        let out = tempfile::tempdir().unwrap();
        let file = source("PICT0030.AVI", FrameRate::new(30, 1), 480);
        let destination = out.path().join("take.mp4");
        ProxyGenerator::new(&ffmpeg)
            .generate(&[(Clip::new(Uuid::new_v4(), file.id, 0, 30), file)], &destination).unwrap();
        assert!(destination.is_file());
    }

    #[test]
    fn leaves_the_dvr_as_recorded() {
        let call = take("PICT0030.AVI", FrameRate::new(1_000_000, 33459), 480);
        assert_eq!(filter(&call), "fps=1000000/33459");
        assert!(!has(&call, "-hwaccel"));
    }

    #[test]
    fn falls_back_to_software_when_the_hardware_decoder_fails() {
        /// A GPU that accepts the job and then fails it.
        #[derive(Default)]
        struct FlakyGpu(Mutex<Vec<Vec<String>>>);
        impl FfmpegInvoking for FlakyGpu {
            fn run(&self, _: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
                self.0.lock().unwrap().push(arguments.to_vec());
                if arguments.iter().any(|a| a == "-hwaccel") {
                    return Ok(ProcessResult { status: 1, stdout: vec![], stderr: "hwaccel failed".into() });
                }
                std::fs::write(arguments.last().unwrap(), b"v").unwrap();
                Ok(ProcessResult { status: 0, stdout: vec![], stderr: String::new() })
            }
        }
        let ffmpeg = FlakyGpu::default();
        let out = tempfile::tempdir().unwrap();
        let file = source("DJI_0042.MP4", FrameRate::new(60000, 1001), 2160);
        let written = ProxyGenerator::new(&ffmpeg)
            .generate(&[(Clip::new(Uuid::new_v4(), file.id, 0, 300), file)], &out.path().join("take.mp4"))
            .unwrap();
        assert!(written.is_file());
        let calls = ffmpeg.0.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(has(&calls[0], "-hwaccel") && !has(&calls[1], "-hwaccel"));
    }

    #[test]
    fn refuses_an_empty_flight() {
        let result = ProxyGenerator::new(&RecordingFfmpeg::default())
            .generate(&[], Path::new("/tmp/never.mp4"));
        assert!(matches!(result, Err(ProxyError::NothingToGenerate)));
    }

    #[test]
    fn reports_a_failed_transcode_with_its_arguments() {
        struct Failing;
        impl FfmpegInvoking for Failing {
            fn run(&self, _: FfmpegTool, _: &[String]) -> Result<ProcessResult, FfmpegError> {
                Ok(ProcessResult { status: 1, stdout: vec![],
                                   stderr: "Could not find tag for codec pcm_s16le".into() })
            }
        }
        let out = tempfile::tempdir().unwrap();
        let file = source("PICT0032.AVI", FrameRate::new(250_000, 10009), 480);
        let error = ProxyGenerator::new(&Failing)
            .generate(&[(Clip::new(Uuid::new_v4(), file.id, 0, 10), file)],
                      &out.path().join("f.mp4"))
            .unwrap_err();
        assert!(error.to_string().contains("pcm_s16le") && error.to_string().contains("mpeg4"));
    }
}
