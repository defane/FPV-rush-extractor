//! Forms keep their drafts while operations run or fail. Confirmation previews
//! come from the same library methods which perform the destructive action.
use crate::{
    ACCENT, App, Reply, Scope,
    browse::{flight_name, labels},
};
use chrono::{Datelike, Months, NaiveDate};
use eframe::egui;
use fpv_library::{
    CacheKind, CacheUsage, Config, ExportMode, FlightDeletion, FlightView, FolderView,
    ImportProgress, ReducePreview, RelocationPreview, SegmentProgress, SessionEdit, SessionView,
};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Import,
    Settings,
    Session,
    Profile,
    Segment,
    RemoveSession,
    Reduce,
    Relocate,
    DeleteFlight,
    Title,
    Editor,
    Export,
    Bulk,
    Load,
    FreeSpace,
}

pub(crate) struct Dialog {
    kind: Kind,
    title: String,
    id: String,
    name: String,
    date: Option<NaiveDate>,
    calendar_month: NaiveDate,
    date_picker_open: bool,
    location: String,
    notes: String,
    labels: String,
    path: String,
    flag: bool,
    profile: String,
    mode: ExportMode,
    acknowledged: bool,
    flight: Option<FlightView>,
    /// The reason a lossless export was refused, while the dialog asks how.
    mismatch: Option<String>,
    /// The file name an export proposes: "<session> — <title>.avi".
    file_name: String,
    flights: Vec<String>,
    folders: Option<Vec<FolderView>>,
    chosen: HashSet<String>,
    reduction: Option<ReducePreview>,
    relocation: Option<RelocationPreview>,
    deletion: Option<FlightDeletion>,
    cache: Option<CacheUsage>,
    saving: Option<u64>,
    restore: HashSet<String>,
    budgets: [f64; 2],
    mute_videos: bool,
    do_not_show_splash: bool,
    editor_clip: usize,
    editor_loading: Option<String>,
    editor_source: Option<(String, fpv_library::NativeSegment)>,
    editor_preview: Option<crate::boundary::SourcePreview>,
}

impl Dialog {
    fn new(kind: Kind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            id: String::new(),
            name: String::new(),
            date: None,
            calendar_month: chrono::Local::now().date_naive().with_day(1).unwrap(),
            date_picker_open: false,
            location: String::new(),
            notes: String::new(),
            labels: String::new(),
            path: String::new(),
            flag: false,
            profile: "standard".into(),
            mode: ExportMode::Lossless,
            acknowledged: false,
            flight: None,
            mismatch: None,
            file_name: String::new(),
            flights: vec![],
            folders: None,
            chosen: HashSet::new(),
            reduction: None,
            relocation: None,
            deletion: None,
            cache: None,
            saving: None,
            restore: HashSet::new(),
            budgets: [10.0, 2.0],
            mute_videos: false,
            do_not_show_splash: false,
            editor_clip: 0,
            editor_loading: None,
            editor_source: None,
            editor_preview: None,
        }
    }
    pub fn import(folder: Option<PathBuf>, name: String) -> Self {
        let today = chrono::Local::now().date_naive();
        Self {
            date: Some(today),
            calendar_month: today.with_day(1).unwrap(),
            path: folder
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            name,
            ..Self::new(Kind::Import, "Import recordings")
        }
    }
    pub fn settings(config: &Config) -> Self {
        Self {
            path: config.media_root.to_string_lossy().into_owned(),
            name: config.folder_pattern.clone(),
            budgets: [config.proxy_budget_gb, config.picture_budget_gb],
            mute_videos: config.mute_videos,
            do_not_show_splash: config.do_not_show_splash,
            ..Self::new(Kind::Settings, "Settings")
        }
    }
    pub fn session(session: &SessionView) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            date: session
                .date
                .as_ref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Local).date_naive()),
            calendar_month: session
                .date
                .as_ref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Local).date_naive())
                .unwrap_or_else(|| chrono::Local::now().date_naive())
                .with_day(1)
                .unwrap(),
            location: session.location.clone().unwrap_or_default(),
            notes: session.notes.clone().unwrap_or_default(),
            labels: session.labels.join(", "),
            ..Self::new(Kind::Session, "Edit session")
        }
    }
    fn of_session(kind: Kind, title: &str, session: &SessionView) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            profile: session.detection_profile.clone(),
            ..Self::new(kind, format!("{title} — {}", session.name))
        }
    }
    pub fn profile(session: &SessionView) -> Self {
        Self::of_session(Kind::Profile, "Detection profile", session)
    }
    pub fn segment(session: &SessionView) -> Self {
        Self::of_session(Kind::Segment, "Segment", session)
    }
    pub fn remove_session(session: &SessionView) -> Self {
        Self {
            flag: true,
            ..Self::of_session(Kind::RemoveSession, "Remove session", session)
        }
    }
    pub fn reduce(session: &SessionView) -> Self {
        Self::of_session(Kind::Reduce, "Reduce", session)
    }
    pub fn relocate(session: &SessionView) -> Self {
        Self::of_session(Kind::Relocate, "Move into import folder", session)
    }
    pub fn free_session(session: &SessionView) -> Self {
        Self {
            flag: true,
            ..Self::of_session(Kind::FreeSpace, "Free up cloud space", session)
        }
    }
    pub fn free_flight(flight: &FlightView) -> Self {
        Self::of_flight(Kind::FreeSpace, "Free up cloud space", flight)
    }
    fn of_flight(kind: Kind, title: &str, flight: &FlightView) -> Self {
        Self {
            id: flight.id.clone(),
            name: flight.title.clone().unwrap_or_default(),
            flight: Some(flight.clone()),
            ..Self::new(kind, format!("{title} — {}", flight_name(flight)))
        }
    }
    pub fn delete_flight(flight: &FlightView) -> Self {
        Self::of_flight(Kind::DeleteFlight, "Delete flight", flight)
    }
    pub fn title(flight: &FlightView) -> Self {
        Self::of_flight(Kind::Title, "Edit title", flight)
    }
    pub fn editor(flight: &FlightView) -> Self {
        Self::of_flight(Kind::Editor, "Edit source boundaries", flight)
    }
    pub fn export(session: &str, flight: &FlightView) -> Self {
        let (name, extension) =
            fpv_library::export_name(session, &flight_name(flight), !flight.action_cam);
        Self {
            mode: ExportMode::Lossless,
            file_name: format!("{name}.{extension}"),
            ..Self::of_flight(Kind::Export, "Export", flight)
        }
    }
    pub fn bulk(flights: Vec<String>) -> Self {
        Self {
            flights,
            mode: ExportMode::Parts,
            ..Self::new(Kind::Bulk, "Export selected flights")
        }
    }
    pub fn load() -> Self {
        Self::new(Kind::Load, "Load from destination folder")
    }

    pub fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::BoundarySource { clip, source } => self.editor_source = Some((clip, source)),
            Reply::Folders(folders) => self.folders = Some(folders),
            Reply::Reduce(preview) => self.reduction = Some(preview),
            Reply::Relocation(preview) => self.relocation = Some(preview),
            Reply::Deletion(preview) => self.deletion = Some(preview),
            Reply::Cache(usage) => self.cache = Some(usage),
            Reply::Picked(Some(path)) => self.path = path.to_string_lossy().into_owned(),
            Reply::Mismatch(reason) => self.mismatch = Some(reason),
            _ => {}
        }
    }
    pub fn finished(&mut self, id: u64, success: bool) -> bool {
        if self.saving == Some(id) {
            self.saving = None;
            success
        } else {
            false
        }
    }
}

impl App {
    /// What a dialog's form waits for: another import for the library-wide
    /// ones, a task remaking its session for the others.
    fn dialog_scope(&self, dialog: &Dialog) -> Scope {
        match dialog.kind {
            Kind::Import | Kind::Load | Kind::Settings => Scope::Library,
            Kind::Session
            | Kind::Profile
            | Kind::Segment
            | Kind::RemoveSession
            | Kind::Reduce
            | Kind::Relocate => Scope::Session(dialog.id.clone()),
            Kind::DeleteFlight | Kind::Title | Kind::Editor | Kind::Export | Kind::FreeSpace => {
                self.session_of_flight(&dialog.id)
                    .or_else(|| {
                        self.sessions
                            .iter()
                            .find(|s| s.id == dialog.id)
                            .map(|s| s.id.as_str())
                    })
                    .map_or(Scope::None, |session| Scope::Session(session.to_owned()))
            }
            Kind::Bulk => Scope::None,
        }
    }

    pub(crate) fn dialog_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut close = false;
        let mut modal = egui::Modal::new(egui::Id::new("form"));
        if dialog.kind == Kind::Editor {
            modal.area = modal.area.default_size(egui::vec2(
                (ctx.content_rect().width() - 32.0).min(1040.0),
                (ctx.content_rect().height() - 48.0).max(120.0),
            ));
        }
        modal.show(ctx, |ui| {
            ui.set_width(if dialog.kind == Kind::Editor {
                (ctx.content_rect().width() - 64.0).clamp(240.0, 1000.0)
            } else {
                520.0
            });
            ui.horizontal(|ui| {
                ui.heading(&dialog.title);
                if ui.small_button("×").clicked() {
                    close = true;
                }
            });
            ui.separator();
            let scope = self.dialog_scope(&dialog);
            ui.add_enabled_ui(dialog.saving.is_none() && !self.busy_with(&scope), |ui| {
                self.dialog_form(ui, &mut dialog);
            });
            if dialog.saving.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Working… progress is shown in Tasks.");
                });
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if !close {
            self.dialog = Some(dialog);
        } else {
            self.dialog_request = None;
        }
    }

    fn pick_path(&mut self, save: bool, name: String) {
        self.dialog_request = Some(self.submit("Choose path", true, move |_, _| {
            let dialog = rfd::FileDialog::new();
            let path = if save {
                dialog.set_file_name(&name).save_file()
            } else {
                dialog.pick_folder()
            };
            Ok(Reply::Picked(path))
        }));
    }

    fn dialog_form(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) {
        match dialog.kind {
            Kind::Import => {
                ui.label("Source folder");
                ui.text_edit_singleline(&mut dialog.path);
                if ui.button("Choose source…").clicked() {
                    self.pick_path(false, String::new());
                }
                ui.label("Session name (use an existing name to add takes to it)");
                ui.text_edit_singleline(&mut dialog.name);
                date_picker(
                    ui,
                    "Flying date (optional for action cams)",
                    &mut dialog.date,
                    &mut dialog.calendar_month,
                    &mut dialog.date_picker_open,
                    true,
                );
                let _ = crate::ui::no_sound_choice(ui, &mut dialog.mute_videos);
                ui.small(format!("Copies go to {}", self.config.media_root.display()));
                ui.checkbox(
                    &mut dialog.flag,
                    "Delete newly imported files from the card after verification",
                );
                if ui
                    .add_enabled(
                        self.ffmpeg_ok && !dialog.path.is_empty() && !dialog.name.trim().is_empty(),
                        egui::Button::new("Import"),
                    )
                    .clicked()
                {
                    let (path, name, date, delete, no_sound) = (
                        PathBuf::from(&dialog.path),
                        dialog.name.clone(),
                        format_date(dialog.date),
                        dialog.flag,
                        dialog.mute_videos,
                    );
                    dialog.saving = Some(self.submit_in(Scope::Library, "Import recordings", false, move |library, progress| {
                        let mut counter = ImportProgress::default();
                        let report = library.import_with_audio(&path, &name, date.as_deref(), delete, no_sound, |event| progress.step(counter.step(event)))?;
                        if report.failed > 0 || !report.kept.is_empty() {
                            return Err(fpv_library::LibraryError::Invalid(format!("Imported {}, skipped {}, failed {}. Kept on card: {}. See task logs.", report.imported, report.skipped, report.failed, report.kept.join("; "))));
                        }
                        Ok(Reply::Done(format!("Imported {}, skipped {}, {} flights", report.imported, report.skipped, report.flights)))
                    }));
                }
            }
            Kind::Settings => {
                ui.checkbox(&mut dialog.mute_videos, "Mute videos")
                    .on_hover_text("Mute player playback and exported videos");
                ui.checkbox(&mut dialog.do_not_show_splash, "Do not show splash");
                ui.label("Import folder (existing recordings stay where they are)");
                ui.text_edit_singleline(&mut dialog.path);
                if ui.button("Choose folder…").clicked() {
                    self.pick_path(false, String::new());
                }
                ui.label("Folder naming pattern: {date}, {session}, {source}");
                ui.text_edit_singleline(&mut dialog.name);
                match fpv_library::Library::folder_preview(&dialog.name, "Alpes") {
                    Ok(preview) => {
                        ui.small(format!("Preview: {preview}"));
                    }
                    Err(error) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, error.to_string());
                    }
                }
                ui.small(format!(
                    "Database and settings: {}",
                    self.config_directory.display()
                ));
                ui.small(format!(
                    "Rebuildable cache: {}",
                    self.cache_directory.display()
                ));
                if let Some(cache) = &dialog.cache {
                    ui.label(format!("Cache: {:.1} MB", cache.total() as f64 / 1e6));
                }
                ui.add(
                    egui::DragValue::new(&mut dialog.budgets[0])
                        .range(0.1..=10000.0)
                        .prefix("Preview budget: ")
                        .suffix(" GB"),
                );
                ui.add(
                    egui::DragValue::new(&mut dialog.budgets[1])
                        .range(0.1..=10000.0)
                        .prefix("Picture budget: ")
                        .suffix(" GB"),
                );
                ui.horizontal_wrapped(|ui| {
                    for (kind, label) in [
                        (CacheKind::Proxies, "Empty previews"),
                        (CacheKind::Strips, "Empty hover frames"),
                        (CacheKind::Posters, "Empty posters"),
                        (CacheKind::Previews, "Empty editor frames"),
                    ] {
                        if ui.small_button(label).clicked() {
                            self.viewer = None;
                            self.thumbnails.clear();
                            self.requested.clear();
                            self.images = crate::images::Images::new(ui.ctx().clone());
                            self.dialog_request =
                                Some(self.submit(label, false, move |library, _| {
                                    library.clear_cache(&[kind])?;
                                    Ok(Reply::Cache(library.cache_usage()?))
                                }));
                        }
                    }
                });
                if ui.button("Empty cache").clicked() {
                    self.viewer = None;
                    self.thumbnails.clear();
                    self.requested.clear();
                    self.images = crate::images::Images::new(ui.ctx().clone());
                    self.dialog_request = Some(self.submit("Empty cache", false, |library, _| {
                        library.clear_cache(&CacheKind::ALL)?;
                        Ok(Reply::Cache(library.cache_usage()?))
                    }));
                }
                if ui.button("Save settings").clicked() {
                    let (path, pattern) = (PathBuf::from(&dialog.path), dialog.name.clone());
                    let budgets = dialog.budgets;
                    let (mute, hide_splash) = (dialog.mute_videos, dialog.do_not_show_splash);
                    self.viewer = None;
                    dialog.saving = Some(self.submit_in(
                        Scope::Library,
                        "Save settings",
                        false,
                        move |library, _| {
                            library.set_folder_pattern(&pattern)?;
                            library.set_media_root(&path)?;
                            library.set_cache_budgets(budgets[0], budgets[1])?;
                            library.set_video_preferences(mute, hide_splash)?;
                            Ok(Reply::VideoPreferences {
                                mute_videos: mute,
                                do_not_show_splash: hide_splash,
                            })
                        },
                    ));
                }
                ui.separator();
                ui.small("FPV Rush · MIT OR Apache-2.0");
                ui.small("This software uses FFmpeg. Bundled tools, license, corresponding source and build instructions are in resources/licenses/ffmpeg.");
            }
            Kind::Session => {
                field(ui, "Name", &mut dialog.name);
                date_picker(
                    ui,
                    "Flying day",
                    &mut dialog.date,
                    &mut dialog.calendar_month,
                    &mut dialog.date_picker_open,
                    true,
                );
                field(ui, "Location", &mut dialog.location);
                ui.label("Notes");
                ui.text_edit_multiline(&mut dialog.notes);
                field(ui, "Session tags (comma separated)", &mut dialog.labels);
                if ui.button("Save session").clicked() {
                    let id = dialog.id.clone();
                    let edit = SessionEdit {
                        name: dialog.name.clone(),
                        date: format_date(dialog.date),
                        location: dialog.location.clone(),
                        notes: dialog.notes.clone(),
                        labels: labels(&dialog.labels),
                    };
                    dialog.saving = Some(self.submit("Edit session", false, move |library, _| {
                        library.update_session(&id, &edit)?;
                        Ok(Reply::Done("Session saved".into()))
                    }));
                }
            }
            Kind::Profile | Kind::Segment => {
                if dialog.kind == Kind::Profile {
                    ui.radio_value(&mut dialog.profile, "standard".into(), "Standard");
                    ui.radio_value(&mut dialog.profile, "textured".into(), "Textured terrain");
                }
                let losses = self
                    .sessions
                    .iter()
                    .find(|session| session.id == dialog.id)
                    .map(crate::browse::resegment_losses)
                    .unwrap_or_default();
                // A first segmentation loses nothing and asks nothing.
                if let Some((warning, lost)) = losses.split_first() {
                    ui.colored_label(egui::Color32::from_rgb(230, 160, 60), warning);
                    for line in lost {
                        ui.label(format!("• {line}"));
                    }
                    ui.weak("Figures stay: they belong to the recordings. Action-cam takes are not touched.");
                    ui.checkbox(
                        &mut dialog.acknowledged,
                        "I accept replacing the existing DVR segmentation",
                    );
                }
                if ui
                    .add_enabled(
                        (losses.is_empty() || dialog.acknowledged) && self.ffmpeg_ok,
                        egui::Button::new("Save and segment"),
                    )
                    .clicked()
                {
                    self.viewer = None;
                    let (id, profile) = (dialog.id.clone(), dialog.profile.clone());
                    dialog.saving = Some(self.submit_in(
                        Scope::Session(id.clone()),
                        "Segment session",
                        false,
                        move |library, progress| {
                            library.set_detection_profile(&id, &profile)?;
                            let total = library
                                .snapshot()?
                                .iter()
                                .find(|s| s.id == id)
                                .map(|s| {
                                    s.files
                                        .iter()
                                        .filter(|f| !f.action_cam && !f.reduced)
                                        .count()
                                })
                                .unwrap_or_default();
                            let mut counter = SegmentProgress::new(total);
                            let report =
                                library.segment(&id, |event| progress.step(counter.step(event)))?;
                            if !report.failed.is_empty() {
                                return Err(fpv_library::LibraryError::Invalid(format!(
                                    "{} flights, failures: {:?}",
                                    report.flights, report.failed
                                )));
                            }
                            Ok(Reply::Done(format!(
                                "{} flights, {} clips",
                                report.flights, report.clips
                            )))
                        },
                    ));
                }
            }
            Kind::RemoveSession => {
                ui.label("Remove this session and its flights from the library?");
                ui.checkbox(
                    &mut dialog.flag,
                    "Keep recordings on disk (can be loaded again later)",
                );
                if !dialog.flag {
                    ui.label("Its recordings will be moved to the system trash.");
                }
                if ui.button("Remove session").clicked() {
                    self.viewer = None;
                    let (id, keep) = (dialog.id.clone(), dialog.flag);
                    dialog.saving = Some(self.submit_in(
                        Scope::Session(id.clone()),
                        "Remove session",
                        false,
                        move |library, _| {
                            library.delete_session(&id, keep)?;
                            Ok(Reply::Done("Session removed".into()))
                        },
                    ));
                }
            }
            Kind::Reduce => {
                if let Some(preview) = &dialog.reduction {
                    ui.label(format!(
                        "{} recordings · {:.1} MB → approximately {:.1} MB",
                        preview.recordings,
                        preview.bytes_now as f64 / 1e6,
                        preview.bytes_after as f64 / 1e6
                    ));
                    ui.label(format!("{} empty recordings removed; {} never-segmented recordings left alone; {} figures in discarded footage lost.", preview.emptied, preview.unsegmented, preview.figures_lost));
                    ui.label("Keeps flights with 5 seconds on each side. Originals go to the trash. Missed flights are lost and this footage cannot be re-segmented. Titles, likes, tags and figures inside kept footage remain.");
                    ui.checkbox(
                        &mut dialog.acknowledged,
                        "I understand this reduction cannot be undone in the app",
                    );
                    if ui
                        .add_enabled(
                            dialog.acknowledged && self.ffmpeg_ok,
                            egui::Button::new("Reduce recordings"),
                        )
                        .clicked()
                    {
                        self.viewer = None;
                        let id = dialog.id.clone();
                        dialog.saving = Some(self.submit_in(
                            Scope::Session(id.clone()),
                            "Reduce session",
                            false,
                            move |library, progress| {
                                let report = library.reduce_session(&id, |done, total, line| {
                                    progress.count(done, total, line)
                                })?;
                                if !report.failed.is_empty() {
                                    return Err(fpv_library::LibraryError::Invalid(format!(
                                        "{} pieces; failures: {:?}",
                                        report.pieces, report.failed
                                    )));
                                }
                                Ok(Reply::Done(format!(
                                    "{} pieces, {:.1} MB moved to trash",
                                    report.pieces,
                                    report.bytes_freed as f64 / 1e6
                                )))
                            },
                        ));
                    }
                } else {
                    ui.spinner();
                    ui.label("Calculating reduction…");
                }
            }
            Kind::Relocate => {
                if let Some(preview) = &dialog.relocation {
                    ui.label(format!(
                        "Move {} recordings ({:.1} MB)",
                        preview.recordings,
                        preview.bytes as f64 / 1e6
                    ));
                    ui.label(format!(
                        "From: {}\nTo: {}",
                        preview.from.join("\n"),
                        preview.to.join("\n")
                    ));
                    if ui.button("Move recordings").clicked() {
                        self.viewer = None;
                        let id = dialog.id.clone();
                        dialog.saving = Some(self.submit_in(
                            Scope::Session(id.clone()),
                            "Move session",
                            false,
                            move |library, progress| {
                                let report = library
                                    .move_session_into_import_folder(&id, |done, total, line| {
                                        progress.count(done, total, line)
                                    })?;
                                if !report.failed.is_empty() {
                                    return Err(fpv_library::LibraryError::Invalid(format!(
                                        "{} moved; failures: {:?}",
                                        report.moved, report.failed
                                    )));
                                }
                                Ok(Reply::Done(format!("{} recordings moved", report.moved)))
                            },
                        ));
                    }
                } else {
                    ui.spinner();
                }
            }
            Kind::DeleteFlight => {
                if let Some(preview) = &dialog.deletion {
                    ui.label(format!("Whole DVR recordings kept: {}\nShared reduced pieces kept: {}\nFiles moved to trash:\n{}", preview.recordings.len(), preview.shared.len(), preview.trashed.join("\n")));
                    if ui.button("Delete flight").clicked() {
                        self.viewer = None;
                        let id = dialog.id.clone();
                        dialog.saving =
                            Some(self.submit("Delete flight", false, move |library, _| {
                                library.delete_flight(&id)?;
                                Ok(Reply::Done("Flight deleted".into()))
                            }));
                    }
                } else {
                    ui.spinner();
                }
            }
            Kind::Title => {
                field(
                    ui,
                    "Flight title (empty restores its number)",
                    &mut dialog.name,
                );
                if ui.button("Save title").clicked() {
                    let (id, title) = (dialog.id.clone(), dialog.name.clone());
                    dialog.saving =
                        Some(self.submit("Edit flight title", false, move |library, _| {
                            library.set_flight_title(&id, &title)?;
                            Ok(Reply::Done("Title saved".into()))
                        }));
                }
            }
            Kind::Editor => {
                if let Some(flight) = &mut dialog.flight {
                    if flight.clips.is_empty() {
                        ui.label("No source clips available.");
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .max_height((ui.ctx().content_rect().height() - 180.0).max(120.0))
                        .show(ui, |ui| {
                            egui::ComboBox::from_id_salt("boundary-source-clip")
                                .selected_text(format!("{} / {} · {}", dialog.editor_clip + 1, flight.clips.len(), flight.clips[dialog.editor_clip].file_name))
                                .show_ui(ui, |ui| {
                                    for (index, clip) in flight.clips.iter().enumerate() {
                                        ui.selectable_value(&mut dialog.editor_clip, index, format!("{} · {}", index + 1, clip.file_name));
                                    }
                                });
                            let clip = &mut flight.clips[dialog.editor_clip];
                            ui.small(format!("{} · {:.6} fps · source frames", clip.file_name, clip.fps));
                            if dialog.editor_loading.as_deref() != Some(&clip.id) {
                                dialog.editor_preview = None;
                                dialog.editor_source = None;
                                dialog.editor_loading = Some(clip.id.clone());
                                let id = clip.file_id.clone();
                                let clip_id = clip.id.clone();
                                let flight_id = flight.id.clone();
                                self.dialog_request = Some(self.submit("Open boundary source", true, move |library, progress| {
                                    let report = library.download_flight(&flight_id, |step| progress.count(step.file, step.files, step.line))?;
                                    if !report.failed.is_empty() {
                                        return Err(fpv_library::LibraryError::Invalid(format!("Source download failed: {:?}", report.failed)));
                                    }
                                    Ok(Reply::BoundarySource { clip: clip_id, source: library.native_source_playback(&id)? })
                                }));
                            }
                            if let Some((id, source)) = dialog.editor_source.take() && id == clip.id {
                                let muted = self.config.mute_videos || !source.has_audio;
                                dialog.editor_preview = Some(crate::boundary::SourcePreview::new(clip, source, self.tools.clone(), muted, ui.ctx()));
                            }
                            if let Some(preview) = &mut dialog.editor_preview {
                                self.notices.extend(preview.show(ui, clip));
                            } else {
                                crate::boundary::preview_canvas(ui, None);
                                let failed = self.tasks.iter().any(|task| Some(task.id) == self.dialog_request && !task.running && task.failed);
                                if failed {
                                    ui.label("Source preview unavailable. See Tasks for the error.");
                                    if ui.button("Retry preview").clicked() { dialog.editor_loading = None; }
                                } else { ui.label("Preparing source preview…"); }
                            }
                            ui.separator();
                            if let Some(frame) = crate::boundary::trim_timeline(ui, clip)
                                && let Some(preview) = &mut dialog.editor_preview {
                                preview.seek_frame(frame);
                                ui.ctx().request_repaint();
                            }
                            ui.small("Drag the left edge for start and the right edge for end. End is exclusive.");
                            if ui.button("Save this clip").clicked() {
                                let (id, start, end) = (clip.id.clone(), clip.start_frame, clip.end_frame);
                                dialog.saving = Some(self.submit("Save clip boundary", false, move |library, _| {
                                    library.update_clip(&id, start, end)?;
                                    Ok(Reply::Done("Boundaries saved".into()))
                                }));
                            }
                        });
                }
            }
            Kind::Export | Kind::Bulk => {
                field(
                    ui,
                    if dialog.kind == Kind::Bulk {
                        "Destination folder"
                    } else {
                        "Destination file"
                    },
                    &mut dialog.path,
                );
                if ui.button("Choose destination…").clicked() {
                    let name = if dialog.kind == Kind::Export {
                        dialog.file_name.clone()
                    } else {
                        "Flights".into()
                    };
                    self.pick_path(dialog.kind == Kind::Export, name);
                }
                // A single export goes lossless and asks only when the
                // recordings disagree on their rate; a batch cannot ask per
                // flight, so it is answered up front.
                let mut chosen = None;
                if let Some(reason) = dialog.mismatch.clone() {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 160, 60),
                        "Different frame rates",
                    );
                    ui.label(reason);
                    let parts = dialog.flight.as_ref().map_or(2, |f| f.recordings);
                    if ui
                        .button(format!("Export {parts} parts separately"))
                        .clicked()
                    {
                        chosen = Some(ExportMode::Parts);
                    }
                    // Re-encoding resamples MJPEG: the DVR's footage only.
                    if dialog.flight.as_ref().is_some_and(|f| !f.action_cam)
                        && ui.button("Re-encode into one file").clicked()
                    {
                        chosen = Some(ExportMode::Reencode);
                    }
                } else if dialog.kind == Kind::Bulk {
                    ui.label("For recordings with different frame rates:");
                    ui.radio_value(
                        &mut dialog.mode,
                        ExportMode::Parts,
                        "One lossless file per recording",
                    );
                    ui.radio_value(
                        &mut dialog.mode,
                        ExportMode::Reencode,
                        "One file re-encoded at the dominant rate (DVR only)",
                    );
                }
                if dialog.mismatch.is_none()
                    && ui
                        .add_enabled(
                            !dialog.path.is_empty() && self.ffmpeg_ok,
                            egui::Button::new("Export"),
                        )
                        .clicked()
                {
                    chosen = Some(dialog.mode);
                }
                if let Some(mode) = chosen {
                    dialog.mismatch = None;
                    let (id, ids, path, bulk) = (
                        dialog.id.clone(),
                        dialog.flights.clone(),
                        PathBuf::from(&dialog.path),
                        dialog.kind == Kind::Bulk,
                    );
                    dialog.saving =
                        Some(
                            self.submit("Export flights", false, move |library, progress| {
                                if bulk {
                                    let report = library.export_flights(
                                        &ids,
                                        &path,
                                        mode,
                                        |done, total, line| progress.count(done, total, line),
                                    )?;
                                    if !report.failed.is_empty() {
                                        return Err(fpv_library::LibraryError::Invalid(format!(
                                            "{} written; failures: {:?}",
                                            report.written.len(),
                                            report.failed
                                        )));
                                    }
                                    Ok(Reply::Done(format!(
                                        "{} files exported",
                                        report.written.len()
                                    )))
                                } else {
                                    // Library::export also refuses overwrite, for all modes.
                                    export_reply(library.export(&id, &path, mode))
                                }
                            }),
                        );
                }
            }
            Kind::FreeSpace => {
                ui.label("Make these recordings Online Only on the cloud drive? Their local content is released; the cloud placeholders and library entries remain. Playing them again downloads them.");
                ui.label(
                    "The cloud provider can refuse to release a file; its error will be reported.",
                );
                if ui.button("Free up space").clicked() {
                    self.viewer = None;
                    let (id, session) = (dialog.id.clone(), dialog.flag);
                    dialog.saving = Some(self.submit(
                        "Free up cloud space",
                        false,
                        move |library, progress| {
                            let report = if session {
                                library.free_up_session(&id, |a, b, c| progress.count(a, b, c))?
                            } else {
                                library.free_up_flight(&id, |a, b, c| progress.count(a, b, c))?
                            };
                            if !report.failed.is_empty() {
                                return Err(fpv_library::LibraryError::Invalid(format!(
                                    "{} freed; failed: {:?}",
                                    report.freed, report.failed
                                )));
                            }
                            Ok(Reply::Done(format!(
                                "Released {} recordings ({:.1} MB)",
                                report.freed,
                                report.bytes as f64 / 1e6
                            )))
                        },
                    ));
                }
            }
            Kind::Load => {
                if let Some(folders) = &dialog.folders {
                    egui::ScrollArea::vertical()
                        .max_height(350.0)
                        .show(ui, |ui| {
                            for folder in folders {
                                let mut chosen = dialog.chosen.contains(&folder.name);
                                if ui
                                    .add_enabled(
                                        folder.new > 0 || dialog.restore.contains(&folder.name),
                                        egui::Checkbox::new(
                                            &mut chosen,
                                            format!(
                                                "{} · {} new / {} recordings",
                                                folder.name, folder.new, folder.recordings
                                            ),
                                        ),
                                    )
                                    .changed()
                                {
                                    if chosen {
                                        dialog.chosen.insert(folder.name.clone());
                                    } else {
                                        dialog.chosen.remove(&folder.name);
                                    }
                                }
                                if !folder.deleted.is_empty() {
                                    ui.indent(&folder.name, |ui| {
                                        let mut restore = dialog.restore.contains(&folder.name);
                                        if ui.checkbox(&mut restore, format!("Restore {} recording(s) previously deleted with their flight", folder.deleted.len())).changed() {
                                            if restore { dialog.restore.insert(folder.name.clone()); dialog.chosen.insert(folder.name.clone()); }
                                            else { dialog.restore.remove(&folder.name); if folder.new == 0 { dialog.chosen.remove(&folder.name); } }
                                        }
                                        ui.collapsing("Deleted recordings", |ui| { for file in &folder.deleted { ui.small(file); } });
                                    });
                                }
                            }
                        });
                    if ui
                        .add_enabled(
                            !dialog.chosen.is_empty(),
                            egui::Button::new("Load selected folders"),
                        )
                        .clicked()
                    {
                        let chosen: Vec<_> = dialog.chosen.iter().cloned().collect();
                        let restore: Vec<_> = dialog
                            .restore
                            .intersection(&dialog.chosen)
                            .cloned()
                            .collect();
                        dialog.saving = Some(self.submit_in(
                            Scope::Library,
                            "Load destination folders",
                            false,
                            move |library, progress| {
                                let mut counter = ImportProgress::default();
                                let report =
                                    library.load_import_folders(&chosen, &restore, |event| {
                                        progress.step(counter.step(event))
                                    })?;
                                if report.failed > 0 {
                                    return Err(fpv_library::LibraryError::Invalid(format!(
                                        "{} loaded; {} failed",
                                        report.imported, report.failed
                                    )));
                                }
                                Ok(Reply::Done(format!(
                                    "{} recordings loaded in place",
                                    report.imported
                                )))
                            },
                        ));
                    }
                } else {
                    ui.spinner();
                }
            }
        }
    }
}

fn field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    ui.label(label);
    ui.text_edit_singleline(text);
}

fn date_picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut Option<NaiveDate>,
    month: &mut NaiveDate,
    open: &mut bool,
    allow_clear: bool,
) {
    ui.label(label);
    let button_label = selected
        .map(|date| date.format("%a, %b %d, %Y").to_string())
        .unwrap_or_else(|| "Choose date…".to_owned());
    if ui.button(button_label).clicked() {
        *open = !*open;
        if let Some(date) = selected {
            *month = date.with_day(1).unwrap();
        }
    }
    if !*open {
        return;
    }

    ui.group(|ui| {
        ui.horizontal(|ui| {
            if ui.small_button("‹").clicked()
                && let Some(previous) = month.checked_sub_months(Months::new(1))
            {
                *month = previous;
            }
            ui.label(month.format("%B %Y").to_string());
            if ui.small_button("›").clicked()
                && let Some(next) = month.checked_add_months(Months::new(1))
            {
                *month = next;
            }
            if allow_clear && selected.is_some() && ui.small_button("Clear").clicked() {
                *selected = None;
                *open = false;
            }
        });
        egui::Grid::new(format!("calendar-{label}"))
            .num_columns(7)
            .show(ui, |ui| {
                for weekday in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                    ui.weak(weekday);
                }
                ui.end_row();

                let first = month.with_day(1).unwrap();
                let leading = first.weekday().num_days_from_monday();
                for _ in 0..leading {
                    ui.label("");
                }
                let next_month = first.checked_add_months(Months::new(1)).unwrap();
                let last_day = (next_month - chrono::Duration::days(1)).day();
                for day in 1..=last_day {
                    let date = first.with_day(day).unwrap();
                    let mut button = egui::Button::new(day.to_string());
                    if *selected == Some(date) {
                        button = button.fill(ACCENT);
                    }
                    if ui.add_sized([28.0, 24.0], button).clicked() {
                        *selected = Some(date);
                        *open = false;
                    }
                    if (leading + day).is_multiple_of(7) {
                        ui.end_row();
                    }
                }
            });
    });
}

fn format_date(date: Option<NaiveDate>) -> Option<String> {
    date.map(|date| date.format("%Y-%m-%d").to_string())
}

/// What a single export says: the files written, or — when a lossless one
/// met recordings at different rates — the library's reason, for the dialog
/// to ask how instead of failing.
fn export_reply(
    written: Result<Vec<PathBuf>, fpv_library::LibraryError>,
) -> Result<Reply, fpv_library::LibraryError> {
    match written {
        Err(
            error @ fpv_library::LibraryError::Export(
                fpv_export::ExportError::MismatchedFrameRates(..),
            ),
        ) => Ok(Reply::Mismatch(error.to_string())),
        Err(error) => Err(error),
        Ok(files) => Ok(Reply::Done(format!(
            "Exported {}",
            files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn an_export_proposes_the_session_and_the_flight_title() {
        let flight = FlightView {
            id: "f".into(),
            order: 3,
            title: Some("Bando".into()),
            liked: false,
            duration: 60.0,
            clips: vec![],
            recordings: 2,
            action_cam: false,
            source: "DVR".into(),
            format: String::new(),
            reduced: false,
            cloud: false,
            freeable: false,
            cache_key: "f".into(),
            labels: vec![],
            figures: vec![],
        };
        let dialog = Dialog::export("Alpes", &flight);
        assert_eq!(dialog.file_name, "Alpes — Bando.avi");
        assert_eq!(
            dialog.mode,
            ExportMode::Lossless,
            "lossless first, asked on a mismatch"
        );
        let take = FlightView {
            title: None,
            action_cam: true,
            ..flight
        };
        assert_eq!(
            Dialog::export("Alpes", &take).file_name,
            "Alpes — Flight 3.mp4"
        );
    }

    #[test]
    fn a_rate_mismatch_is_a_question_not_a_failure() {
        use fpv_core::FrameRate;
        let mismatch =
            fpv_library::LibraryError::Export(fpv_export::ExportError::MismatchedFrameRates(
                FrameRate::new(1000000, 33459),
                FrameRate::new(1000000, 40839),
            ));
        let Ok(Reply::Mismatch(reason)) = export_reply(Err(mismatch)) else {
            panic!("asked, not failed");
        };
        assert!(!reason.is_empty());
        let other = fpv_library::LibraryError::Invalid("disk full".into());
        assert!(export_reply(Err(other)).is_err());
        assert!(matches!(
            export_reply(Ok(vec!["/tmp/a.avi".into()])),
            Ok(Reply::Done(done)) if done == "Exported /tmp/a.avi"
        ));
    }
}

#[cfg(test)]
mod date_picker_tests {
    use super::*;

    #[test]
    fn new_import_selects_today_and_still_allows_clearing_the_date() {
        let before = chrono::Local::now().date_naive();
        let mut dialog = Dialog::import(None, "Test import".into());
        let after = chrono::Local::now().date_naive();
        assert!(dialog.date == Some(before) || dialog.date == Some(after));
        assert_eq!(
            dialog.calendar_month,
            dialog.date.unwrap().with_day(1).unwrap()
        );
        dialog.date = None;
        assert_eq!(format_date(dialog.date), None);
    }

    #[test]
    fn formats_selected_day_for_the_library_and_preserves_clear() {
        assert_eq!(
            format_date(Some(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())),
            Some("2024-02-29".into())
        );
        assert_eq!(format_date(None), None);
    }

    #[test]
    fn calendar_month_length_handles_leap_years_and_year_boundaries() {
        let february = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        let next = february.checked_add_months(Months::new(1)).unwrap();
        assert_eq!((next - chrono::Duration::days(1)).day(), 29);

        let december = NaiveDate::from_ymd_opt(2025, 12, 1).unwrap();
        assert_eq!(
            december.checked_add_months(Months::new(1)).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );
        assert_eq!(
            december.checked_sub_months(Months::new(1)).unwrap(),
            NaiveDate::from_ymd_opt(2025, 11, 1).unwrap()
        );
    }
}
