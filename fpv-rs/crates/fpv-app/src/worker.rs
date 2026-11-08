//! One owner for the library. Disk and ffmpeg work never run on the UI thread.
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use fpv_library::{
    CacheUsage, Card, Config, FlightDeletion, FolderView, Library, NativeSegment, ReducePreview,
    RelocationPreview, SessionView, Step, Thumbnail, detect_cards, ffmpeg_available,
    tool_directory,
};

pub type Work =
    Box<dyn FnOnce(&mut Library, &Progress) -> Result<Reply, fpv_library::LibraryError> + Send>;

#[derive(Debug)]
pub enum Reply {
    Done(String),
    Export(BackgroundWork),
    SessionCreated(String),
    FlightsTransferred(String),
    Playback {
        flight: String,
        at: f64,
        parts: Vec<NativeSegment>,
        playback_mute: Option<bool>,
    },
    BoundarySource {
        clip: String,
        source: NativeSegment,
    },
    Image {
        key: String,
        thumbnail: Thumbnail,
    },
    BoundaryStrip {
        clip: String,
        index: usize,
        thumbnail: Thumbnail,
    },
    Folders(Vec<FolderView>),
    FlightInfo(Vec<fpv_library::RecordingInfo>),
    Reduce(ReducePreview),
    Relocation(RelocationPreview),
    Deletion(FlightDeletion),
    Cache(CacheUsage),
    VideoPreferences {
        mute_videos: bool,
        do_not_show_splash: bool,
    },
    Picked(Option<PathBuf>),
    Strips(fpv_library::StripsPrepared),
    /// A lossless export met recordings at different rates: the library's
    /// reason, for the dialog to ask how instead.
    Mismatch(String),
    NeedsReencode(String),
}

/// Prepared work owns snapshots and source leases, never the database.
type ExportWork = Box<dyn FnOnce(&Progress) -> Result<Reply, fpv_library::LibraryError> + Send>;
pub struct BackgroundWork(pub ExportWork);

impl std::fmt::Debug for BackgroundWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prepared export")
    }
}

pub struct Job {
    pub id: u64,
    pub work: Work,
}

pub enum Event {
    Ready {
        sessions: Vec<SessionView>,
        config: Config,
        config_directory: PathBuf,
        cache_directory: PathBuf,
        tools: Option<PathBuf>,
        ffmpeg_ok: bool,
    },
    Progress {
        id: u64,
        step: Step,
    },
    Finished {
        id: u64,
        result: Result<Reply, String>,
    },
    Snapshot(Vec<SessionView>, Config),
    Cards(Vec<Card>),
    Fatal(String),
}

pub struct Progress {
    id: u64,
    sender: Sender<Event>,
    context: egui::Context,
}

impl Progress {
    #[cfg(test)]
    pub(crate) fn for_test(id: u64, context: egui::Context) -> (Self, Receiver<Event>) {
        let (sender, events) = mpsc::channel();
        (
            Self {
                id,
                sender,
                context,
            },
            events,
        )
    }

    pub fn step(&self, step: Step) {
        let _ = self.sender.send(Event::Progress { id: self.id, step });
        self.context.request_repaint();
    }
    pub fn count(&self, done: usize, total: usize, line: impl Into<String>) {
        self.step(Step {
            done,
            total,
            line: line.into(),
        });
    }
}

pub struct Worker {
    pub sender: Sender<Job>,
    pub events: Receiver<Event>,
}

impl Worker {
    fn run_jobs(mut library: Library, jobs: Receiver<Job>, tx: Sender<Event>, ctx: egui::Context) {
        // Keep the existing one-export-at-a-time resource policy, independent of library reads.
        let (exports, pending) = mpsc::channel::<(u64, BackgroundWork)>();
        let output = tx.clone();
        let context = ctx.clone();
        std::thread::spawn(move || {
            for (id, work) in pending {
                let progress = Progress {
                    id,
                    sender: output.clone(),
                    context: context.clone(),
                };
                progress.count(0, 0, "Exporting…");
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (work.0)(&progress)))
                        .map_err(|_| {
                            "The export stopped unexpectedly; unfinished output was discarded."
                                .to_string()
                        })
                        .and_then(|r| r.map_err(|error| error.to_string()));
                if output.send(Event::Finished { id, result }).is_err() {
                    break;
                }
                context.request_repaint();
            }
        });
        for job in jobs {
            let progress = Progress {
                id: job.id,
                sender: tx.clone(),
                context: ctx.clone(),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (job.work)(&mut library, &progress)
            }))
            .map_err(|_| {
                "The operation stopped unexpectedly; the library transaction was rolled back."
                    .to_string()
            })
            .and_then(|result| result.map_err(|error| error.to_string()));
            if let Ok(Reply::Export(work)) = result {
                if exports.send((job.id, work)).is_err() {
                    let _ = tx.send(Event::Finished {
                        id: job.id,
                        result: Err("The export worker is unavailable.".into()),
                    });
                    ctx.request_repaint();
                }
                continue;
            }
            let _ = tx.send(Event::Finished { id: job.id, result });
            match library.snapshot() {
                Ok(snapshot) => {
                    let _ = tx.send(Event::Snapshot(snapshot, library.config().clone()));
                }
                Err(error) => {
                    let _ = tx.send(Event::Fatal(error.to_string()));
                }
            }
            ctx.request_repaint();
        }
    }

    pub fn start(context: egui::Context) -> Self {
        let (sender, jobs) = mpsc::channel::<Job>();
        let (events, receiver) = mpsc::channel();
        let tx = events.clone();
        let ctx = context.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let resources = resource_directory();
                let tools = tool_directory(resources.as_deref());
                let env = |name: &str| std::env::var_os(name);
                let config_directory =
                    first_set(CONFIG_VARIABLES, env).unwrap_or_else(Config::default_directory);
                let cache_directory =
                    first_set(CACHE_VARIABLES, env).unwrap_or_else(Config::default_cache_directory);
                let mut config = Config::load_or_create(&config_directory)?;
                if let Some(root) = first_set(MEDIA_VARIABLES, env) {
                    config.media_root = root;
                }
                let library =
                    Library::open_with(&config_directory, &cache_directory, config, tools.clone())?;
                let sessions = library.snapshot()?;
                let _ = tx.send(Event::Ready {
                    sessions,
                    config: library.config().clone(),
                    config_directory: library.config_directory.clone(),
                    cache_directory: library.cache_directory.clone(),
                    ffmpeg_ok: ffmpeg_available(tools.as_deref()),
                    tools,
                });
                Ok::<_, fpv_library::LibraryError>(library)
            })();
            ctx.request_repaint();
            let library = match result {
                Ok(library) => library,
                Err(error) => {
                    let _ = tx.send(Event::Fatal(error.to_string()));
                    ctx.request_repaint();
                    return;
                }
            };
            Self::run_jobs(library, jobs, tx, ctx);
        });
        // Card polling is independent of long-running imports and segmentation.
        std::thread::spawn(move || {
            while events.send(Event::Cards(detect_cards())).is_ok() {
                context.request_repaint();
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        });
        Self {
            sender,
            events: receiver,
        }
    }
}

/// The same layout in portable archives, app bundles and installed Linux packages.
pub fn resource_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let candidates = [
        directory.join("resources"),
        directory.join("../Resources"),
        directory.join("../lib/fpv-rush"),
        directory.to_path_buf(),
    ];
    candidates
        .into_iter()
        .find(|path| path.join("bin").is_dir())
}

/// One run's folders, overridden by these variables: the native app's names
/// first, then the Tauri app's, which scripts and notes still use.
const CONFIG_VARIABLES: &[&str] = &["FPV_CONFIG_DIR", "FPV_RUSH_CONFIG"];
const CACHE_VARIABLES: &[&str] = &["FPV_CACHE_DIR", "FPV_RUSH_CACHE"];
const MEDIA_VARIABLES: &[&str] = &["FPV_MEDIA_ROOT", "FPV_RUSH_LIBRARY"];

/// The first of `names` set to something, as a path.
fn first_set(
    names: &[&str],
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|name| lookup(name).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tauri_apps_variables_still_point_a_run_at_its_folders() {
        let set = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.into())
            }
        };
        assert_eq!(
            first_set(CONFIG_VARIABLES, set(&[("FPV_RUSH_CONFIG", "/tmp/old")])),
            Some(PathBuf::from("/tmp/old"))
        );
        assert_eq!(
            first_set(
                CACHE_VARIABLES,
                set(&[
                    ("FPV_RUSH_CACHE", "/tmp/old"),
                    ("FPV_CACHE_DIR", "/tmp/new")
                ])
            ),
            Some(PathBuf::from("/tmp/new")),
            "the native name wins"
        );
        assert_eq!(
            first_set(MEDIA_VARIABLES, set(&[("FPV_MEDIA_ROOT", "")])),
            None,
            "an empty variable is no override"
        );
    }
}

#[cfg(test)]
mod export_scheduling_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_held_export_does_not_block_playback_preparation_or_start_a_second_export() {
        let root = tempfile::tempdir().unwrap();
        let library = Library::open_with(
            &root.path().join("config"),
            &root.path().join("cache"),
            Config {
                media_root: root.path().join("videos"),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let (jobs, input) = mpsc::channel();
        let (events, output) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            Worker::run_jobs(library, input, events, egui::Context::default())
        });
        let (release, held) = mpsc::channel::<()>();
        jobs.send(Job {
            id: 1,
            work: Box::new(move |_, _| {
                Ok(Reply::Export(BackgroundWork(Box::new(move |progress| {
                    progress.count(0, 1, "Export running");
                    let _ = held.recv();
                    Ok(Reply::Done("Export complete".into()))
                }))))
            }),
        })
        .unwrap();
        loop {
            if matches!(
                output.recv_timeout(Duration::from_secs(10)).unwrap(),
                Event::Progress { id: 1, .. }
            ) {
                break;
            }
        }
        jobs.send(Job {
            id: 2,
            work: Box::new(|_, _| {
                Ok(Reply::Export(BackgroundWork(Box::new(|_| {
                    Ok(Reply::Done("Second export complete".into()))
                }))))
            }),
        })
        .unwrap();
        jobs.send(Job {
            id: 3,
            work: Box::new(|_, _| {
                Ok(Reply::Playback {
                    flight: "next".into(),
                    at: 0.0,
                    parts: vec![],
                    playback_mute: Some(true),
                })
            }),
        })
        .unwrap();
        loop {
            match output.recv_timeout(Duration::from_secs(10)).unwrap() {
                Event::Finished { id: 1 | 2, .. } => {
                    panic!("the held export and queued export must not have completed")
                }
                Event::Finished {
                    id: 3,
                    result:
                        Ok(Reply::Playback {
                            playback_mute: Some(true),
                            ..
                        }),
                } => break,
                _ => {}
            }
        }
        drop(release);
        let mut finished = vec![];
        while finished.len() < 2 {
            if let Event::Finished { id, result } =
                output.recv_timeout(Duration::from_secs(10)).unwrap()
            {
                assert!(result.is_ok());
                finished.push(id);
            }
        }
        assert_eq!(finished, [1, 2], "exports preserve FIFO execution");
        drop(jobs);
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod real_export_playback_tests {
    use super::*;
    use fpv_library::{ExportEncoding, ExportMode};
    use fpv_media::{FfmpegRunner, FfmpegTool, MediaProbe};
    use std::time::{Duration, Instant};

    #[test]
    fn real_exports_and_native_decoding_complete_concurrently() {
        let root = tempfile::tempdir().unwrap();
        let card = root.path().join("card");
        std::fs::create_dir(&card).unwrap();
        let tools = std::env::var_os("FPV_TEST_BUNDLED_TOOLS").map(PathBuf::from);
        assert!(
            FfmpegTool::Ffmpeg
                .command(tools.as_deref())
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=640x480:rate=25:duration=4",
                    "-c:v",
                    "mpeg4",
                    "-q:v",
                    "4",
                    "-y"
                ])
                .arg(card.join("GH010001.MP4"))
                .status()
                .unwrap()
                .success()
        );
        let library = Library::open_with(
            &root.path().join("config"),
            &root.path().join("cache"),
            Config {
                media_root: root.path().join("videos"),
                ..Default::default()
            },
            tools.clone(),
        )
        .unwrap();
        library
            .import(&card, "Concurrent", None, false, |_| {})
            .unwrap();
        let flight = library.snapshot().unwrap()[0].flights[0].id.clone();
        let (jobs, input) = mpsc::channel();
        let (events, output) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            Worker::run_jobs(library, input, events, egui::Context::default())
        });
        for (index, encoding, bulk) in [
            (0, ExportEncoding::AsIs, false),
            (1, ExportEncoding::ProRes422Hq, false),
            (2, ExportEncoding::ProRes422Hq, true),
        ] {
            let destination = root.path().join(format!(
                "output-{index}.{}",
                if encoding == ExportEncoding::AsIs {
                    "mp4"
                } else {
                    "mov"
                }
            ));
            let id = flight.clone();
            let path = destination.clone();
            let export = index * 2 + 1;
            jobs.send(Job {
                id: export,
                work: Box::new(move |library, _| {
                    if bulk {
                        let plan = library.prepare_exports(&[id])?;
                        Ok(Reply::Export(BackgroundWork(Box::new(move |progress| {
                            plan.write(
                                &path.with_extension("batch"),
                                ExportMode::Parts,
                                encoding,
                                |done, total, line| progress.count(done, total, line),
                            )?;
                            Ok(Reply::Done("Batch exported".into()))
                        }))))
                    } else {
                        let plan = library.prepare_export(&id)?;
                        Ok(Reply::Export(BackgroundWork(Box::new(move |_| {
                            plan.write(&path, ExportMode::Lossless, encoding)?;
                            Ok(Reply::Done("Exported".into()))
                        }))))
                    }
                }),
            })
            .unwrap();
            loop {
                if matches!(output.recv_timeout(Duration::from_secs(10)).unwrap(), Event::Progress { id, .. } if id == export)
                {
                    break;
                }
            }
            let start = Instant::now();
            let id = flight.clone();
            jobs.send(Job {
                id: export + 1,
                work: Box::new(move |library, _| {
                    Ok(Reply::Playback {
                        parts: library.native_playback(&id)?,
                        playback_mute: library.playback_mute(&id)?,
                        flight: id,
                        at: 0.0,
                    })
                }),
            })
            .unwrap();
            let mut finished = false;
            let mut export_ms = None;
            let parts = loop {
                match output.recv_timeout(Duration::from_secs(10)).unwrap() {
                    Event::Finished { id, result } if id == export => {
                        result.unwrap();
                        finished = true;
                        export_ms = Some(start.elapsed().as_millis());
                    }
                    Event::Finished {
                        id,
                        result: Ok(Reply::Playback { parts, .. }),
                    } if id == export + 1 => break parts,
                    _ => {}
                }
            };
            let prepared_ms = start.elapsed().as_millis();
            let player = crate::player::Player::new_source_preview(
                parts,
                tools.clone(),
                0.0,
                egui::Context::default(),
                0.0,
                25.0,
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(frame) = player.frame() {
                    assert!(!frame.rgb.is_empty());
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "native frame: {:?}",
                    player.warnings()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let frame_ms = start.elapsed().as_millis();
            drop(player);
            while !finished {
                if let Event::Finished { id, result } =
                    output.recv_timeout(Duration::from_secs(10)).unwrap()
                    && id == export
                {
                    result.unwrap();
                    finished = true;
                    export_ms = Some(start.elapsed().as_millis());
                }
            }
            if !bulk {
                assert!(
                    MediaProbe::new(&FfmpegRunner::new(tools.clone()))
                        .probe(&destination)
                        .unwrap()
                        .duration
                        > 0.0
                );
            }
            eprintln!(
                "encoding={encoding:?} batch={bulk}: prepare={prepared_ms}ms first-frame={frame_ms}ms export-result-observed={export_ms:?}ms"
            );
        }
        drop(jobs);
        worker.join().unwrap();
    }
}
