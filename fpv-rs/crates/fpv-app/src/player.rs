//! Native, bounded ffmpeg pipes. No browser, media server or linked libav*.
//! Source parts are decoded separately so each keeps its own capture rate.
use std::collections::VecDeque;
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fpv_library::NativeSegment;
use fpv_media::{FfmpegTool, seconds_argument};

const DISPLAY_RATE: f64 = 30.0;
/// The hardware decoder asked for long-GOP footage. On the pilot's Mac, 30 s of
/// 4K 10-bit HEVC decoded in 78 s in software and 10 s with VideoToolbox
/// (CLAUDE.md, "Proxy"), so software alone cannot play it in real time.
/// Elsewhere the bundled FFmpeg has no hardware decoder to ask for.
const HWACCEL: Option<&str> = if cfg!(target_os = "macos") {
    Some("videotoolbox")
} else {
    None
};
const MAX_WIDTH: usize = 1280;
const MAX_HEIGHT: usize = 720;

pub struct VideoFrame {
    pub at: f64,
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

struct Clock {
    start: Option<Instant>,
    paused_at: Option<Instant>,
    paused_for: Duration,
}

struct State {
    stop: AtomicBool,
    paused: AtomicBool,
    audio: AtomicBool,
    consumed: AtomicU64,
    sample_rate: AtomicU64,
    frames: Mutex<VecDeque<VideoFrame>>,
    samples: Mutex<VecDeque<f32>>,
    clock: Mutex<Clock>,
    children: Mutex<Vec<Arc<Mutex<Child>>>>,
    warnings: Mutex<Vec<String>>,
    base: f64,
    duration: f64,
    requested_at: Instant,
    volume: Mutex<f32>,
    video_done: AtomicBool,
    context: eframe::egui::Context,
    source_fps: Option<f64>,
    hwaccel: Option<&'static str>,
}

impl State {
    fn warn(&self, warning: String) {
        log::warn!("{warning}");
        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(warning);
        }
        self.context.request_repaint();
    }

    fn time(&self) -> f64 {
        let elapsed = if self.audio.load(Ordering::Relaxed) {
            self.consumed.load(Ordering::Relaxed) as f64
                / self.sample_rate.load(Ordering::Relaxed).max(1) as f64
        } else if let Ok(clock) = self.clock.lock() {
            clock
                .start
                .map(|start| {
                    clock
                        .paused_at
                        .unwrap_or_else(Instant::now)
                        .saturating_duration_since(start)
                        .saturating_sub(clock.paused_for)
                        .as_secs_f64()
                })
                .unwrap_or_default()
        } else {
            0.0
        };
        (self.base + elapsed).min(self.duration)
    }

    fn started(&self) -> bool {
        self.clock.lock().is_ok_and(|clock| clock.start.is_some())
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(children) = self.children.lock() {
            for child in children.iter() {
                if let Ok(mut child) = child.lock() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
}

pub struct Player {
    state: Arc<State>,
    audio_enabled: bool,
    tools: Option<PathBuf>,
    parts: Vec<NativeSegment>,
    context: eframe::egui::Context,
    source_fps: Option<f64>,
    hwaccel: Option<&'static str>,
}

impl Player {
    #[cfg(test)]
    pub fn new(
        parts: Vec<NativeSegment>,
        tools: Option<PathBuf>,
        at: f64,
        context: eframe::egui::Context,
    ) -> Self {
        Self::new_with_volume(parts, tools, at, context, 1.0)
    }

    /// A player asking for `hwaccel` instead of the platform's own.
    #[cfg(test)]
    fn with_hwaccel(
        parts: Vec<NativeSegment>,
        tools: Option<PathBuf>,
        at: f64,
        context: eframe::egui::Context,
        hwaccel: Option<&'static str>,
    ) -> Self {
        let decoding = Decoding {
            source_fps: None,
            hwaccel,
        };
        Self::start(parts, tools, at, context, 1.0, false, decoding)
    }

    pub fn new_with_volume(
        parts: Vec<NativeSegment>,
        tools: Option<PathBuf>,
        at: f64,
        context: eframe::egui::Context,
        volume: f32,
    ) -> Self {
        Self::start(
            parts,
            tools,
            at,
            context,
            volume,
            volume > 0.0,
            Decoding {
                source_fps: None,
                hwaccel: HWACCEL,
            },
        )
    }

    /// Boundary previews decode every original source frame, without the normal
    /// display-rate conversion. Preserve this mode across seeks.
    pub fn new_source_preview(
        parts: Vec<NativeSegment>,
        tools: Option<PathBuf>,
        at: f64,
        context: eframe::egui::Context,
        volume: f32,
        fps: f64,
    ) -> Self {
        Self::start(
            parts,
            tools,
            at,
            context,
            volume,
            volume > 0.0,
            Decoding {
                source_fps: Some(fps),
                hwaccel: HWACCEL,
            },
        )
    }

    fn start(
        parts: Vec<NativeSegment>,
        tools: Option<PathBuf>,
        at: f64,
        context: eframe::egui::Context,
        volume: f32,
        audio_enabled: bool,
        Decoding {
            source_fps,
            hwaccel,
        }: Decoding,
    ) -> Self {
        let duration = parts
            .last()
            .map(|p| p.segment.offset + p.segment.end - p.segment.start)
            .unwrap_or_default()
            .max(0.0);
        let at = bounded_time(at, duration);
        let state = Arc::new(State {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            audio: AtomicBool::new(audio_enabled && parts.iter().any(|p| p.has_audio)),
            consumed: AtomicU64::new(0),
            sample_rate: AtomicU64::new(48000),
            frames: Mutex::new(VecDeque::new()),
            samples: Mutex::new(VecDeque::new()),
            clock: Mutex::new(Clock {
                start: None,
                paused_at: None,
                paused_for: Duration::ZERO,
            }),
            children: Mutex::new(vec![]),
            warnings: Mutex::new(vec![]),
            base: at,
            duration,
            requested_at: Instant::now(),
            volume: Mutex::new(volume.clamp(0.0, 1.0)),
            video_done: AtomicBool::new(false),
            context: context.clone(),
            source_fps,
            hwaccel,
        });
        let s = state.clone();
        let t = tools.clone();
        let p = parts.clone();
        std::thread::spawn(move || {
            if let Err(error) = decode_video(&s, &p, t)
                && !s.stop.load(Ordering::Relaxed)
            {
                s.warn(format!("Video playback: {error}"));
            }
            s.video_done.store(true, Ordering::Relaxed);
            s.context.request_repaint();
        });
        if state.audio.load(Ordering::Relaxed) {
            let s = state.clone();
            let t = tools.clone();
            let p = parts.clone();
            std::thread::spawn(move || {
                if let Err(error) = audio_output(&s, p, t) {
                    // Keep the current instant when switching to the wall clock.
                    let elapsed = s.consumed.load(Ordering::Relaxed) as f64
                        / s.sample_rate.load(Ordering::Relaxed).max(1) as f64;
                    if let Ok(mut clock) = s.clock.lock()
                        && clock.start.is_some()
                    {
                        clock.start = Some(Instant::now() - Duration::from_secs_f64(elapsed));
                        clock.paused_for = Duration::ZERO;
                        clock.paused_at = s.paused.load(Ordering::Relaxed).then(Instant::now);
                    }
                    s.audio.store(false, Ordering::Relaxed);
                    s.warn(format!(
                        "Audio unavailable (video remains playable): {error}"
                    ));
                }
            });
        }
        Self {
            state,
            source_fps,
            hwaccel,
            audio_enabled,
            tools,
            parts,
            context,
        }
    }

    pub fn position(&self) -> f64 {
        self.state.time()
    }
    pub fn paused(&self) -> bool {
        self.state.paused.load(Ordering::Relaxed)
    }
    pub fn duration(&self) -> f64 {
        self.state.duration
    }
    pub fn ended(&self) -> bool {
        self.position() >= self.duration()
    }
    pub fn buffering(&self) -> bool {
        !self.state.started() && !self.state.video_done.load(Ordering::Relaxed)
    }

    pub fn pause(&self, pause: bool) {
        if self.state.paused.swap(pause, Ordering::Relaxed) == pause {
            return;
        }
        if let Ok(mut clock) = self.state.clock.lock() {
            if pause {
                clock.paused_at = Some(Instant::now());
            } else if let Some(at) = clock.paused_at.take() {
                clock.paused_for += at.elapsed();
            }
        }
    }

    #[cfg(test)]
    pub fn volume(&self) -> f32 {
        *self.state.volume.lock().unwrap()
    }

    /// A player opened muted runs no audio at all: turning the sound up starts
    /// it, from where the video is, so the sound can always come back.
    pub fn set_volume(&mut self, volume: f32) {
        if let Ok(mut gain) = self.state.volume.lock() {
            *gain = volume.clamp(0.0, 1.0);
        }
        if volume > 0.0 && !self.audio_enabled && self.parts.iter().any(|part| part.has_audio) {
            self.audio_enabled = true;
            // Restarts the decoders where the video is, paused or not.
            self.seek(self.position());
        }
    }

    #[cfg(test)]
    pub fn audio_enabled(&self) -> bool {
        self.audio_enabled
    }

    pub fn seek(&mut self, at: f64) {
        let paused = self.paused();
        let volume = self.state.volume.lock().map(|gain| *gain).unwrap_or(1.0);
        self.state.stop();
        *self = Self::start(
            self.parts.clone(),
            self.tools.clone(),
            at,
            self.context.clone(),
            volume,
            self.audio_enabled,
            Decoding {
                source_fps: self.source_fps,
                hwaccel: self.hwaccel,
            },
        );
        self.pause(paused);
    }

    pub fn frame(&self) -> Option<VideoFrame> {
        let mut frames = self.state.frames.lock().ok()?;
        let time = self.position();
        let mut latest = None;
        let tolerance = self.source_fps.map(|fps| 0.25 / fps).unwrap_or(0.025);
        while frames
            .front()
            .is_some_and(|frame| frame.at <= time + tolerance)
        {
            latest = frames.pop_front();
        }
        latest
    }

    /// The part playing now, of how many (`part_at`).
    pub fn part(&self) -> Option<(usize, usize)> {
        part_at(&self.parts, self.position())
    }

    pub fn warnings(&self) -> Vec<String> {
        self.state
            .warnings
            .lock()
            .map(|mut warnings| std::mem::take(&mut *warnings))
            .unwrap_or_default()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.state.stop();
    }
}

pub fn bounded_time(time: f64, duration: f64) -> f64 {
    if time.is_finite() {
        time.clamp(0.0, duration.max(0.0))
    } else {
        0.0
    }
}

pub fn dimensions(width: i64, height: i64) -> (usize, usize) {
    let width = width.clamp(1, 16384) as f64;
    let height = height.clamp(1, 16384) as f64;
    let ratio = (MAX_WIDTH as f64 / width)
        .min(MAX_HEIGHT as f64 / height)
        .min(1.0);
    (
        (width * ratio).round().max(1.0) as usize,
        (height * ratio).round().max(1.0) as usize,
    )
}

/// Which of a flight's parts plays `at` flight seconds in, 1-based, and of
/// how many — "chapter 2/2"; nothing for a flight in one file. A part's end
/// belongs to the next, as in `remaining`.
pub fn part_at(parts: &[NativeSegment], at: f64) -> Option<(usize, usize)> {
    if parts.len() < 2 {
        return None;
    }
    let index = parts
        .iter()
        .position(|part| {
            at < part.segment.offset + (part.segment.end - part.segment.start).max(0.0)
        })
        .unwrap_or(parts.len() - 1);
    Some((index + 1, parts.len()))
}

/// The seek instant belongs to the next part at an exact chapter boundary.
fn remaining(parts: &[NativeSegment], at: f64) -> impl Iterator<Item = (&NativeSegment, f64, f64)> {
    parts.iter().filter_map(move |part| {
        let length = (part.segment.end - part.segment.start).max(0.0);
        let skip = (at - part.segment.offset).max(0.0);
        (skip < length).then_some((part, skip, length - skip))
    })
}

struct Process {
    child: Arc<Mutex<Child>>,
    stdout: ChildStdout,
    errors: Option<std::thread::JoinHandle<String>>,
    arguments: Vec<String>,
}

impl Process {
    fn open(
        state: &State,
        tools: Option<&std::path::Path>,
        arguments: &[String],
    ) -> Result<Self, String> {
        log::debug!("ffmpeg {}", arguments.join(" "));
        let mut child = FfmpegTool::Ffmpeg
            .command(tools)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run ffmpeg: {error}"))?;
        let stdout = child.stdout.take().ok_or("ffmpeg has no output pipe")?;
        let mut stderr = child.stderr.take().ok_or("ffmpeg has no error pipe")?;
        let errors = std::thread::spawn(move || {
            let mut tail = VecDeque::new();
            let mut buffer = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                tail.extend(&buffer[..n]);
                while tail.len() > 8192 {
                    tail.pop_front();
                }
            }
            String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned()
        });
        let child = Arc::new(Mutex::new(child));
        if let Ok(mut children) = state.children.lock() {
            // Cancellation can win the race between spawn and registration.
            children.push(child.clone());
            if state.stop.load(Ordering::Relaxed)
                && let Ok(mut child) = child.lock()
            {
                let _ = child.kill();
            }
        }
        Ok(Self {
            child,
            stdout,
            errors: Some(errors),
            arguments: arguments.to_vec(),
        })
    }

    fn finish(&mut self) -> Result<(), String> {
        let status = self
            .child
            .lock()
            .map_err(|e| e.to_string())?
            .wait()
            .map_err(|e| e.to_string())?;
        let errors = self
            .errors
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "ffmpeg exited {status}: {errors}\nffmpeg {}",
                self.arguments.join(" ")
            ))
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// How the player decodes: at the source's own rate (the boundary editor)
/// or the display's, and through which hardware decoder, if any.
#[derive(Debug, Clone, Copy)]
struct Decoding {
    source_fps: Option<f64>,
    hwaccel: Option<&'static str>,
}

/// One way of decoding a part: through a hardware decoder or not, seeking on
/// the index or from the stream's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attempt {
    hwaccel: Option<&'static str>,
    output_seek: bool,
}

/// The ways to try, in order, until one yields a frame. Long-GOP footage asks
/// the hardware first and falls back to software, which a recording the
/// hardware refuses (or a machine without it) still plays; MJPEG is cheap to
/// decode and no hardware decoder takes it. A seek into long-GOP footage keeps
/// its last resort, decoding from the start.
fn attempts(part: &NativeSegment, input_time: f64, hwaccel: Option<&'static str>) -> Vec<Attempt> {
    let mut plan = vec![];
    if !part.all_intra
        && let Some(hwaccel) = hwaccel
    {
        plan.push(Attempt {
            hwaccel: Some(hwaccel),
            output_seek: false,
        });
    }
    plan.push(Attempt {
        hwaccel: None,
        output_seek: false,
    });
    if !part.all_intra && input_time > 0.0 {
        plan.push(Attempt {
            hwaccel: None,
            output_seek: true,
        });
    }
    plan
}

fn input(part: &NativeSegment, skip: f64, duration: f64, output_seek: bool) -> Vec<String> {
    input_through(
        part,
        skip,
        duration,
        Attempt {
            hwaccel: None,
            output_seek,
        },
    )
}

fn input_through(part: &NativeSegment, skip: f64, duration: f64, attempt: Attempt) -> Vec<String> {
    let output_seek = attempt.output_seek;
    let mut arguments = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    if let Some(hwaccel) = attempt.hwaccel {
        arguments.extend(["-hwaccel".into(), hwaccel.into()]);
    }
    if !output_seek {
        arguments.extend(["-ss".into(), seconds_argument(part.segment.start + skip)]);
    }
    arguments.extend([
        "-i".into(),
        part.segment.path.to_string_lossy().into_owned(),
    ]);
    if output_seek {
        // Retry from the stream start if a broken index makes input seeking fail.
        arguments.extend(["-ss".into(), seconds_argument(part.segment.start + skip)]);
    }
    arguments.extend(["-t".into(), seconds_argument(duration)]);
    arguments
}

fn decode_video(
    state: &State,
    parts: &[NativeSegment],
    tools: Option<PathBuf>,
) -> Result<(), String> {
    for (part, skip, length) in remaining(parts, state.base) {
        if state.stop.load(Ordering::Relaxed) {
            break;
        }
        let (width, height) = dimensions(part.width, part.height);
        let input_time = part.segment.start + skip;
        let plan = attempts(part, input_time, state.hwaccel);
        let mut decoded = false;
        for (n, &attempt) in plan.iter().enumerate() {
            let mut args = input_through(part, skip, length, attempt);
            args.extend([
                "-an".into(),
                "-vf".into(),
                if state.source_fps.is_some() {
                    format!("scale={width}:{height}")
                } else {
                    format!("fps={DISPLAY_RATE},scale={width}:{height}")
                },
                "-pix_fmt".into(),
                "rgb24".into(),
                "-f".into(),
                "rawvideo".into(),
                "pipe:1".into(),
            ]);
            let mut process = Process::open(state, tools.as_deref(), &args)?;
            let mut index = 0;
            loop {
                if state.stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if state
                    .frames
                    .lock()
                    .map(|frames| frames.len() >= 4)
                    .unwrap_or(true)
                {
                    std::thread::sleep(Duration::from_millis(4));
                    continue;
                }
                let mut rgb = vec![0; width * height * 3];
                match process.stdout.read_exact(&mut rgb) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error.to_string()),
                }
                decoded = true;
                let frame = VideoFrame {
                    at: part.segment.offset
                        + skip
                        + index as f64 / state.source_fps.unwrap_or(DISPLAY_RATE),
                    width,
                    height,
                    rgb,
                };
                if let Ok(mut frames) = state.frames.lock() {
                    frames.push_back(frame);
                }
                if let Ok(mut clock) = state.clock.lock()
                    && clock.start.is_none()
                {
                    let now = Instant::now();
                    log::debug!(
                        "First video frame at {:.3}s after {:?} (attempt: {attempt:?})",
                        state.base,
                        state.requested_at.elapsed()
                    );
                    clock.start = Some(now);
                    clock.paused_at = state.paused.load(Ordering::Relaxed).then_some(now);
                }
                state.context.request_repaint();
                index += 1;
            }
            let result = process.finish();
            if decoded {
                result?;
                break;
            }
            if n + 1 == plan.len() {
                result?;
                return Err("ffmpeg produced no video frames".into());
            }
            log::debug!(
                "{attempt:?} produced no frame ({}); trying {:?}",
                result.err().unwrap_or_default(),
                plan[n + 1]
            );
        }
    }
    Ok(())
}

fn audio_output(
    state: &Arc<State>,
    parts: Vec<NativeSegment>,
    tools: Option<PathBuf>,
) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    if channels == 0 || rate == 0 {
        return Err("invalid audio device format".into());
    }
    state.sample_rate.store(rate as u64, Ordering::Relaxed);
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config.into(), state.clone(), channels)?,
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config.into(), state.clone(), channels)?,
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config.into(), state.clone(), channels)?,
        format => return Err(format!("unsupported device sample format: {format}")),
    };
    stream.play().map_err(|error| error.to_string())?;
    let result = decode_audio(state, &parts, tools, rate, channels);
    // Keep the output stream alive until buffered samples have played.
    while result.is_ok() && !state.stop.load(Ordering::Relaxed) && state.time() < state.duration {
        std::thread::sleep(Duration::from_millis(10));
    }
    result
}

fn stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<State>,
    channels: usize,
) -> Result<cpal::Stream, String> {
    let errors = state.clone();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let playing = state.started()
                    && !state.paused.load(Ordering::Relaxed)
                    && !state.stop.load(Ordering::Relaxed);
                let gain = state.volume.try_lock().map(|gain| *gain).unwrap_or(1.0);
                let mut samples = state.samples.try_lock().ok();
                let available = samples.as_ref().map(|samples| samples.len()).unwrap_or(0);
                // An underrun freezes the audio master clock and hence the picture.
                let take = if playing {
                    available.min(output.len()) / channels * channels
                } else {
                    0
                };
                for (index, sample) in output.iter_mut().enumerate() {
                    let value = if index < take {
                        samples
                            .as_mut()
                            .and_then(|samples| samples.pop_front())
                            .unwrap_or_default()
                            * gain
                    } else {
                        0.0
                    };
                    *sample = T::from_sample(value);
                }
                state
                    .consumed
                    .fetch_add((take / channels) as u64, Ordering::Relaxed);
            },
            move |error| errors.warn(format!("Audio output: {error}")),
            None,
        )
        .map_err(|error| error.to_string())
}

fn decode_audio(
    state: &State,
    parts: &[NativeSegment],
    tools: Option<PathBuf>,
    rate: u32,
    channels: usize,
) -> Result<(), String> {
    let limit = rate as usize * channels; // One second ahead, bounded even on long takes.
    for (part, skip, length) in remaining(parts, state.base) {
        if state.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let target = (length * rate as f64).ceil() as usize * channels;
        let mut written = 0;
        let mut process = if part.has_audio {
            let mut args = input(part, skip, length, false);
            args.extend([
                "-vn".into(),
                "-ac".into(),
                channels.to_string(),
                "-ar".into(),
                rate.to_string(),
                "-f".into(),
                "f32le".into(),
                "-acodec".into(),
                "pcm_f32le".into(),
                "pipe:1".into(),
            ]);
            Some(Process::open(state, tools.as_deref(), &args)?)
        } else {
            None
        };
        let mut ended = false;
        while written < target {
            if state.stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if state
                .samples
                .lock()
                .map(|samples| samples.len() >= limit)
                .unwrap_or(true)
            {
                std::thread::sleep(Duration::from_millis(4));
                continue;
            }
            let count = (target - written).min(1024 * channels);
            let mut bytes = vec![0; count * 4];
            if let Some(process) = &mut process
                && !ended
            {
                match process.stdout.read_exact(&mut bytes) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                        ended = true;
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            let decoded = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
            if let Ok(mut samples) = state.samples.lock() {
                samples.extend(decoded);
            }
            written += count;
        }
        if let Some(mut process) = process {
            // Drop the pipe before wait: an encoder may have sub-frame padding
            // beyond our exact output length. Cancellation closes the child.
            if !ended {
                if let Ok(mut child) = process.child.lock() {
                    let _ = child.kill();
                }
            } else {
                process.finish()?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fpv_library::Segment;
    #[test]
    fn display_size_preserves_aspect_and_never_upscales_dvr() {
        assert_eq!(dimensions(640, 480), (640, 480));
        assert_eq!(dimensions(3840, 2160), (1280, 720));
        assert_eq!(dimensions(1080, 1920), (405, 720));
        assert_eq!(dimensions(0, -1), (1, 1));
    }
    #[test]
    fn chapter_boundary_and_seeks_keep_each_sources_time() {
        let part = |start, end, offset| NativeSegment {
            segment: Segment {
                path: "test.avi".into(),
                start,
                end,
                offset,
            },
            width: 640,
            height: 480,
            has_audio: false,
            all_intra: true,
        };
        let parts = [part(140.0, 141.0, 0.0), part(0.0, 60.0, 1.0)];
        let rest: Vec<_> = remaining(&parts, 1.0).collect();
        assert_eq!(rest.len(), 1);
        assert_eq!((rest[0].1, rest[0].2), (0.0, 60.0));
        let rest: Vec<_> = remaining(&parts, 11.0).collect();
        assert_eq!((rest[0].1, rest[0].2), (10.0, 50.0));
        let args = input(rest[0].0, rest[0].1, rest[0].2, false);
        let seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let input_index = args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(args[seek + 1], seconds_argument(10.0));
        assert!(seek < input_index, "fast seeks use input-side seeking");

        let mut long_gop = rest[0].0.clone();
        long_gop.all_intra = false;
        let args = input(&long_gop, rest[0].1, rest[0].2, false);
        let fast_seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let fast_input_index = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(
            fast_seek < fast_input_index,
            "long-GOP playback first tries fast index seeking"
        );

        let args = input(&long_gop, rest[0].1, rest[0].2, true);
        let seek = args.iter().position(|arg| arg == "-ss").unwrap();
        let input_index = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(
            input_index < seek,
            "long-GOP footage must seek after its input"
        );
    }
    #[test]
    fn long_gop_footage_asks_the_hardware_first_then_software() {
        let take = |all_intra| NativeSegment {
            segment: Segment {
                path: "take.mp4".into(),
                start: 0.0,
                end: 60.0,
                offset: 0.0,
            },
            width: 3840,
            height: 2160,
            has_audio: true,
            all_intra,
        };
        let hardware = Attempt {
            hwaccel: Some("videotoolbox"),
            output_seek: false,
        };
        let software = Attempt {
            hwaccel: None,
            output_seek: false,
        };
        let from_start = Attempt {
            hwaccel: None,
            output_seek: true,
        };
        assert_eq!(
            attempts(&take(false), 12.0, Some("videotoolbox")),
            [hardware, software, from_start]
        );
        assert_eq!(
            attempts(&take(false), 0.0, Some("videotoolbox")),
            [hardware, software]
        );
        assert_eq!(attempts(&take(false), 12.0, None), [software, from_start]);
        // MJPEG: no hardware decoder takes it, and every frame is a keyframe.
        assert_eq!(
            attempts(&take(true), 12.0, Some("videotoolbox")),
            [software]
        );

        let args = input_through(&take(false), 12.0, 1.0, hardware);
        let accel = args.iter().position(|arg| arg == "-hwaccel").unwrap();
        let input_index = args.iter().position(|arg| arg == "-i").unwrap();
        assert!(accel < input_index && args[accel + 1] == "videotoolbox");
        assert!(
            !input_through(&take(false), 12.0, 1.0, software).contains(&"-hwaccel".to_string())
        );
    }

    #[test]
    fn plays_in_software_when_the_hardware_decoder_is_refused() {
        // Linux's FFmpeg has no VideoToolbox: the first attempt fails outright,
        // as it would on a Mac whose decoder refuses a recording.
        let part = NativeSegment {
            segment: Segment {
                path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/h264-initial-headers.avi"),
                start: 0.0,
                end: 3.0,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: false,
            all_intra: false,
        };
        let refused = if cfg!(target_os = "macos") {
            "vaapi"
        } else {
            "videotoolbox"
        };
        // From the start: no "decode from the start" attempt hides a missing fallback.
        let player = Player::with_hwaccel(
            vec![part],
            test_tools(),
            0.0,
            eframe::egui::Context::default(),
            Some(refused),
        );
        player.pause(true);
        let deadline = Instant::now() + Duration::from_secs(10);
        while player.frame().is_none() {
            assert!(
                Instant::now() < deadline,
                "no frame: {:?}",
                player.warnings()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(player.warnings().is_empty(), "{:?}", player.warnings());
    }

    #[test]
    fn turning_the_sound_up_starts_the_audio_a_muted_opening_left_out() {
        let part = NativeSegment {
            segment: Segment {
                path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/h264-initial-headers.avi"),
                start: 0.0,
                end: 3.0,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: true,
            all_intra: false,
        };
        let mut player = Player::new_with_volume(
            vec![part.clone()],
            test_tools(),
            1.0,
            eframe::egui::Context::default(),
            0.0,
        );
        player.pause(true);
        assert!(!player.audio_enabled());
        player.set_volume(0.8);
        assert!(player.audio_enabled(), "the sound comes back");
        assert_eq!(player.volume(), 0.8);
        assert!(player.paused(), "still paused where it was");
        assert_eq!(player.position(), 1.0);

        // A flight with no sound at all has nothing to start.
        let silent = NativeSegment {
            has_audio: false,
            ..part
        };
        let mut player = Player::new_with_volume(
            vec![silent],
            test_tools(),
            0.0,
            eframe::egui::Context::default(),
            0.0,
        );
        player.set_volume(0.8);
        assert!(!player.audio_enabled());
    }

    #[test]
    fn says_which_chapter_plays() {
        let part = |offset: f64, length: f64| NativeSegment {
            segment: Segment {
                path: "GH010042.MP4".into(),
                start: 0.0,
                end: length,
                offset,
            },
            width: 1920,
            height: 1080,
            has_audio: true,
            all_intra: false,
        };
        assert_eq!(
            part_at(&[part(0.0, 60.0)], 10.0),
            None,
            "one file: nothing to say"
        );
        let chapters = [part(0.0, 60.0), part(60.0, 30.0)];
        assert_eq!(part_at(&chapters, 0.0), Some((1, 2)));
        assert_eq!(part_at(&chapters, 59.9), Some((1, 2)));
        assert_eq!(
            part_at(&chapters, 60.0),
            Some((2, 2)),
            "the cut is the next chapter's"
        );
        assert_eq!(part_at(&chapters, 999.0), Some((2, 2)));
    }

    #[test]
    fn corrupt_seek_input_cannot_panic_the_clock() {
        assert_eq!(bounded_time(f64::NAN, 10.0), 0.0);
        assert_eq!(bounded_time(-10.0, 10.0), 0.0);
        assert_eq!(bounded_time(20.0, 10.0), 10.0);
    }

    fn test_tools() -> Option<PathBuf> {
        std::env::var_os("FPV_TEST_BUNDLED_TOOLS").map(PathBuf::from)
    }

    #[test]
    fn h264_indexed_seek_recovers_headers_without_replaying_the_take() {
        // This AVI stores SPS/PPS only in its first packet, like EV300D DVRs.
        // A minimal FFmpeg without extract_extradata cannot decode after a seek.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/h264-initial-headers.avi");
        let part = NativeSegment {
            segment: Segment {
                path,
                start: 1.0,
                end: 3.0,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: false,
            all_intra: false,
        };
        let tools = test_tools();
        let frame = |output_seek| {
            let mut args = input(&part, 0.52, 1.0, output_seek);
            args.extend([
                "-an".into(),
                "-vf".into(),
                "fps=30,scale=64:48".into(),
                "-frames:v".into(),
                "1".into(),
                "-pix_fmt".into(),
                "rgb24".into(),
                "-f".into(),
                "rawvideo".into(),
                "pipe:1".into(),
            ]);
            let output = FfmpegTool::Ffmpeg
                .command(tools.as_deref())
                .args(&args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "indexed seek must decode without a slow replay: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout.len(), 64 * 48 * 3);
            output.stdout
        };
        let expected = frame(true);
        assert_eq!(frame(false), expected, "seek must return the source frame");
        let player = Player::new(vec![part], tools, 0.52, eframe::egui::Context::default());
        player.pause(true);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(frame) = player.frame() {
                assert_eq!(frame.rgb, expected);
                assert!(!player.buffering());
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", player.warnings());
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        assert!(player.state.frames.lock().unwrap().len() <= 4);
        assert_eq!(player.position(), 0.52);
    }

    #[test]
    fn muted_playback_stays_muted_after_seeking_without_starting_audio() {
        let part = NativeSegment {
            segment: Segment {
                path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/h264-initial-headers.avi"),
                start: 0.0,
                end: 3.0,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: true,
            all_intra: false,
        };
        let mut player = Player::new_with_volume(
            vec![part],
            test_tools(),
            0.0,
            eframe::egui::Context::default(),
            0.0,
        );
        assert!(!player.state.audio.load(Ordering::Relaxed));
        player.pause(true);
        player.seek(1.0);
        assert!(!player.state.audio.load(Ordering::Relaxed));
        assert_eq!(*player.state.volume.lock().unwrap(), 0.0);
        assert!(player.paused());
    }

    #[test]
    fn decodes_source_frames_seeks_while_paused_and_reaps_children() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("PICT0001.AVI");
        let tools = test_tools();
        let status = FfmpegTool::Ffmpeg
            .command(tools.as_deref())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:rate=25:duration=2",
                "-c:v",
                "mjpeg",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let parts = vec![NativeSegment {
            segment: Segment {
                path,
                start: 0.5,
                end: 1.5,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: false,
            all_intra: true,
        }];
        let mut player = Player::new_source_preview(
            parts,
            tools,
            0.0,
            eframe::egui::Context::default(),
            0.0,
            25.0,
        );
        player.pause(true);
        let wait_frame = |player: &Player| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(frame) = player.frame() {
                    return frame;
                }
                assert!(
                    Instant::now() < deadline,
                    "no decoded frame: {:?}",
                    player.warnings()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let first = wait_frame(&player);
        assert_eq!(
            (first.width, first.height, first.rgb.len()),
            (64, 48, 64 * 48 * 3)
        );
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(player.position(), 0.0, "pause must freeze the clock");
        player.seek(0.5);
        let sought = wait_frame(&player);
        assert!((sought.at - 0.5).abs() < 0.04);
        assert!(player.paused());
        assert_ne!(first.rgb, sought.rgb);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let frames = player.state.frames.lock().unwrap();
            if frames.len() >= 2 {
                assert!(
                    (frames[1].at - frames[0].at - 1.0 / 25.0).abs() < 0.000001,
                    "boundary previews must keep source cadence across seeks"
                );
                break;
            }
            drop(frames);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let state = player.state.clone();
        drop(player);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stopped = state
                .children
                .lock()
                .unwrap()
                .iter()
                .all(|child| child.lock().unwrap().try_wait().unwrap().is_some());
            if stopped {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "closing the player must reap ffmpeg"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn decodes_pcm_audio_and_pads_silent_chapters_without_an_audio_device() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sound.avi");
        let tools = test_tools();
        let status = FfmpegTool::Ffmpeg
            .command(tools.as_deref())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x48:duration=0.2",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=0.2:sample_rate=8000",
                "-c:v",
                "mjpeg",
                "-c:a",
                "pcm_s16le",
                "-shortest",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let mut parts = vec![NativeSegment {
            segment: Segment {
                path,
                start: 0.0,
                end: 0.2,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: false,
            all_intra: true,
        }];
        // No CPAL device is opened in this test: inspect the decoded sample queue.
        let player = Player::new(
            parts.clone(),
            tools.clone(),
            0.0,
            eframe::egui::Context::default(),
        );
        decode_audio(&player.state, &parts, tools.clone(), 8000, 2).unwrap();
        let silence: Vec<_> = player.state.samples.lock().unwrap().drain(..).collect();
        assert_eq!(silence.len(), 3200);
        assert!(silence.iter().all(|sample| *sample == 0.0));
        parts[0].has_audio = true;
        decode_audio(&player.state, &parts, tools, 8000, 2).unwrap();
        let samples = player.state.samples.lock().unwrap();
        assert_eq!(samples.len(), 3200);
        assert!(samples.iter().any(|sample| sample.abs() > 0.01));
        assert!(samples.iter().all(|sample| sample.is_finite()));
    }
}
