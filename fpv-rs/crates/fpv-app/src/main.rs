//! Native desktop front end. All domain actions go through fpv-library.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(any(target_os = "macos", test))]
mod about_menu;
mod boundary;
mod browse;
mod cloudwait;
mod dialogs;
mod images;
mod notices;
mod player;
mod strips;
mod taskbar;
mod ui;
mod worker;

use browse::{Browse, flight_name};
use dialogs::Dialog;
use eframe::egui;
use fpv_library::{Card, Config, FlightView, Library, LibraryError, SessionView};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
use worker::{Event, Job, Progress, Reply, Worker};

/// What a task changes, so only what conflicts with it waits: the library as
/// a whole (an import, the import folder), one session (its flights remade or
/// moved), or nothing anyone else depends on (a like, a poster, an export).
/// The worker runs every job in turn anyway; the scope is about meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    None,
    Library,
    Session(String),
}

/// What Escape closes: the topmost thing open, one press at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// A pop-up menu, which egui closes itself.
    Menu,
    /// Full screen, which the player leaves itself.
    FullScreen,
    /// The cloud wait: Continue in Background.
    CloudWait,
    Dialog,
    Player,
    /// The task and notification lists.
    Panels,
    Nothing,
}

#[derive(Default)]
struct Open {
    menu: bool,
    full_screen: bool,
    cloud_wait: bool,
    dialog: bool,
    player: bool,
    panels: bool,
}

fn escape_target(open: Open) -> Escape {
    if open.menu {
        Escape::Menu
    } else if open.full_screen {
        Escape::FullScreen
    } else if open.cloud_wait {
        Escape::CloudWait
    } else if open.dialog {
        Escape::Dialog
    } else if open.player {
        Escape::Player
    } else if open.panels {
        Escape::Panels
    } else {
        Escape::Nothing
    }
}

struct Task {
    id: u64,
    scope: Scope,
    label: String,
    line: String,
    done: usize,
    total: usize,
    running: bool,
    failed: bool,
    quiet: bool,
    log: Vec<String>,
    started: std::time::Instant,
    ended: Option<std::time::Instant>,
}

#[derive(Default)]
struct PlayerWindow {
    maximized: bool,
    normal_rect: Option<egui::Rect>,
    was_expanded: bool,
}

struct Viewer {
    flight: FlightView,
    player: player::Player,
    texture: Option<egui::TextureHandle>,
    mark: Option<f64>,
    figure: String,
    label: String,
    volume: f32,
    remembered_volume: f32,
    playback_muted: bool,
    muted: bool,
    fullscreen: bool,
    window: PlayerWindow,
    seek_time: Option<f64>,
}

pub(crate) struct App {
    worker: Worker,
    sessions: Vec<SessionView>,
    cards: Vec<Card>,
    config: Config,
    config_directory: PathBuf,
    cache_directory: PathBuf,
    tools: Option<PathBuf>,
    ffmpeg_ok: bool,
    ready: bool,
    browse: Browse,
    tasks: Vec<Task>,
    next_id: u64,
    dialog: Option<Dialog>,
    dialog_request: Option<u64>,
    viewer: Option<Viewer>,
    /// A flight waiting for its recordings to come down before it plays.
    cloud_wait: Option<cloudwait::CloudWait>,
    /// Downloads left to finish in the background: their flight does not open.
    detached: HashSet<u64>,
    /// Hover strips made ahead of the pointer, and the batch running now.
    strips: strips::Strips,
    strip_batch: Option<u64>,
    /// A card's title being edited in place: (flight, text so far).
    title_edit: Option<(String, String)>,
    /// The session the sidebar's page last followed (`browse::follow`).
    followed_session: Option<String>,
    images: images::Images,
    thumbnails: HashMap<String, String>,
    requested: HashSet<String>,
    selected: HashSet<String>,
    selecting: bool,
    search: String,
    notices: notices::Notices,
    show_notices: bool,
    show_tasks: bool,
    exit_after: Option<std::time::Instant>,
    screenshot_requested: bool,
    splash_started: Option<std::time::Instant>,
    splash_texture: egui::TextureHandle,
    show_about: bool,
    #[cfg(target_os = "macos")]
    about_menu: Option<about_menu::AboutMenu>,
}

pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(255, 91, 46);
/// What deletes: the fill of a button that removes something for good.
pub(crate) const DANGER: egui::Color32 = egui::Color32::from_rgb(196, 48, 43);
/// The same, as text on the dark background (a menu item, a small "×").
pub(crate) const DANGER_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 107, 99);

/// A button that deletes: red, so it is never pressed by mistake.
pub(crate) fn danger(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.into()).color(egui::Color32::WHITE)).fill(DANGER)
}

/// A menu item or small control that deletes: red text.
pub(crate) fn danger_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).color(DANGER_TEXT)
}

/// A section's heading, in the accent.
pub(crate) fn section(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).heading().color(ACCENT)
}

fn splash_image() -> image::RgbaImage {
    image::load_from_memory(include_bytes!(
        "../../../../docs/logo-letz_kwad-full-transparent.png"
    ))
    .expect("bundled splash logo is a valid image")
    .thumbnail(512, 512)
    .to_rgba8()
}

fn configure_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    let mut style = (*ctx.style()).clone();
    // Room to breathe: between a checkbox or radio and its label, between
    // rows, inside buttons, windows and menus.
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.icon_spacing = 8.0;
    style.spacing.indent = 22.0;
    style.spacing.interact_size.y = 24.0;
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.visuals.panel_fill = egui::Color32::from_rgb(24, 26, 32);
    style.visuals.window_fill = egui::Color32::from_rgb(27, 29, 35);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(14, 16, 20);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(34, 36, 43);
    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(24, 26, 32);
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(39, 41, 48);
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(190, 67, 38);
    style.visuals.widgets.active.bg_fill = ACCENT;
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(220, 223, 230));
    style.visuals.weak_text_alpha = 0.65;
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(34, 36, 43);
    style.visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(48, 52, 62));
    style.visuals.widgets.noninteractive.bg_stroke = style.visuals.widgets.inactive.bg_stroke;
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(55, 58, 68);
    style.visuals.widgets.active.weak_bg_fill = ACCENT;
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));
    ctx.set_style(style);
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let mut browse: Browse = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, "browse"))
            .unwrap_or_default();
        browse.page_size = browse.page_size.clamp(12, 96);
        let exit_after = std::env::var("FPV_SMOKE_SECONDS")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .map(|seconds| std::time::Instant::now() + Duration::from_secs(seconds));
        let splash_image = splash_image();
        let splash_pixels = egui::ColorImage::from_rgba_unmultiplied(
            [
                splash_image.width() as usize,
                splash_image.height() as usize,
            ],
            splash_image.as_raw(),
        );
        let splash_texture =
            cc.egui_ctx
                .load_texture("splash-logo", splash_pixels, egui::TextureOptions::LINEAR);
        Self {
            worker: Worker::start(cc.egui_ctx.clone()),
            sessions: vec![],
            cards: vec![],
            config: Config::default(),
            config_directory: PathBuf::new(),
            cache_directory: PathBuf::new(),
            tools: None,
            ffmpeg_ok: true,
            ready: false,
            browse,
            tasks: vec![],
            next_id: 1,
            dialog: None,
            dialog_request: None,
            viewer: None,
            cloud_wait: None,
            detached: HashSet::new(),
            strips: strips::Strips::default(),
            strip_batch: None,
            title_edit: None,
            followed_session: None,
            images: images::Images::new(cc.egui_ctx.clone()),
            thumbnails: HashMap::new(),
            requested: HashSet::new(),
            selected: HashSet::new(),
            selecting: false,
            search: String::new(),
            notices: notices::Notices::default(),
            show_notices: false,
            show_tasks: false,
            exit_after,
            screenshot_requested: false,
            splash_started: None,
            splash_texture,
            show_about: false,
            #[cfg(target_os = "macos")]
            about_menu: about_menu::AboutMenu::install(&cc.egui_ctx),
        }
    }

    fn submit(
        &mut self,
        label: impl Into<String>,
        quiet: bool,
        work: impl FnOnce(&mut Library, &Progress) -> Result<Reply, LibraryError> + Send + 'static,
    ) -> u64 {
        self.submit_in(Scope::None, label, quiet, work)
    }

    fn submit_in(
        &mut self,
        scope: Scope,
        label: impl Into<String>,
        quiet: bool,
        work: impl FnOnce(&mut Library, &Progress) -> Result<Reply, LibraryError> + Send + 'static,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(Task {
            id,
            scope,
            label: label.into(),
            quiet,
            log: vec![],
            line: "Queued".into(),
            done: 0,
            total: 0,
            running: true,
            failed: false,
            started: std::time::Instant::now(),
            ended: None,
        });
        if self
            .worker
            .sender
            .send(Job {
                id,
                work: Box::new(work),
            })
            .is_err()
        {
            self.notices
                .push("The library worker is unavailable. Restart the application.");
            if let Some(task) = self.tasks.last_mut() {
                task.running = false;
                task.ended = Some(std::time::Instant::now());
                task.failed = true;
            }
        }
        id
    }
    fn busy(&self) -> bool {
        self.tasks.iter().any(|task| task.running && !task.quiet)
    }

    /// A task of this scope runs: what conflicts with it waits.
    fn busy_with(&self, scope: &Scope) -> bool {
        *scope != Scope::None
            && self
                .tasks
                .iter()
                .any(|task| task.running && !task.quiet && task.scope == *scope)
    }

    fn session_busy(&self, session: &str) -> bool {
        self.busy_with(&Scope::Session(session.to_owned()))
    }

    fn session_of_flight(&self, flight: &str) -> Option<&str> {
        self.sessions
            .iter()
            .find(|session| session.flights.iter().any(|f| f.id == flight))
            .map(|session| session.id.as_str())
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                Event::Ready {
                    sessions,
                    config,
                    config_directory,
                    cache_directory,
                    tools,
                    ffmpeg_ok,
                } => {
                    self.sessions = sessions;
                    self.config = config;
                    if !self.config.do_not_show_splash {
                        self.splash_started = Some(std::time::Instant::now());
                    }
                    self.config_directory = config_directory;
                    self.cache_directory = cache_directory;
                    self.tools = tools;
                    self.ffmpeg_ok = ffmpeg_ok;
                    self.ready = true;
                    self.submit("Identify cameras", true, |library, _| {
                        Ok(Reply::Done(format!(
                            "Identified {} camera(s)",
                            library.identify_cameras()?
                        )))
                    });
                    // Also covers a cache emptied since the last run.
                    self.request_strips();
                }
                Event::Progress { id, step } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                        task.line = step.line;
                        if task.log.last() != Some(&task.line) {
                            task.log.push(task.line.clone());
                            if task.log.len() > 200 {
                                task.log.remove(0);
                            }
                        }
                        task.done = step.done;
                        task.total = step.total;
                    }
                }
                Event::Finished { id, result } => {
                    if self.cloud_wait.as_ref().is_some_and(|wait| wait.task == id) {
                        self.cloud_wait = None;
                    }
                    let detached = self.detached.remove(&id);
                    let strip_batch = self.strip_batch == Some(id);
                    if strip_batch {
                        self.strip_batch = None;
                    }
                    // Whatever the pilot ran may have made or re-keyed flights:
                    // their strips are made before the pointer asks.
                    let changed = !strip_batch
                        && result.is_ok()
                        && self.tasks.iter().any(|task| task.id == id && !task.quiet);
                    // A refused lossless export keeps its dialog open, to ask how.
                    let done = matches!(&result, Ok(reply) if !matches!(reply, Reply::Mismatch(_)));
                    if self
                        .dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.finished(id, done))
                    {
                        self.dialog = None;
                    }
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                        task.running = false;
                        task.ended = Some(std::time::Instant::now());
                        task.failed = result.is_err();
                        task.line = match &result {
                            Ok(Reply::Done(text)) => text.clone(),
                            Ok(_) => "Done".into(),
                            Err(error) => error.clone(),
                        };
                    }
                    match result {
                        Err(error) if strip_batch => {
                            self.strips.failed();
                            self.notices.push(format!("Hover strips: {error}"));
                        }
                        Err(error) => self.notices.push(error),
                        Ok(Reply::Mismatch(reason)) => {
                            if let Some(dialog) = &mut self.dialog {
                                dialog.reply(Reply::Mismatch(reason));
                            }
                        }
                        Ok(Reply::Strips(prepared)) => {
                            let (next, failed) =
                                self.strips
                                    .finished(prepared.made, prepared.more, prepared.failed);
                            for (flight, reason) in failed {
                                self.notices
                                    .push(format!("Hover strip of {flight}: {reason}"));
                            }
                            if let Some(first) = next {
                                self.strip_batch_start(first);
                            }
                        }
                        Ok(Reply::Done(_)) => {}
                        Ok(Reply::VideoPreferences {
                            mute_videos,
                            do_not_show_splash,
                        }) => {
                            self.config.mute_videos = mute_videos;
                            self.config.do_not_show_splash = do_not_show_splash;
                        }
                        // Downloaded in the background, as asked: nothing opens.
                        Ok(Reply::Playback { .. }) if detached => {}
                        Ok(Reply::Playback { flight, at, parts }) => {
                            if let Some(flight) = self
                                .sessions
                                .iter()
                                .flat_map(|s| &s.flights)
                                .find(|f| f.id == flight)
                            {
                                // No sound to play: no audio track, or imported
                                // with No Sound. Mute videos only starts muted:
                                // the speaker brings the sound back.
                                let silent = parts.iter().all(|part| !part.has_audio);
                                let muted = silent || self.config.mute_videos;
                                let player = player::Player::new_with_volume(
                                    parts,
                                    self.tools.clone(),
                                    at,
                                    ctx.clone(),
                                    if muted { 0.0 } else { 1.0 },
                                );
                                self.viewer = Some(Viewer {
                                    flight: flight.clone(),
                                    player,
                                    texture: None,
                                    mark: None,
                                    figure: String::new(),
                                    label: String::new(),
                                    volume: if silent { 0.0 } else { 1.0 },
                                    remembered_volume: 1.0,
                                    playback_muted: muted && !silent,
                                    muted: silent,
                                    fullscreen: false,
                                    window: PlayerWindow::default(),
                                    seek_time: None,
                                });
                            }
                        }
                        Ok(Reply::Image { key, thumbnail }) => {
                            self.thumbnails
                                .insert(key, thumbnail.path.to_string_lossy().into_owned());
                        }
                        Ok(reply) => {
                            if self.dialog_request == Some(id)
                                && let Some(dialog) = &mut self.dialog
                            {
                                dialog.reply(reply);
                            }
                        }
                    }
                    if changed {
                        self.request_strips();
                    }
                }
                Event::Snapshot(sessions, config) => {
                    self.sessions = sessions;
                    self.config = config;
                    if let Some(viewer) = &mut self.viewer {
                        if let Some(flight) = self
                            .sessions
                            .iter()
                            .flat_map(|s| &s.flights)
                            .find(|f| f.id == viewer.flight.id)
                        {
                            viewer.flight = flight.clone();
                        } else {
                            self.viewer = None;
                        }
                    }
                    self.selected.retain(|id| {
                        self.sessions
                            .iter()
                            .any(|s| s.flights.iter().any(|f| &f.id == id))
                    });
                }
                Event::Cards(cards) => self.cards = cards,
                Event::Fatal(error) => self.notices.push(error),
            }
        }
        if self.tasks.len() > 200 {
            let keep_from = self.tasks.len() - 100;
            let mut index = 0;
            self.tasks.retain(|task| {
                let keep = task.running || index >= keep_from;
                index += 1;
                keep
            });
        }
    }
    /// Escape closes the topmost thing open, as dialogs and menus always did.
    fn escape(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return;
        }
        let target = escape_target(Open {
            menu: egui::Popup::is_any_open(ctx),
            full_screen: self.viewer.as_ref().is_some_and(|viewer| viewer.fullscreen),
            cloud_wait: self.cloud_wait.is_some(),
            dialog: self.dialog.is_some(),
            player: self.viewer.is_some(),
            panels: self.show_tasks || self.show_notices,
        });
        match target {
            Escape::CloudWait => {
                if let Some(wait) = self.cloud_wait.take() {
                    self.detached.insert(wait.task);
                }
            }
            Escape::Dialog => {
                // A task it started goes on, in the task bar.
                self.dialog = None;
                self.dialog_request = None;
            }
            Escape::Player => self.viewer = None,
            Escape::Panels => {
                self.show_tasks = false;
                self.show_notices = false;
            }
            Escape::Menu | Escape::FullScreen | Escape::Nothing => {}
        }
    }

    /// Asks for the strips the library lacks, the session on screen first.
    fn request_strips(&mut self) {
        if !self.ffmpeg_ok {
            return;
        }
        if let Some(first) = self.strips.request(self.browse.session.clone()) {
            self.strip_batch_start(first);
        }
    }

    fn strip_batch_start(&mut self, first: Option<String>) {
        // Quiet: a batch must not mark the app busy, which disables forms.
        let id = self.submit("Hover strips", true, move |library, progress| {
            Ok(Reply::Strips(library.prepare_strips_up_to(
                strips::STRIP,
                first.as_deref(),
                strips::BATCH,
                |done, total, label| progress.count(done, total, label),
            )?))
        });
        self.strip_batch = Some(id);
    }

    fn play(&mut self, flight: &FlightView, at: f64) {
        self.viewer = None;
        let id = flight.id.clone();
        let task = self.submit(
            format!("Open {}", flight_name(flight)),
            false,
            move |library, progress| {
                let report = library.download_flight(&id, |step| {
                    progress.count(
                        if step.done == step.total {
                            step.files
                        } else {
                            step.file.saturating_sub(1)
                        },
                        step.files,
                        step.line,
                    );
                })?;
                if !report.failed.is_empty() {
                    return Err(LibraryError::Invalid(format!(
                        "Cloud download failed: {:?}",
                        report.failed
                    )));
                }
                Ok(Reply::Playback {
                    parts: library.native_playback(&id)?,
                    flight: id,
                    at,
                })
            },
        );
        if flight.cloud {
            self.cloud_wait = Some(cloudwait::CloudWait {
                task,
                flight: flight_name(flight),
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        #[cfg(target_os = "macos")]
        {
            if self.about_menu.is_none() {
                self.about_menu = about_menu::AboutMenu::install(ctx);
            }
            if let Some(menu) = &mut self.about_menu {
                if self.ready {
                    menu.smoke_test_if_requested();
                }
                if menu.take_request() {
                    self.show_about = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
        self.escape(ctx);
        self.task_bar(ctx);
        self.sidebar(ctx);
        self.browser(ctx);
        self.player_window(ctx);
        self.dialog_window(ctx);
        self.cloud_wait_window(ctx);
        self.splash_window(ctx);
        self.about_window(ctx);
        if let Some(path) = std::env::var_os("FPV_SMOKE_SCREENSHOT") {
            let capture_due = self.exit_after.is_none_or(|end| {
                end.saturating_duration_since(std::time::Instant::now()) <= Duration::from_secs(1)
            });
            if self.ready && !self.screenshot_requested && capture_due {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.screenshot_requested = true;
            }
            ctx.input(|input| {
                for event in &input.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        let pixels: Vec<u8> = image
                            .pixels
                            .iter()
                            .flat_map(|pixel| pixel.to_array())
                            .collect();
                        if let Err(error) = image::save_buffer(
                            &path,
                            &pixels,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        ) {
                            log::warn!("Could not save smoke screenshot: {error}");
                        }
                    }
                }
            });
        }
        if self
            .exit_after
            .is_some_and(|end| std::time::Instant::now() >= end)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.busy() || self.exit_after.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "browse", &self.browse);
    }
}

fn day(date: &Option<String>) -> String {
    date.as_ref()
        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

/// Shows a recording selected in the system's file manager — never opens it,
/// which would play it — else opens the folder it is in.
fn reveal(path: &std::path::Path) -> Result<(), LibraryError> {
    if cfg!(target_os = "windows") {
        // Explorer exits 1 even when it selected the file: its status says nothing.
        std::process::Command::new("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()?;
        return Ok(());
    }
    let (program, arguments) = reveal_command(path);
    match std::process::Command::new(&program)
        .args(&arguments)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        outcome => {
            log::debug!("{program} {arguments:?}: {outcome:?}; opening the folder instead");
            open_folder(path.parent().unwrap_or(path))
        }
    }
}

/// macOS selects with `open -R`; Linux asks the file manager over D-Bus, the
/// interface every desktop's file manager answers (Files, Dolphin, Thunar…).
fn reveal_command(path: &std::path::Path) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        return (
            "open".into(),
            vec!["-R".into(), path.to_string_lossy().into_owned()],
        );
    }
    (
        "dbus-send".into(),
        vec![
            "--session".into(),
            "--dest=org.freedesktop.FileManager1".into(),
            "--type=method_call".into(),
            "--print-reply".into(),
            "/org/freedesktop/FileManager1".into(),
            "org.freedesktop.FileManager1.ShowItems".into(),
            format!("array:string:{}", file_uri(path)),
            "string:".into(),
        ],
    )
}

/// `file://` and the path, every byte but unreserved ones and `/` escaped —
/// commas too, since dbus-send splits an array on them.
fn file_uri(path: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn open_folder(path: &std::path::Path) -> Result<(), LibraryError> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer.exe")
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(path).spawn()?;
    Ok(())
}

fn main() -> eframe::Result {
    env_logger::init();
    if std::env::args().any(|arg| arg == "--version") {
        println!("FPV Rush {} (native egui)", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let icon = image::load_from_memory(include_bytes!("../icons/128x128.png"))
        .ok()
        .map(|image| {
            let image = image.to_rgba8();
            egui::IconData {
                width: image.width(),
                height: image.height(),
                rgba: image.into_raw(),
            }
        });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("FPV Rush")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([900.0, 600.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "FPV Rush",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[cfg(test)]
mod escape_tests {
    use super::*;

    #[test]
    fn escape_closes_the_topmost_thing_open() {
        let all = || Open {
            menu: true,
            full_screen: true,
            cloud_wait: true,
            dialog: true,
            player: true,
            panels: true,
        };
        assert_eq!(escape_target(all()), Escape::Menu);
        assert_eq!(
            escape_target(Open {
                menu: false,
                ..all()
            }),
            Escape::FullScreen
        );
        let windowed = || Open {
            menu: false,
            full_screen: false,
            ..all()
        };
        assert_eq!(escape_target(windowed()), Escape::CloudWait);
        assert_eq!(
            escape_target(Open {
                cloud_wait: false,
                ..windowed()
            }),
            Escape::Dialog
        );
        let player = Open {
            cloud_wait: false,
            dialog: false,
            ..windowed()
        };
        assert_eq!(escape_target(player), Escape::Player);
        assert_eq!(
            escape_target(Open {
                panels: true,
                ..Open::default()
            }),
            Escape::Panels
        );
        assert_eq!(escape_target(Open::default()), Escape::Nothing);
    }
}

#[cfg(test)]
mod reveal_tests {
    use super::*;

    #[test]
    fn writes_a_file_uri_dbus_can_carry() {
        assert_eq!(
            file_uri(std::path::Path::new(
                "/home/pilot/Videos/FPV Rush/L'été, 1/GH010042.MP4"
            )),
            "file:///home/pilot/Videos/FPV%20Rush/L%27%C3%A9t%C3%A9%2C%201/GH010042.MP4"
        );
    }

    #[test]
    fn selects_the_recording_instead_of_opening_it() {
        let path = std::path::Path::new("/media/FPV Rush/PICT0032.AVI");
        let (program, arguments) = reveal_command(path);
        if cfg!(target_os = "macos") {
            assert_eq!((program.as_str(), arguments[0].as_str()), ("open", "-R"));
        } else {
            assert_eq!(program, "dbus-send");
            assert!(arguments.contains(&"org.freedesktop.FileManager1.ShowItems".to_string()));
            assert!(
                arguments
                    .contains(&"array:string:file:///media/FPV%20Rush/PICT0032.AVI".to_string())
            );
        }
    }
}

#[cfg(test)]
mod branding_tests {
    #[test]
    fn branded_logo_fits_the_software_renderer_and_retains_transparency() {
        let image = super::splash_image();
        assert!(image.width() > 0 && image.width() <= 512);
        assert!(image.height() > 0 && image.height() <= 512);
        assert!(image.pixels().any(|pixel| pixel[3] == 0));
        assert!(image.pixels().any(|pixel| pixel[3] == 255));
    }
}
