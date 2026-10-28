use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fpv_core::{Clip, SourceFile, uuid_string};

use crate::{FfmpegError, FfmpegInvoking, FfmpegTool};

/// Pulls a still out of a recording.
pub struct ThumbnailGenerator<'a> {
    ffmpeg: &'a dyn FfmpegInvoking,
}

impl<'a> ThumbnailGenerator<'a> {
    pub fn new(ffmpeg: &'a dyn FfmpegInvoking) -> Self {
        Self { ffmpeg }
    }

    /// Where in a clip to take the poster frame.
    ///
    /// Not the first frame: a clip opens on the moment the picture returns, and
    /// analog reception comes back through a fade — bands of sync loss and noise
    /// that read as signal but look like nothing. Measured on PICT0032, that
    /// fade ran 1.5 s (CLAUDE.md 10), so the floor is 2 s.
    ///
    /// Capped at 10 s so a long flight's poster still shows its take-off, and
    /// halved for clips too short to hold the floor — the reference set contains
    /// a legitimate 0.75 s one, the opening of a flight the DVR cut across two
    /// files, and asking ffmpeg to seek past its end writes nothing at all.
    pub fn poster_offset(clip_duration: f64) -> f64 {
        if clip_duration < 4.0 {
            return (clip_duration / 2.0).max(0.0);
        }
        (clip_duration * 0.25).clamp(2.0, 10.0)
    }

    /// Writes a JPEG still taken `seconds` into `source`, `width` wide; the
    /// height follows the source aspect.
    ///
    /// The still appears under `destination` only once whole: ffmpeg writes a
    /// hidden file beside it, which is then renamed over. Strips are made in
    /// the background while the pointer may ask for the same frame, and a
    /// half-written JPEG must never be the one that is shown.
    pub fn generate(
        &self,
        source: &Path,
        seconds: f64,
        width: u32,
        destination: &Path,
    ) -> Result<PathBuf, FfmpegError> {
        self.generate_with(source, seconds, width, destination, false, false)
    }

    /// Like `generate`, but the still is the keyframe at or before `seconds`,
    /// and only that keyframe is decoded. For hover strips on long-GOP
    /// action-cam takes: an exact still decodes from the previous keyframe up
    /// to the frame, which on 10-bit 4K HEVC was ~1.7 s a still on the pilot's
    /// Mac; a strip does not need the exact frame. Never the keyframe *after*
    /// — near the end there may be none, and ffmpeg then writes nothing.
    pub fn generate_near(
        &self,
        source: &Path,
        seconds: f64,
        width: u32,
        destination: &Path,
    ) -> Result<PathBuf, FfmpegError> {
        self.generate_with(source, seconds, width, destination, true, false)
    }

    fn generate_with(
        &self,
        source: &Path,
        seconds: f64,
        width: u32,
        destination: &Path,
        near: bool,
        output_seek: bool,
    ) -> Result<PathBuf, FfmpegError> {
        let io = |source| FfmpegError::Launch {
            tool: "ffmpeg".into(),
            source,
        };
        let parent = destination.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(io)?;
        // Unique per writer, so two making the same still never share one;
        // ending in .jpg, which is how ffmpeg picks the format.
        static WRITERS: AtomicU64 = AtomicU64::new(0);
        let name = destination
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let partial = parent.join(format!(
            ".{name}.{}-{}.jpg",
            std::process::id(),
            WRITERS.fetch_add(1, Ordering::Relaxed)
        ));
        let written = self.write(source, seconds, width, &partial, near, output_seek);
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
            return written;
        }
        std::fs::rename(&partial, destination).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            io(error)
        })?;
        Ok(destination.to_path_buf())
    }

    fn write(
        &self,
        source: &Path,
        seconds: f64,
        width: u32,
        destination: &Path,
        near: bool,
        output_seek: bool,
    ) -> Result<PathBuf, FfmpegError> {
        // -ss before -i seeks by index instead of decoding up to the point, which
        // is the difference between milliseconds and a minute on a 600 s file.
        let mut arguments: Vec<String> =
            vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
        if near {
            // -copyts keeps the keyframe's own time: without it the seek point
            // becomes 0, the keyframe before it goes negative and is dropped,
            // and near the end of a take nothing is written (measured).
            arguments
                .extend(["-skip_frame", "nokey", "-noaccurate_seek", "-copyts"].map(String::from));
        }
        if !output_seek {
            arguments.extend(["-ss".into(), format!("{:.3}", seconds.max(0.0))]);
        }
        arguments.extend(["-i".into(), source.to_string_lossy().into_owned()]);
        if output_seek {
            // Some AVI indexes seek into an H.264 GOP without its parameter
            // sets. Output-side seeking decodes from the stream start and is
            // slower, but produces a real frame for long-GOP AVI sources.
            arguments.extend(["-ss".into(), format!("{:.3}", seconds.max(0.0))]);
        }
        arguments.extend([
            "-frames:v".into(),
            "1".into(),
            "-an".into(),
            "-vf".into(),
            format!("scale={width}:-2"),
            "-q:v".into(),
            "3".into(),
            "-y".into(),
            destination.to_string_lossy().into_owned(),
        ]);
        let result = self.ffmpeg.run(FfmpegTool::Ffmpeg, &arguments)?;
        if !result.succeeded() {
            return Err(FfmpegError::tool_failed(
                FfmpegTool::Ffmpeg,
                &arguments,
                &result,
            ));
        }
        if !destination.exists() {
            // ffmpeg reports success when a seek lands past the last frame and it
            // writes nothing at all, which would otherwise surface much later as
            // a missing thumbnail with no explanation.
            return Err(FfmpegError::ToolFailed {
                tool: "ffmpeg".into(),
                arguments,
                status: 0,
                message: "no frame written — seek past end of file?".into(),
            });
        }
        Ok(destination.to_path_buf())
    }

    /// The poster frame for one clip, named after it.
    pub fn generate_for_clip(
        &self,
        clip: &Clip,
        file: &SourceFile,
        directory: &Path,
        width: u32,
    ) -> Result<PathBuf, FfmpegError> {
        let rate = file.media.frame_rate;
        let start = rate.seconds(clip.start_frame);
        let duration = rate.seconds(clip.end_frame) - start;
        let name = format!("{}.jpg", uuid_string(clip.id));
        self.generate_with(
            Path::new(&file.imported_path),
            start + Self::poster_offset(duration),
            width,
            &directory.join(name),
            false,
            !file.media.is_all_intra(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use fpv_core::{FrameRate, MediaInfo};
    use uuid::Uuid;

    use super::*;
    use crate::ProcessResult;

    struct RecordingFfmpeg {
        calls: Mutex<Vec<Vec<String>>>,
        write_output: bool,
    }

    impl RecordingFfmpeg {
        fn new(write_output: bool) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                write_output,
            }
        }
        fn last_call(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .last()
                .cloned()
                .unwrap_or_default()
        }
    }

    impl FfmpegInvoking for RecordingFfmpeg {
        fn run(&self, _: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
            self.calls.lock().unwrap().push(arguments.to_vec());
            if let Some(index) = arguments.iter().rposition(|a| a == "-y")
                && self.write_output
            {
                std::fs::write(&arguments[index + 1], b"x").unwrap();
            }
            Ok(ProcessResult {
                status: 0,
                stdout: vec![],
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn seeks_by_index_rather_than_decoding_up_to_the_point() {
        let ffmpeg = RecordingFfmpeg::new(true);
        let directory = tempfile::tempdir().unwrap();
        ThumbnailGenerator::new(&ffmpeg)
            .generate(
                Path::new("/library/PICT0032.AVI"),
                120.0,
                480,
                &directory.path().join("poster.jpg"),
            )
            .unwrap();
        let arguments = ffmpeg.last_call();
        let seek = arguments.iter().position(|a| a == "-ss").unwrap();
        let input = arguments.iter().position(|a| a == "-i").unwrap();
        // -ss after -i decodes 120 s of MJPEG before writing one frame.
        assert!(seek < input);
    }

    #[test]
    fn never_lets_a_half_written_still_be_seen() {
        // Strips are made in the background while the pointer may be asking
        // for the same frame: ffmpeg writes beside it, and the still appears
        // under its name only once whole.
        let ffmpeg = RecordingFfmpeg::new(true);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("strip-3.jpg");
        ThumbnailGenerator::new(&ffmpeg)
            .generate(Path::new("/library/PICT0032.AVI"), 12.0, 320, &destination)
            .unwrap();
        let arguments = ffmpeg.last_call();
        let written = &arguments[arguments.iter().rposition(|a| a == "-y").unwrap() + 1];
        assert_ne!(Path::new(written), destination);
        assert!(
            written.ends_with(".jpg"),
            "ffmpeg picks the format from the name: {written}"
        );
        assert!(destination.is_file());
        let left: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["strip-3.jpg"]);
    }

    #[test]
    fn decodes_only_the_keyframe_at_or_before_a_rough_position() {
        let ffmpeg = RecordingFfmpeg::new(true);
        let directory = tempfile::tempdir().unwrap();
        ThumbnailGenerator::new(&ffmpeg)
            .generate_near(
                Path::new("/library/DJI_0042.MP4"),
                400.0,
                320,
                &directory.path().join("strip.jpg"),
            )
            .unwrap();
        let arguments = ffmpeg.last_call();
        let input = arguments.iter().position(|a| a == "-i").unwrap();
        for flag in ["-skip_frame", "nokey", "-noaccurate_seek", "-copyts", "-ss"] {
            let at = arguments
                .iter()
                .position(|a| a == flag)
                .unwrap_or_else(|| panic!("no {flag}"));
            assert!(at < input, "{flag} must come before -i");
        }
    }

    #[test]
    fn takes_the_poster_frame_past_the_fade_in() {
        assert_eq!(ThumbnailGenerator::poster_offset(5.0), 2.0);
        assert_eq!(ThumbnailGenerator::poster_offset(20.0), 5.0);
        assert_eq!(ThumbnailGenerator::poster_offset(600.0), 10.0);
        // The 0.75 s clip in PICT0030 is real: seeking 2 s into it writes nothing.
        assert_eq!(ThumbnailGenerator::poster_offset(0.75), 0.375);
    }

    #[test]
    fn places_a_clip_poster_using_the_files_own_frame_rate() {
        let ffmpeg = RecordingFfmpeg::new(true);
        let directory = tempfile::tempdir().unwrap();
        // 24.978 fps, not 25: frame 2500 is at 100.09 s, and a nominal rate would
        // put the poster a full second away.
        let rate = FrameRate::new(250_000, 10009);
        let file = SourceFile::new(
            Uuid::new_v4(),
            "/card/PICT0032.AVI",
            "/library/PICT0032.AVI",
            Some(32),
            MediaInfo::new(640, 480, rate, 601.6, "mjpeg"),
        );
        let clip = Clip::new(Uuid::new_v4(), file.id, 2500, 3000);
        ThumbnailGenerator::new(&ffmpeg)
            .generate_for_clip(&clip, &file, directory.path(), 480)
            .unwrap();

        let arguments = ffmpeg.last_call();
        let seek = arguments.iter().position(|a| a == "-ss").unwrap();
        let input = arguments.iter().position(|a| a == "-i").unwrap();
        assert!(
            seek < input,
            "MJPEG posters should keep fast input-side seeking"
        );
        let requested: f64 = arguments[seek + 1].parse().unwrap();
        let start = rate.seconds(2500);
        let expected = start + ThumbnailGenerator::poster_offset(rate.seconds(3000) - start);
        assert!((requested - expected).abs() < 0.01);
        assert!((requested - 105.1).abs() < 0.1);
    }

    #[test]
    fn long_gop_poster_seeks_after_the_input_to_avoid_broken_avi_indexes() {
        let ffmpeg = RecordingFfmpeg::new(true);
        let directory = tempfile::tempdir().unwrap();
        let rate = FrameRate::new(30, 1);
        let file = SourceFile::new(
            Uuid::new_v4(),
            "/card/EV300D_0002.AVI",
            "/library/EV300D_0002.AVI",
            Some(32),
            MediaInfo::new(720, 576, rate, 340.5, "h264"),
        );
        let clip = Clip::new(Uuid::new_v4(), file.id, 0, 10_215);
        ThumbnailGenerator::new(&ffmpeg)
            .generate_for_clip(&clip, &file, directory.path(), 480)
            .unwrap();
        let arguments = ffmpeg.last_call();
        let seek = arguments.iter().position(|a| a == "-ss").unwrap();
        let input = arguments.iter().position(|a| a == "-i").unwrap();
        assert!(
            input < seek,
            "long-GOP AVI thumbnails must decode through the requested frame"
        );
    }

    #[test]
    fn fails_loudly_when_ffmpeg_writes_nothing() {
        // ffmpeg exits 0 when a seek lands past the last frame and writes no file.
        let ffmpeg = RecordingFfmpeg::new(false);
        let directory = tempfile::tempdir().unwrap();
        let result = ThumbnailGenerator::new(&ffmpeg).generate(
            Path::new("/library/PICT0030.AVI"),
            9999.0,
            480,
            &directory.path().join("x.jpg"),
        );
        assert!(matches!(
            result,
            Err(FfmpegError::ToolFailed { status: 0, .. })
        ));
    }
}
