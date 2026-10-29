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
            let mut library = match result {
                Ok(library) => library,
                Err(error) => {
                    let _ = tx.send(Event::Fatal(error.to_string()));
                    ctx.request_repaint();
                    return;
                }
            };
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
