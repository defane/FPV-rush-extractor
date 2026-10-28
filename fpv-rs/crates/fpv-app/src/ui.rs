use crate::{
    ACCENT, App, Dialog, Reply, Scope, Viewer,
    browse::{self, Sort, View, clock, flight_name, page_range},
    day, reveal,
};
use eframe::egui::{self, Color32, RichText};
use fpv_library::{FlightView, SessionView};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy)]
enum MetadataIcon {
    Dvr,
    Film,
    Speaker(bool),
    Settings,
    Chevron,
    Cloud,
    Camera,
    Folder,
    Export,
}

fn metadata_icon(ui: &mut egui::Ui, kind: MetadataIcon) {
    let (area, response) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::hover());
    paint_metadata_icon(ui, kind, area);
    response.on_hover_text(match kind {
        MetadataIcon::Dvr => "Recording source",
        MetadataIcon::Film => "Video format",
        MetadataIcon::Speaker(true) => "Muted",
        MetadataIcon::Speaker(false) => "Playback volume",
        MetadataIcon::Settings => "Settings and About",
        MetadataIcon::Chevron => "Import options",
        MetadataIcon::Cloud => "Online Only on the cloud drive: downloads when played",
        MetadataIcon::Camera => "Action camera",
        MetadataIcon::Folder => "In an earlier import folder",
        MetadataIcon::Export => "Export…",
    });
}

/// Names in use matching what is typed: a click puts one in the field.
fn suggestions(ui: &mut egui::Ui, field: &mut String, names: Vec<String>) {
    if names.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for name in names {
            if ui.small_button(&name).clicked() {
                *field = name;
            }
        }
    });
}

/// The speaker: mutes, or brings the sound back at the volume it had.
fn toggle_mute(viewer: &mut Viewer) {
    if viewer.volume <= 0.001 {
        viewer.volume = viewer.remembered_volume.max(0.01);
        viewer.playback_muted = false;
    } else {
        viewer.remembered_volume = viewer.volume;
        viewer.playback_muted = !viewer.playback_muted;
    }
    viewer.player.set_volume(if viewer.playback_muted {
        0.0
    } else {
        viewer.volume
    });
}

fn player_speaker(ui: &mut egui::Ui, viewer: &mut Viewer) -> egui::Response {
    let response = ui.add_enabled(
        !viewer.muted,
        egui::Button::new(" ")
            .frame(false)
            .min_size(egui::vec2(26.0, 26.0)),
    );
    if response.clicked() {
        toggle_mute(viewer);
    }
    let muted = viewer.muted || viewer.playback_muted || viewer.volume <= 0.001;
    paint_metadata_icon(ui, MetadataIcon::Speaker(muted), response.rect.shrink(3.0));
    response.on_hover_text(if viewer.muted {
        "No sound for this recording"
    } else if muted {
        "Unmute"
    } else {
        "Mute"
    })
}

fn player_volume_overlay(
    ui: &mut egui::Ui,
    viewer: &mut Viewer,
    video: egui::Rect,
) -> Option<egui::Response> {
    if viewer.muted {
        return None;
    }
    let rect = egui::Rect::from_min_size(
        video.right_bottom() - egui::vec2(40.0, 230.0_f32.min(video.height())),
        egui::vec2(32.0, 140.0_f32.min(video.height() * 0.4)),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("video-volume")
            .max_rect(rect),
    );
    child.set_clip_rect(ui.clip_rect().intersect(video));
    child.spacing_mut().slider_width = rect.height();
    let response = child.add(
        egui::Slider::new(&mut viewer.volume, 0.0..=1.0)
            .vertical()
            .show_value(false),
    );
    if response.changed() {
        viewer.playback_muted = false;
        if viewer.volume > 0.001 {
            viewer.remembered_volume = viewer.volume;
        }
        viewer.player.set_volume(viewer.volume);
    }
    Some(response.on_hover_text("Volume"))
}

fn paint_metadata_icon(ui: &egui::Ui, kind: MetadataIcon, area: egui::Rect) {
    let color = ui.visuals().weak_text_color();
    let stroke = egui::Stroke::new(1.2_f32, color);
    let body = egui::Rect::from_min_max(
        area.min + egui::vec2(2.0, 3.0),
        area.max - egui::vec2(2.0, 2.0),
    );
    let painter = ui.painter();
    match kind {
        MetadataIcon::Settings => {
            let center = body.center();
            painter.circle_stroke(center, 4.0, stroke);
            painter.circle_stroke(center, 1.5, stroke);
            for tooth in 0..8 {
                let angle = tooth as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 4.0, center + direction * 6.5], stroke);
            }
        }
        MetadataIcon::Chevron => {
            painter.line_segment(
                [
                    body.left_center() - egui::vec2(0.0, 2.0),
                    body.center() + egui::vec2(0.0, 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    body.center() + egui::vec2(0.0, 2.0),
                    body.right_center() - egui::vec2(0.0, 2.0),
                ],
                stroke,
            );
        }

        MetadataIcon::Dvr => {
            painter.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    body.left_top() + egui::vec2(3.0, 3.0),
                    body.right_top() - egui::vec2(3.0, -3.0),
                ],
                stroke,
            );
            painter.circle_filled(body.right_bottom() - egui::vec2(3.0, 3.0), 1.2_f32, color);
        }
        MetadataIcon::Camera => {
            let lens = body.center() - egui::vec2(1.5, 0.0);
            painter.rect_stroke(
                egui::Rect::from_min_max(
                    body.min + egui::vec2(0.0, 2.0),
                    body.max - egui::vec2(3.0, 1.0),
                ),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(lens, 2.2, stroke);
            painter.line_segment(
                [
                    body.right_center() - egui::vec2(3.0, 0.0),
                    body.right_top() + egui::vec2(0.0, 3.0),
                ],
                stroke,
            );
        }
        MetadataIcon::Export => {
            // An arrow down into a tray.
            let x = body.center().x;
            painter.line_segment(
                [
                    egui::pos2(x, body.top()),
                    egui::pos2(x, body.bottom() - 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(x - 3.0, body.bottom() - 7.0),
                    egui::pos2(x, body.bottom() - 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(x + 3.0, body.bottom() - 7.0),
                    egui::pos2(x, body.bottom() - 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    body.left_bottom() + egui::vec2(1.0, 0.0),
                    body.right_bottom() - egui::vec2(1.0, 0.0),
                ],
                stroke,
            );
        }
        MetadataIcon::Folder => {
            let top = body.top() + 3.0;
            painter.rect_stroke(
                egui::Rect::from_min_max(egui::pos2(body.left(), top), body.max),
                1.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    egui::pos2(body.left() + 1.0, top),
                    egui::pos2(body.left() + 5.0, top - 2.0),
                ],
                stroke,
            );
        }
        MetadataIcon::Cloud => {
            // Two puffs on a flat base, in the accent: something to notice.
            let stroke = egui::Stroke::new(1.4_f32, crate::ACCENT);
            let base = body.center().y + 3.0;
            painter.circle_stroke(egui::pos2(body.center().x - 2.5, base - 2.5), 3.0, stroke);
            painter.circle_stroke(egui::pos2(body.center().x + 2.0, base - 3.5), 4.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(body.left() + 1.0, base),
                    egui::pos2(body.right() - 1.0, base),
                ],
                stroke,
            );
        }
        MetadataIcon::Film => {
            painter.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Inside);
            let screen = egui::Rect::from_min_max(
                body.min + egui::vec2(5.0, 2.0),
                body.max - egui::vec2(5.0, 2.0),
            );
            painter.rect_stroke(screen, 0.5, stroke, egui::StrokeKind::Inside);
            for y in [body.top() + 3.0, body.center().y, body.bottom() - 3.0] {
                painter.circle_filled(egui::pos2(body.left() + 2.5, y), 0.7_f32, color);
                painter.circle_filled(egui::pos2(body.right() - 2.5, y), 0.7_f32, color);
            }
        }
        MetadataIcon::Speaker(muted) => {
            let speaker = egui::Rect::from_min_max(
                body.min + egui::vec2(1.0, 3.0),
                body.min + egui::vec2(6.0, 10.0),
            );
            painter.rect_filled(speaker, 0.5, color);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(body.left() + 5.0, body.top() + 2.0),
                    egui::pos2(body.left() + 11.0, body.top()),
                    egui::pos2(body.left() + 11.0, body.bottom()),
                    egui::pos2(body.left() + 5.0, body.bottom() - 2.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
            if muted {
                painter.line_segment(
                    [body.right_top(), body.left_bottom() + egui::vec2(2.0, -1.0)],
                    egui::Stroke::new(1.6_f32, color),
                );
            } else {
                painter.line_segment(
                    [
                        egui::pos2(body.left() + 10.5, body.top() + 1.0),
                        egui::pos2(body.right(), body.center().y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(body.right(), body.center().y),
                        egui::pos2(body.left() + 10.5, body.bottom() - 1.0),
                    ],
                    stroke,
                );
            }
        }
    }
}

pub(crate) fn no_sound_choice(ui: &mut egui::Ui, no_sound: &mut bool) -> egui::Response {
    ui.horizontal(|ui| {
        let icon = ui
            .add(
                egui::Button::new("")
                    .selected(*no_sound)
                    .min_size(egui::vec2(26.0, 24.0)),
            )
            .on_hover_text("Toggle sound for this import");
        if icon.clicked() {
            *no_sound = !*no_sound;
        }
        paint_metadata_icon(ui, MetadataIcon::Speaker(*no_sound), icon.rect.shrink(3.0));
        ui.checkbox(no_sound, "No Sound");
        icon
    })
    .inner
}

fn thumbnail_badge(ui: &egui::Ui, image: egui::Rect, text: &str, fill: Color32, right: bool) {
    let label = if text.chars().count() > 26 {
        format!("{}…", text.chars().take(25).collect::<String>())
    } else {
        text.to_owned()
    };
    let galley =
        ui.painter()
            .layout_no_wrap(label, egui::FontId::proportional(12.0), Color32::WHITE);
    let size = galley.size() + egui::vec2(12.0, 4.0);
    let x = if right {
        image.right() - size.x - 6.0
    } else {
        image.left() + 6.0
    };
    let rect = egui::Rect::from_min_size(egui::pos2(x, image.bottom() - size.y - 6.0), size);
    ui.painter().rect_filled(rect, 9.0, fill);
    ui.painter()
        .galley(rect.min + egui::vec2(6.0, 2.0), galley, Color32::WHITE);
}

fn splash_logo(ui: &mut egui::Ui, texture: &egui::TextureHandle, progress: f32) {
    let opacity = (1.0 - (progress - 0.55).max(0.0) / 0.45).clamp(0.0, 1.0);
    let logo_size = 48.0 + 752.0 * progress.powf(1.5);
    ui.vertical_centered(|ui| {
        ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(logo_size * egui::vec2(1.0, 1.4))
                .tint(Color32::from_white_alpha((255.0 * opacity) as u8)),
        );
    });
}

fn pagination_footer(
    ui: &mut egui::Ui,
    state: &mut browse::Browse,
    total: usize,
    range: &std::ops::Range<usize>,
) {
    let row_height = 28.0_f32.max(ui.spacing().interact_size.y);
    let width = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let counter_width = (width * 0.3).min(200.0);
    let per_page_width = 140.0_f32.min(width * 0.28);
    let navigation_width = (width - counter_width - per_page_width - 2.0 * gap).max(1.0);
    let page_count = total.div_ceil(state.page_size).max(1);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let number_width = |page: usize| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap((page + 1).to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .max(10.0)
            + 2.0 * ui.spacing().button_padding.x
    };
    let navigation_size = |pages: &[Option<usize>]| {
        56.0 + gap * (pages.len() + 1) as f32
            + pages
                .iter()
                .map(|p| p.map(number_width).unwrap_or(14.0))
                .sum::<f32>()
    };
    let mut pages = browse::visible_pages(state.page, page_count);
    if navigation_size(&pages) > navigation_width {
        let mut indexes = vec![0, state.page, page_count - 1];
        indexes.sort_unstable();
        indexes.dedup();
        pages.clear();
        for index in indexes {
            if let Some(Some(last)) = pages.last()
                && index > last + 1
            {
                pages.push(None);
            }
            pages.push(Some(index));
        }
    }
    if navigation_size(&pages) > navigation_width {
        pages = vec![Some(state.page)];
    }
    let navigation_size = navigation_size(&pages);
    let button_widths: Vec<_> = pages
        .iter()
        .map(|p| p.map(number_width).unwrap_or(14.0))
        .collect();
    let showing = format!(
        "Showing {}–{} of {total}",
        range.start.saturating_add(1).min(total),
        range.end
    );
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(counter_width, row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(counter_width, row_height));
                ui.add(egui::Label::new(RichText::new(&showing).small().weak()).truncate())
                    .on_hover_text(&showing);
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(navigation_width, row_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(navigation_width, row_height));
                ui.add_space(((navigation_width - navigation_size) / 2.0).max(0.0));
                if ui
                    .add_enabled(
                        state.page > 0,
                        egui::Button::new("‹").min_size(egui::vec2(28.0, row_height)),
                    )
                    .clicked()
                {
                    state.page -= 1;
                }
                for (page, width) in pages.iter().zip(button_widths) {
                    if let Some(page) = *page {
                        let mut button = egui::Button::new((page + 1).to_string())
                            .wrap_mode(egui::TextWrapMode::Extend);
                        if page == state.page {
                            button = button.fill(ACCENT);
                        }
                        if ui.add_sized([width, row_height], button).clicked() {
                            state.page = page;
                        }
                    } else {
                        ui.label("…");
                    }
                }
                if ui
                    .add_enabled(
                        range.end < total,
                        egui::Button::new("›").min_size(egui::vec2(28.0, row_height)),
                    )
                    .clicked()
                {
                    state.page += 1;
                }
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(per_page_width, row_height),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(per_page_width, row_height));
                egui::ComboBox::from_id_salt("page-size")
                    .width(70.0)
                    .selected_text(state.page_size.to_string())
                    .show_ui(ui, |ui| {
                        for size in [12, 24, 48, 96] {
                            if ui
                                .selectable_value(&mut state.page_size, size, size.to_string())
                                .changed()
                            {
                                state.page = 0;
                            }
                        }
                    });
                ui.small("Per page");
            },
        );
    });
}

impl App {
    pub(crate) fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        egui::Window::new("About FPV Rush")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .corner_radius(8.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Image::new(&self.splash_texture)
                                    .max_size(180.0 * egui::vec2(1.0, 1.4)),
                            );
                        });
                    ui.heading("FPV Rush");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.hyperlink_to(
                        "github.com/defane/FPV-rush-extractor",
                        "https://github.com/defane/FPV-rush-extractor",
                    );
                    ui.add_space(8.0);
                    for (role, names) in [
                        ("Coded by", ["Claude", "Codex"]),
                        ("Driven by", ["Defane", "Thanat0s"]),
                    ] {
                        ui.strong(role);
                        for name in names {
                            ui.label(format!("• {name}"));
                        }
                        ui.add_space(4.0);
                    }
                    ui.hyperlink_to("letzkwad.lu", "https://letzkwad.lu/");
                });
            });
    }

    pub(crate) fn splash_window(&mut self, ctx: &egui::Context) {
        let Some(started) = self.splash_started else {
            return;
        };
        let progress = (started.elapsed().as_secs_f32() / 1.5).clamp(0.0, 1.0);
        if progress >= 1.0 {
            self.splash_started = None;
            return;
        }
        egui::Window::new("Letz Kwad")
            .id(egui::Id::new("startup-splash"))
            .constrain(false)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .frame(egui::Frame::NONE)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| splash_logo(ui, &self.splash_texture, progress));
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    /// No session open: what to do first.
    fn no_session(&mut self, ui: &mut egui::Ui) {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.heading("No session selected");
            ui.label("Insert a goggle or action-cam SD card, or import a folder of recordings.");
            if ui
                .add_enabled(
                    self.ffmpeg_ok && !self.busy_with(&Scope::Library),
                    egui::Button::new("Import Folder…"),
                )
                .clicked()
            {
                self.open_import(None, String::new());
            }
            ui.horizontal(|ui| {
                ui.weak(format!(
                    "Imports go to {} —",
                    self.config.media_root.display()
                ));
                if ui.link("change").clicked() {
                    self.dialog = Some(Dialog::settings(&self.config));
                }
            });
        });
    }

    fn open_import(&mut self, folder: Option<PathBuf>, name: String) {
        self.dialog = Some(Dialog::import(folder, name));
    }

    pub(crate) fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .default_width(268.0)
            .min_width(220.0)
            .max_width(360.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.ready && !self.busy_with(&Scope::Library), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().button_padding = egui::vec2(6.0, 4.0);
                        ui.label(RichText::new("FPV Rush").strong());
                        if ui.button("Import Folder…").clicked() {
                            self.open_import(None, String::new());
                        }
                        let import_options = ui.menu_button("  ", |ui| {
                            if ui.button("Load from destination…").clicked() {
                                self.dialog = Some(Dialog::load());
                                self.dialog_request = Some(self.submit(
                                    "Read destination folders",
                                    true,
                                    |library, _| {
                                        Ok(Reply::Folders(library.import_folder_contents()?))
                                    },
                                ));
                                ui.close();
                            }
                        });
                        paint_metadata_icon(
                            ui,
                            MetadataIcon::Chevron,
                            import_options.response.rect.shrink(2.0),
                        );
                        let settings = ui.menu_button("  ", |ui| {
                            if ui.button("Settings").clicked() {
                                self.dialog = Some(Dialog::settings(&self.config));
                                self.dialog_request =
                                    Some(self.submit("Read cache usage", true, |library, _| {
                                        Ok(Reply::Cache(library.cache_usage()?))
                                    }));
                                ui.close();
                            }
                            if ui.button("About FPV Rush").clicked() {
                                self.show_about = true;
                                ui.close();
                            }
                        });
                        paint_metadata_icon(
                            ui,
                            MetadataIcon::Settings,
                            settings.response.rect.shrink(2.0),
                        );
                        settings.response.on_hover_text("Settings and About");
                    });
                });
                for card in self.cards.clone() {
                    if ui
                        .add_enabled(
                            self.ffmpeg_ok && !self.busy_with(&Scope::Library),
                            egui::Button::new(format!(
                                "Import {} ({})",
                                card.name, card.file_count
                            )),
                        )
                        .clicked()
                    {
                        self.open_import(Some(card.folder), card.name);
                    }
                }
                ui.separator();
                let counts = [
                    self.sessions.len(),
                    self.sessions.iter().map(|s| s.flights.len()).sum(),
                    self.sessions
                        .iter()
                        .flat_map(|s| &s.flights)
                        .map(|f| f.figures.len())
                        .sum(),
                ];
                ui.columns(3, |columns| {
                    for (column, (view, label, count)) in columns.iter_mut().zip([
                        (View::Sessions, "Sessions", counts[0]),
                        (View::Flights, "All flights", counts[1]),
                        (View::Figures, "Figures", counts[2]),
                    ]) {
                        if column
                            .add_sized(
                                [column.available_width(), 42.0],
                                egui::Button::new(format!("{label}\n{count}"))
                                    .selected(self.browse.view == view),
                            )
                            .clicked()
                        {
                            self.browse.view = view;
                            self.browse.page = 0;
                            self.selected.clear();
                            self.selecting = false;
                        }
                    }
                });
                ui.separator();
                if self.browse.view == View::Sessions {
                    ui.small(RichText::new("SESSIONS").strong().weak());
                    egui::ScrollArea::vertical()
                        .id_salt("sessions")
                        .max_height((ui.available_height() - 170.0).max(100.0))
                        .show(ui, |ui| {
                            for session in self.sessions.clone() {
                                let selected = self.browse.session.as_ref() == Some(&session.id);
                                let fill = if selected {
                                    Color32::from_rgb(68, 39, 36)
                                } else {
                                    Color32::TRANSPARENT
                                };
                                let row = egui::Frame::new()
                                    .fill(fill)
                                    .corner_radius(7.0)
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        ui.set_width((ui.available_width() - 12.0).max(1.0));
                                        ui.add(egui::Label::new(&session.name).truncate());
                                        ui.small(
                                            RichText::new(format!(
                                                "{} · {} files · {} flights",
                                                day(&session.date),
                                                session.files.len(),
                                                session.flights.len()
                                            ))
                                            .weak(),
                                        );
                                        let sources = browse::session_sources(&session);
                                        let away = browse::elsewhere(&session);
                                        ui.horizontal_wrapped(|ui| {
                                            if let Some(dvr) = sources.dvr {
                                                metadata_icon(ui, MetadataIcon::Dvr);
                                                let label = ui.small(RichText::new(dvr).weak());
                                                if dvr != "DVR" {
                                                    label.on_hover_text(
                                                        "Cut down to its flights with Reduce…",
                                                    );
                                                }
                                            }
                                            for camera in &sources.cameras {
                                                metadata_icon(ui, MetadataIcon::Camera);
                                                ui.small(RichText::new(camera).weak());
                                            }
                                            if !away.is_empty() {
                                                metadata_icon(ui, MetadataIcon::Folder);
                                                ui.small(RichText::new("elsewhere").weak())
                                                    .on_hover_text(format!(
                                                        "In an earlier import folder: {}",
                                                        away.join(", ")
                                                    ));
                                            }
                                        });
                                        if let Some(state) = browse::cloud_state(&session) {
                                            ui.horizontal_wrapped(|ui| {
                                                metadata_icon(ui, MetadataIcon::Cloud);
                                                ui.small(RichText::new(state).color(crate::ACCENT));
                                            });
                                        }
                                    });
                                let response = row.response.interact(egui::Sense::click());
                                // Its menu on a right-click, as in the session's header.
                                let remade = self.session_busy(&session.id);
                                response.context_menu(|ui| {
                                    ui.add_enabled_ui(!remade, |ui| {
                                        self.session_menu(ui, &session)
                                    });
                                });
                                if response.clicked() {
                                    self.browse.session = Some(session.id);
                                    self.browse.page = 0;
                                    self.selected.clear();
                                    self.selecting = false;
                                }
                            }
                        });
                }
                ui.separator();
                if self.browse.view != View::Sessions {
                    let sessions = &self.sessions;
                    let filter = &mut self.browse.filters_mut().session;
                    let old = filter.clone();
                    egui::ComboBox::from_id_salt("session-filter")
                        .selected_text(
                            sessions
                                .iter()
                                .find(|s| Some(&s.id) == filter.as_ref())
                                .map(|s| s.name.as_str())
                                .unwrap_or("All sessions"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(filter, None, "All sessions");
                            for session in sessions {
                                ui.selectable_value(
                                    filter,
                                    Some(session.id.clone()),
                                    &session.name,
                                );
                            }
                        });
                    if old != *filter {
                        self.browse.page = 0;
                    }
                }
                // A session's flights are not filtered by tag: the panel is
                // All flights' and Figures'.
                if self.browse.view == View::Sessions {
                    if ui.small_button("Reset filters").clicked() {
                        let sort = self.browse.in_session.sort;
                        self.browse.in_session = browse::Filters {
                            sort,
                            ..Default::default()
                        };
                        self.browse.page = 0;
                    }
                    return;
                }
                let mut names = Vec::<String>::new();
                for session in &self.sessions {
                    let labels: Vec<&String> = if self.browse.view == View::Figures {
                        session
                            .flights
                            .iter()
                            .flat_map(|f| f.figures.iter().map(|tag| &tag.label))
                            .collect()
                    } else {
                        session
                            .labels
                            .iter()
                            .chain(session.flights.iter().flat_map(|f| &f.labels))
                            .collect()
                    };
                    for label in labels {
                        if !names.iter().any(|known| browse::same(known, label)) {
                            names.push(label.clone());
                        }
                    }
                }
                names.sort_by_key(|name| name.to_lowercase());
                let chosen = &mut self.browse.filters_mut().labels;
                let mut changed = false;
                egui::ScrollArea::vertical()
                    .id_salt("labels")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for label in names {
                            let mut checked =
                                chosen.iter().any(|selected| browse::same(selected, &label));
                            if ui.checkbox(&mut checked, &label).changed() {
                                if checked {
                                    chosen.push(label);
                                } else {
                                    chosen.retain(|selected| !browse::same(selected, &label));
                                }
                                changed = true;
                            }
                        }
                    });
                if changed {
                    self.browse.page = 0;
                }
                if self.browse.view == View::Flights
                    && ui
                        .checkbox(&mut self.browse.flights.all_labels, "Match all tags")
                        .changed()
                {
                    self.browse.page = 0;
                }
                if ui.small_button("Reset filters").clicked() {
                    let sort = self.browse.filters().sort;
                    *self.browse.filters_mut() = browse::Filters {
                        sort,
                        ..Default::default()
                    };
                    self.browse.page = 0;
                }
            });
    }

    fn session_header(&mut self, ui: &mut egui::Ui, session: &SessionView) {
        ui.horizontal(|ui| {
            ui.heading(&session.name);
            ui.label(day(&session.date));
            ui.add_enabled_ui(!self.session_busy(&session.id), |ui| {
                ui.menu_button("Session…", |ui| {
                    self.session_menu(ui, session);
                });
                if ui
                    .add_enabled(
                        session.segmentable && self.ffmpeg_ok,
                        egui::Button::new("Segment"),
                    )
                    .clicked()
                {
                    self.dialog = Some(Dialog::segment(session));
                }
            });
        });
        if let Some(location) = &session.location
            && !location.is_empty()
        {
            ui.label(location);
        }
        if let Some(notes) = &session.notes
            && !notes.is_empty()
        {
            ui.label(notes);
        }
        ui.label(RichText::new(session.labels.join("  ·  ")).weak());
        ui.collapsing(format!("{} recordings", session.files.len()), |ui| {
            for file in &session.files {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&file.name);
                    metadata_icon(ui, MetadataIcon::Dvr);
                    ui.label(&file.source);
                    metadata_icon(ui, MetadataIcon::Film);
                    ui.label(&file.format);
                    if file.reduced {
                        ui.label("reduced");
                    }
                    if file.cloud {
                        ui.label("Online Only");
                    }
                    if let Some(folder) = &file.elsewhere {
                        ui.label("elsewhere").on_hover_text(folder);
                    }
                    if ui.small_button("Open folder").clicked() {
                        let id = file.id.clone();
                        self.submit("Open recording folder", true, move |library, _| {
                            let path = library.recording_path(&id)?;
                            reveal(&path)?;
                            Ok(Reply::Done("Folder opened".into()))
                        });
                    }
                });
            }
        });
    }

    fn session_menu(&mut self, ui: &mut egui::Ui, session: &SessionView) {
        if ui
            .add_enabled(
                session.files.iter().any(|file| file.cloud),
                egui::Button::new("Download recordings"),
            )
            .clicked()
        {
            self.download_cloud(session.id.clone(), true);
            ui.close();
        }
        if ui
            .add_enabled(
                session.files.iter().any(|file| file.freeable),
                egui::Button::new("Free up cloud space…"),
            )
            .clicked()
        {
            self.dialog = Some(Dialog::free_session(session));
            ui.close();
        }
        if ui.button("Edit session…").clicked() {
            self.dialog = Some(Dialog::session(session));
            ui.close();
        }
        if ui.button("Detection profile…").clicked() {
            self.dialog = Some(Dialog::profile(session));
            ui.close();
        }
        if ui.button("Open folders").clicked() {
            let id = session.id.clone();
            self.submit("Open session folders", true, move |library, _| {
                // One recording per folder, shown selected in it.
                for recording in library.session_folders(&id)? {
                    reveal(&recording)?;
                }
                Ok(Reply::Done("Folders opened".into()))
            });
            ui.close();
        }
        if ui.button("Export labels…").clicked() {
            let id = session.id.clone();
            self.submit("Export labels", false, move |library, _| {
                let Some(path) = rfd::FileDialog::new()
                    .set_file_name("labels.csv")
                    .save_file()
                else {
                    return Ok(Reply::Done("Cancelled".into()));
                };
                Ok(Reply::Done(format!(
                    "Exported {} labels",
                    library.export_labels(&id, &path)?
                )))
            });
            ui.close();
        }
        if ui.button("Move to import folder…").clicked() {
            self.dialog = Some(Dialog::relocate(session));
            let id = session.id.clone();
            self.dialog_request =
                Some(self.submit("Plan session move", true, move |library, _| {
                    Ok(Reply::Relocation(library.relocation_preview(&id)?))
                }));
            ui.close();
        }
        if ui
            .add_enabled(
                session.reducible && self.ffmpeg_ok,
                egui::Button::new("Reduce…"),
            )
            .clicked()
        {
            self.dialog = Some(Dialog::reduce(session));
            let id = session.id.clone();
            self.dialog_request = Some(self.submit("Plan reduction", true, move |library, _| {
                Ok(Reply::Reduce(library.reduce_preview(&id)?))
            }));
            ui.close();
        }
        if ui.button("Remove session…").clicked() {
            self.dialog = Some(Dialog::remove_session(session));
            ui.close();
        }
    }

    pub(crate) fn browser(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.ready { ui.heading("Opening your library…"); ui.spinner(); return; }
            if !self.ffmpeg_ok { ui.colored_label(Color32::LIGHT_RED, "FFmpeg/ffprobe unavailable. Install a complete FPV Rush package or put both tools on PATH."); }
            let session = self.sessions.iter().find(|s| Some(&s.id) == self.browse.session.as_ref()).cloned();
            if self.browse.view == View::Sessions {
                if let Some(session) = &session { self.session_header(ui, session); }
                else { self.no_session(ui); return; }
            } else { ui.heading(if self.browse.view == View::Flights { "All flights" } else { "Figures" }); }
            ui.horizontal_wrapped(|ui| {
                let liked = self.browse.filters().liked;
                if ui.selectable_label(liked, "♡ Liked").clicked() { self.browse.filters_mut().liked = !liked; self.browse.page = 0; }
                if self.browse.view != View::Figures && ui.selectable_label(self.selecting, "Select").clicked() { self.selecting = !self.selecting; if !self.selecting { self.selected.clear(); } }
                ui.label("Sort:"); let old = self.browse.filters().sort;
                let chosen = &mut self.browse.filters_mut().sort;
                egui::ComboBox::from_id_salt("sort").selected_text(format!("{chosen:?}")).show_ui(ui, |ui| {
                    for sort in [Sort::Newest, Sort::Oldest, Sort::Longest] { ui.selectable_value(chosen, sort, format!("{sort:?}")); }
                });
                if old != self.browse.filters().sort { self.browse.page = 0; }
                if ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search titles, sessions, figures…")).changed() { self.browse.page = 0; }
            });
            let mut items = vec![];
            let query = self.search.to_lowercase();
            for (session, flight) in self.browse.flights(&self.sessions) {
                if self.browse.view == View::Figures {
                    for tag in &flight.figures {
                        let names = &self.browse.figures.labels;
                        if !names.is_empty() && !names.iter().any(|name| browse::same(name, &tag.label)) { continue; }
                        if format!("{} {} {}", session.name, flight_name(flight), tag.label).to_lowercase().contains(&query) {
                            items.push((session.clone(), flight.clone(), Some(tag.clone())));
                        }
                    }
                } else if format!("{} {}", session.name, flight_name(flight)).to_lowercase().contains(&query) {
                    items.push((session.clone(), flight.clone(), None));
                }
            }
            if self.browse.view == View::Figures {
                items.sort_by(|a, b| {
                    let start = |item: &(SessionView, FlightView, Option<fpv_library::TagView>)|
                        item.2.as_ref().map(|t| t.start).unwrap_or_default();
                    let length = |item: &(SessionView, FlightView, Option<fpv_library::TagView>)|
                        item.2.as_ref().map(|t| t.end - t.start).unwrap_or_default();
                    match self.browse.filters().sort {
                        Sort::Longest => length(b).total_cmp(&length(a)),
                        sort => {
                            let order = a.0.date.cmp(&b.0.date).then_with(|| a.0.id.cmp(&b.0.id))
                                .then_with(|| a.1.order.cmp(&b.1.order)).then_with(|| start(a).total_cmp(&start(b)));
                            if sort == Sort::Newest { order.reverse() } else { order }
                        }
                    }
                });
            }
            if self.browse.view != View::Figures && self.selecting { ui.horizontal_wrapped(|ui| {
                if ui.small_button("Select all filtered").clicked() { self.selected.extend(items.iter().map(|(_, f, _)| f.id.clone())); }
                if ui.small_button("Clear selection").clicked() { self.selected.clear(); }
                let remade = self.selected.iter().any(|flight| {
                    self.session_of_flight(flight).is_some_and(|session| self.session_busy(session))
                });
                if ui.add_enabled(!self.selected.is_empty() && !remade && self.ffmpeg_ok,
                    egui::Button::new(format!("Export {}…", self.selected.len()))).clicked() {
                    self.dialog = Some(Dialog::bulk(self.selected.iter().cloned().collect()));
                }
            }); }
            let range = page_range(items.len(), &mut self.browse.page, self.browse.page_size);
            ui.separator();
            if items.is_empty() {
                let has_flights = session.as_ref().is_some_and(|s| !s.flights.is_empty());
                let (title, detail, action) = browse::empty_state(
                    self.browse.view, has_flights, self.browse.filters(), !self.search.is_empty());
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.heading(title);
                    if !detail.is_empty() { ui.label(detail); }
                    match action {
                        browse::EmptyAction::Segment => {
                            if let Some(session) = session.as_ref().filter(|s| s.segmentable)
                                && ui.add_enabled(self.ffmpeg_ok && !self.session_busy(&session.id), egui::Button::new("Segment")).clicked() {
                                self.dialog = Some(Dialog::segment(session));
                            }
                        }
                        browse::EmptyAction::ResetFilters => {
                            if ui.button("Reset filters").clicked() {
                                let sort = self.browse.filters().sort;
                                *self.browse.filters_mut() = browse::Filters { sort, ..Default::default() };
                                self.search.clear();
                                self.browse.page = 0;
                            }
                        }
                        browse::EmptyAction::Nothing => {}
                    }
                });
                return;
            }
            egui::ScrollArea::vertical().id_salt("flight-grid").auto_shrink([false, false]).max_height((ui.available_height() - 28.0 - 3.0 * ui.spacing().item_spacing.y).max(0.0)).show(ui, |ui| {
                let columns = ((ui.available_width() / 270.0) as usize).clamp(1, 6);
                for row in items[range.clone()].chunks(columns) {
                    ui.columns(columns, |uis| {
                        for (ui, (session, flight, figure)) in uis.iter_mut().zip(row) { self.flight_card(ui, session, flight, figure.as_ref()); }
                    }); ui.add_space(12.0);
                }
            });
            ui.separator();
            pagination_footer(ui, &mut self.browse, items.len(), &range);
        });
    }

    fn flight_card(
        &mut self,
        ui: &mut egui::Ui,
        session: &SessionView,
        flight: &FlightView,
        figure: Option<&fpv_library::TagView>,
    ) {
        egui::Frame::NONE.show(ui, |ui| {
            let width = ui.available_width();
            let key = format!("poster:{}", flight.cache_key);
            let mut path = self
                .thumbnails
                .get(&key)
                .cloned()
                .or_else(|| flight.clips.first().and_then(|clip| clip.thumbnail.clone()));
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, width * 0.75), egui::Sense::click());
            let remade = self.session_busy(&session.id);
            response.context_menu(|ui| {
                ui.add_enabled_ui(!remade, |ui| self.flight_menu(ui, &session.id, flight));
            });
            ui.painter()
                .rect_filled(rect, 4.0, Color32::from_rgb(22, 28, 37));
            // Under the pointer, the strip's frame there and its time; on a
            // figure's card, the frame nearest the figure's moment.
            let strip = crate::strips::STRIP;
            let pointer = response.hover_pos().filter(|_| !flight.cloud);
            let hover_time = pointer.map(|pointer| {
                ((pointer.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64 * flight.duration
            });
            let wanted = match (hover_time, figure) {
                (Some(at), _) => Some(browse::strip_index(at, flight.duration, strip)),
                (None, Some(figure)) if !flight.cloud => {
                    Some(browse::strip_index(figure.start, flight.duration, strip))
                }
                _ => None,
            };
            if let Some(index) = wanted {
                let frame = |i: usize| format!("strip:{}:{i}", flight.cache_key);
                if let Some(known) = self.thumbnails.get(&frame(index)) {
                    path = Some(known.clone());
                } else {
                    // While it is made, the nearest frame already here.
                    if let Some(near) = browse::nearest_known(index, strip, |i| {
                        self.thumbnails.contains_key(&frame(i))
                    }) {
                        path = self.thumbnails.get(&frame(near)).cloned();
                    }
                    if self.ffmpeg_ok
                        && self.tasks.iter().filter(|t| t.running).count() < 12
                        && self.requested.insert(frame(index))
                    {
                        let (id, key) = (flight.id.clone(), frame(index));
                        self.submit("Hover frame", true, move |library, _| {
                            Ok(Reply::Image {
                                thumbnail: library.filmstrip_frame(&id, index, strip)?,
                                key,
                            })
                        });
                    }
                }
            }
            let mut shown = false;
            if let Some(path) = path {
                if let Some(texture) = self.images.get(ui.ctx(), &path) {
                    let size = texture.size_vec2();
                    let scale = (rect.width() / size.x).min(rect.height() / size.y);
                    let target = egui::Rect::from_center_size(rect.center(), size * scale);
                    ui.painter().image(
                        texture.id(),
                        target,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    shown = true;
                } else if self.images.failed(&path)
                    && !flight.cloud
                    && self.ffmpeg_ok
                    && self.requested.insert(format!("repair:{key}"))
                    && let Some(clip) = flight.clips.first()
                {
                    let id = clip.id.clone();
                    let key = key.clone();
                    self.submit("Rebuild poster", true, move |library, _| {
                        Ok(Reply::Image {
                            key,
                            thumbnail: library.remake_poster(&id)?,
                        })
                    });
                }
            } else if !flight.cloud
                && self.ffmpeg_ok
                && self.requested.insert(key.clone())
                && let Some(clip) = flight.clips.first()
            {
                let id = clip.id.clone();
                self.submit("Poster", true, move |library, _| {
                    Ok(Reply::Image {
                        key,
                        thumbnail: library.poster(&id)?,
                    })
                });
            }
            if !shown {
                // No picture yet: a film frame, not an empty box.
                paint_metadata_icon(
                    ui,
                    MetadataIcon::Film,
                    egui::Rect::from_center_size(rect.center(), egui::vec2(40.0, 34.0)),
                );
            }
            if let Some(pointer) = pointer {
                // The playhead under the pointer.
                let x = pointer.x.clamp(rect.left(), rect.right());
                ui.painter().line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(2.0_f32, ACCENT),
                );
            }
            let heart_rect = egui::Rect::from_min_size(
                rect.right_top() + egui::vec2(-30.0, 4.0),
                egui::vec2(26.0, 24.0),
            );
            let mut like_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("like", &flight.id))
                    .max_rect(heart_rect),
            );
            like_ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);
            let remade = self.session_busy(&session.id);
            let heart = like_ui.add_enabled(
                !remade,
                egui::Button::new(
                    RichText::new(if flight.liked { "♥" } else { "♡" }).color(Color32::WHITE),
                )
                .frame(false)
                .min_size(heart_rect.size()),
            );
            if heart.clicked() && !remade {
                let id = flight.id.clone();
                self.submit("Like flight", true, move |library, _| {
                    library.toggle_like(&id)?;
                    Ok(Reply::Done("Like updated".into()))
                });
            }
            if let Some(figure) = figure {
                thumbnail_badge(ui, rect, &figure.label, ACCENT, false);
                if !figure.marker {
                    thumbnail_badge(
                        ui,
                        rect,
                        &format!("{:.1} s", figure.end - figure.start),
                        Color32::from_black_alpha(190),
                        true,
                    );
                }
            } else if !flight.figures.is_empty() {
                thumbnail_badge(
                    ui,
                    rect,
                    &format!(
                        "{} figure{}",
                        flight.figures.len(),
                        if flight.figures.len() == 1 { "" } else { "s" }
                    ),
                    Color32::from_black_alpha(190),
                    false,
                );
            }
            if response.clicked() && !heart.clicked() {
                self.play(flight, figure.map(|tag| tag.start).unwrap_or_default());
            }
            ui.horizontal(|ui| {
                if figure.is_none() && self.selecting {
                    let mut selected = self.selected.contains(&flight.id);
                    if ui.checkbox(&mut selected, "").changed() {
                        if selected {
                            self.selected.insert(flight.id.clone());
                        } else {
                            self.selected.remove(&flight.id);
                        }
                    }
                }
                self.card_title(ui, flight, remade);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.flight_actions(ui, &session.id, flight);
                    if figure.is_none() {
                        let export = ui
                            .add_enabled_ui(!remade, |ui| {
                                let (area, response) = ui.allocate_exact_size(
                                    egui::vec2(20.0, 18.0),
                                    egui::Sense::click(),
                                );
                                paint_metadata_icon(ui, MetadataIcon::Export, area);
                                response.on_hover_text("Export…")
                            })
                            .inner;
                        if export.clicked() {
                            let name = session.name.clone();
                            self.dialog = Some(Dialog::export(&name, flight));
                        }
                        // The time under the pointer while it scrubs.
                        ui.small(
                            RichText::new(clock(hover_time.unwrap_or(flight.duration))).weak(),
                        );
                    }
                });
            });
            if self.browse.view != View::Sessions {
                ui.small(
                    RichText::new(format!("{} · {}", session.name, day(&session.date))).weak(),
                );
            }
            if let Some(figure) = figure {
                ui.small(
                    RichText::new(if figure.marker {
                        clock(figure.start)
                    } else {
                        format!("{} – {}", clock(figure.start), clock(figure.end))
                    })
                    .weak(),
                );
            } else {
                ui.horizontal_wrapped(|ui| {
                    metadata_icon(
                        ui,
                        if flight.action_cam {
                            MetadataIcon::Camera
                        } else {
                            MetadataIcon::Dvr
                        },
                    );
                    ui.small(
                        RichText::new(format!(
                            "{}{}",
                            flight.source,
                            if flight.reduced { " · reduced" } else { "" }
                        ))
                        .weak(),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    metadata_icon(ui, MetadataIcon::Film);
                    ui.small(RichText::new(&flight.format).weak());
                });
                if let Some(parts) = browse::parts_label(flight) {
                    ui.small(RichText::new(parts).weak());
                }
                if flight.cloud {
                    ui.horizontal_wrapped(|ui| {
                        metadata_icon(ui, MetadataIcon::Cloud);
                        ui.small(RichText::new("In the cloud").color(crate::ACCENT))
                            .on_hover_text("Online Only: downloads when played");
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    for (label, inherited) in browse::card_labels(flight, session) {
                        // A session's tag is outlined, the flight's own filled.
                        let chip = egui::Frame::new()
                            .fill(if inherited {
                                Color32::TRANSPARENT
                            } else {
                                Color32::from_rgb(35, 37, 43)
                            })
                            .stroke(if inherited {
                                egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color())
                            } else {
                                egui::Stroke::NONE
                            })
                            .corner_radius(9.0)
                            .inner_margin(egui::Margin::symmetric(6, 1))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&label).size(11.0));
                            });
                        if inherited {
                            chip.response.on_hover_text("From the session");
                        }
                    }
                });
            }
        });
    }

    /// A card's title; a double-click edits it in place, Enter saves it.
    fn card_title(&mut self, ui: &mut egui::Ui, flight: &FlightView, remade: bool) {
        if let Some((id, text)) = &mut self.title_edit
            && *id == flight.id
        {
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(140.0));
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if edit.lost_focus() {
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    let (id, title) = (id.clone(), text.trim().to_owned());
                    self.submit("Edit flight title", true, move |library, _| {
                        library.set_flight_title(&id, &title)?;
                        Ok(Reply::Done("Title updated".into()))
                    });
                }
                self.title_edit = None;
            }
            return;
        }
        let title = ui.add(
            egui::Label::new(RichText::new(flight_name(flight)).strong().size(16.0))
                .truncate()
                .sense(egui::Sense::click()),
        );
        if title.double_clicked() && !remade {
            self.title_edit = Some((flight.id.clone(), flight.title.clone().unwrap_or_default()));
        }
        title.on_hover_text("Double-click to rename");
    }

    fn flight_actions(&mut self, ui: &mut egui::Ui, session: &str, flight: &FlightView) {
        // A flight its session's task is remaking cannot be acted on meanwhile.
        ui.add_enabled_ui(!self.session_busy(session), |ui| {
            ui.menu_button("…", |ui| self.flight_menu(ui, session, flight));
        });
    }

    /// A card's actions: behind its "…" and on a right-click alike.
    fn flight_menu(&mut self, ui: &mut egui::Ui, session: &str, flight: &FlightView) {
        if ui
            .add_enabled(flight.cloud, egui::Button::new("Download recordings"))
            .clicked()
        {
            self.download_cloud(flight.id.clone(), false);
            ui.close();
        }
        if ui
            .add_enabled(flight.freeable, egui::Button::new("Free up cloud space…"))
            .clicked()
        {
            self.dialog = Some(Dialog::free_flight(flight));
            ui.close();
        }
        if ui.button("Edit title…").clicked() {
            self.dialog = Some(Dialog::title(flight));
            ui.close();
        }
        if ui.button("Edit boundaries…").clicked() {
            self.viewer = None;
            self.dialog = Some(Dialog::editor(flight));
            ui.close();
        }
        if ui.button("Export…").clicked() {
            let name = self
                .sessions
                .iter()
                .find(|s| s.id == session)
                .map_or("Session", |s| s.name.as_str());
            self.dialog = Some(Dialog::export(name, flight));
            ui.close();
        }
        if ui.button("Delete flight…").clicked() {
            self.dialog = Some(Dialog::delete_flight(flight));
            let id = flight.id.clone();
            self.dialog_request = Some(self.submit(
                "Plan flight deletion",
                true,
                move |library, _| Ok(Reply::Deletion(library.flight_deletion(&id)?)),
            ));
            ui.close();
        }
    }

    pub(crate) fn player_window(&mut self, ctx: &egui::Context) {
        let Some(mut viewer) = self.viewer.take() else {
            return;
        };
        self.notices.extend(viewer.player.warnings());
        if let Some(frame) = viewer.player.frame() {
            let image = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb);
            if let Some(texture) = &mut viewer.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                viewer.texture =
                    Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR));
            }
        }
        let mut open = true;
        let fullscreen = viewer.fullscreen;
        let expanded = fullscreen || viewer.window.maximized;
        let bounds = ctx.content_rect().shrink(8.0);
        let frame = egui::Frame::window(&ctx.style());
        let heading_font = egui::TextStyle::Heading.resolve(&ctx.style());
        let title_height = ctx
            .fonts_mut(|fonts| fonts.row_height(&heading_font))
            .max(ctx.style().spacing.interact_size.y)
            + frame.inner_margin.sum().y
            + frame.stroke.width;
        let decorations = frame.total_margin().sum() + egui::vec2(0.0, title_height);
        let content_limit = (bounds.size() - decorations).max(egui::vec2(1.0, 1.0));
        let mut window = egui::Window::new(flight_name(&viewer.flight))
            .id(egui::Id::new("player"))
            .collapsible(false)
            .constrain_to(bounds)
            .max_size(content_limit)
            .vscroll(true)
            .resizable(!expanded);
        if fullscreen {
            // Paint on the player's own layer even during egui's invisible
            // sizing pass, so no frame exposes the library around its contents.
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("player"),
            ))
            .rect_filled(ctx.content_rect(), 0.0, Color32::BLACK);
            window = window
                .title_bar(false)
                .fade_in(false)
                .fade_out(false)
                .constrain_to(ctx.content_rect())
                .frame(egui::Frame::NONE.fill(Color32::BLACK))
                .fixed_rect(ctx.content_rect());
        } else if viewer.window.maximized {
            window = window
                .open(&mut open)
                .fixed_pos(bounds.min)
                .fixed_size(content_limit);
        } else {
            window = window.open(&mut open);
            // Reset persisted egui geometry on opening, and restore the floating
            // rectangle after maximizing/fullscreen without remembering it on reopen.
            if viewer.window.normal_rect.is_none() || viewer.window.was_expanded {
                let compact_size = (bounds.size() * 0.82).min(egui::vec2(1000.0, 620.0));
                let rect = viewer
                    .window
                    .normal_rect
                    .unwrap_or_else(|| egui::Rect::from_center_size(bounds.center(), compact_size));
                let size = rect.size().min(bounds.size());
                let position = rect.min.clamp(bounds.min, bounds.max - size);
                // Area otherwise constrains the new position using last frame's
                // expanded size before Resize has applied the restored size.
                window = window.constrain(false).current_pos(position).fixed_size(
                    (size - decorations)
                        .max(egui::vec2(1.0, 1.0))
                        .min(content_limit),
                );
            }
        }
        let mut close_requested = false;
        let shown = window.show(ctx, |ui| {
            if fullscreen && !ui.visuals().dark_mode {
                // The fullscreen backing is black even when the OS follows a
                // light theme. Scope contrasting controls to this player only.
                let mut visuals = egui::Visuals::dark();
                visuals.override_text_color = Some(Color32::from_rgb(220, 223, 230));
                visuals.selection.bg_fill = ACCENT;
                visuals.widgets.active.bg_fill = ACCENT;
                visuals.hyperlink_color = ACCENT;
                *ui.visuals_mut() = visuals;
            }
            self.player_contents(
                ui,
                &mut viewer,
                &mut close_requested,
                ctx,
                if fullscreen {
                    ctx.content_rect().height()
                } else {
                    ui.available_height().min(content_limit.y)
                },
            )
        });
        if !expanded && let Some(shown) = shown {
            viewer.window.normal_rect = Some(shown.response.rect);
        }
        viewer.window.was_expanded = expanded;
        if close_requested {
            open = false;
        }
        if viewer.fullscreen && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            viewer.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if open {
            if !viewer.player.paused() && !viewer.player.ended() || viewer.player.buffering() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            self.viewer = Some(viewer);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }

    fn player_contents(
        &mut self,
        ui: &mut egui::Ui,
        viewer: &mut Viewer,
        close_requested: &mut bool,
        ctx: &egui::Context,
        content_height: f32,
    ) {
        let header = ui.horizontal_wrapped(|ui| {
            if !viewer.fullscreen
                && ui
                    .button(if viewer.window.maximized {
                        "Restore"
                    } else {
                        "Maximize"
                    })
                    .clicked()
            {
                viewer.window.maximized = !viewer.window.maximized;
            }
            if ui
                .button(if viewer.fullscreen {
                    "Exit full screen"
                } else {
                    "Full screen"
                })
                .clicked()
            {
                viewer.fullscreen = !viewer.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(viewer.fullscreen));
            }
            if ui.button("×").on_hover_text("Close player").clicked() {
                *close_requested = true;
            }
            ui.label(flight_name(&viewer.flight));
        });
        let separator = ui.separator();
        let available = egui::vec2(
            ui.available_width(),
            (content_height
                - header.response.rect.height()
                - separator.rect.height()
                - 2.0 * ui.spacing().item_spacing.y)
                .max(1.0),
        );
        let side_width = 300.0_f32.min(available.x * 0.42);
        let left_width = (available.x - side_width - 12.0).max(260.0);

        let show_video = |ui: &mut egui::Ui, viewer: &mut Viewer| {
            let button_height = ui
                .spacing()
                .interact_size
                .y
                .max(
                    ui.text_style_height(&egui::TextStyle::Button)
                        + 2.0 * ui.spacing().button_padding.y,
                )
                .max(26.0)
                + ui.spacing().item_spacing.y;
            // Use the actual remaining column height. In scrolling layouts it
            // can be unbounded, so retain the finite player-content budget.
            let height = (ui.available_height().min(available.y) - button_height).max(1.0);
            let (canvas, video_response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), height),
                egui::Sense::click(),
            );
            ui.painter().rect_filled(canvas, 4.0, Color32::BLACK);
            let video = if let Some(texture) = &viewer.texture {
                let source = texture.size_vec2();
                let scale = (canvas.width() / source.x).min(canvas.height() / source.y);
                let video = egui::Rect::from_center_size(canvas.center(), source * scale);
                ui.painter().image(
                    texture.id(),
                    video,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                video
            } else {
                canvas
            };
            player_volume_overlay(ui, viewer, video);
            let row_height = ui.spacing().interact_size.y.max(
                ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y,
            );
            let rows = 1.0;
            let controls_height =
                (rows * row_height + (rows - 1.0) * ui.spacing().item_spacing.y + 12.0)
                    .min(video.height());
            let controls = egui::Rect::from_min_max(
                egui::pos2(video.left(), video.bottom() - controls_height),
                video.max,
            );
            let mut overlay = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("video-overlay")
                    .max_rect(controls),
            );
            overlay.set_clip_rect(ui.clip_rect().intersect(video));
            egui::Frame::new()
                .inner_margin(6.0)
                .show(&mut overlay, |ui| {
                    ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [45.0, 18.0],
                            egui::Label::new(clock(viewer.player.position())),
                        );
                        let mut time = viewer.seek_time.unwrap_or_else(|| viewer.player.position());
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - 45.0 - 2.0 * ui.spacing().item_spacing.x)
                                .max(1.0);
                        let seek = ui.add(
                            egui::Slider::new(&mut time, 0.0..=viewer.player.duration())
                                .show_value(false),
                        );
                        if seek.dragged() {
                            viewer.seek_time = Some(time);
                        }
                        if seek.drag_stopped() || (seek.changed() && !seek.dragged()) {
                            viewer.player.seek(time);
                            viewer.seek_time = None;
                        }
                        ui.add_sized(
                            [45.0, 18.0],
                            egui::Label::new(clock(viewer.player.duration())),
                        );
                    });
                });
            if video_response.clicked() {
                viewer.player.pause(!viewer.player.paused());
            }
            ui.horizontal_wrapped(|ui| {
                if let Some((part, of)) = viewer.player.part() {
                    let what = if viewer.flight.action_cam {
                        "chapter"
                    } else {
                        "recording"
                    };
                    ui.small(format!("{what} {part}/{of}"));
                }
                if !viewer.fullscreen
                    && ui
                        .button(if viewer.player.paused() {
                            "Play"
                        } else {
                            "Pause"
                        })
                        .clicked()
                {
                    viewer.player.pause(!viewer.player.paused());
                }
                player_speaker(ui, viewer);
                if viewer.muted {
                    ui.small("No sound");
                }
                if viewer.player.ended() && ui.button("Replay").clicked() {
                    viewer.player.seek(0.0);
                    viewer.player.pause(false);
                }
                if viewer.player.buffering() {
                    ui.spinner();
                    ui.label("Buffering…");
                }
            });
        };
        let show_metadata = |this: &mut Self, ui: &mut egui::Ui, viewer: &mut Viewer| {
            ui.heading("Tags");
            if viewer.flight.labels.is_empty() {
                ui.weak("No tags yet.");
            }
            ui.horizontal_wrapped(|ui| {
                for label in viewer.flight.labels.clone() {
                    if ui.button(format!("{label} ×")).clicked() {
                        let id = viewer.flight.id.clone();
                        this.submit("Remove flight tag", true, move |library, _| {
                            library.remove_flight_label(&id, &label)?;
                            Ok(Reply::Done("Tag removed".into()))
                        });
                    }
                }
            });
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut viewer.label)
                        .hint_text("Tag the flight: freestyle, gates…")
                        .desired_width((ui.available_width() - 80.0).max(20.0)),
                );
                let enter =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if (ui.button("Add").clicked() || enter) && !viewer.label.trim().is_empty() {
                    let id = viewer.flight.id.clone();
                    let label = std::mem::take(&mut viewer.label);
                    this.submit("Add flight tag", true, move |library, _| {
                        library.add_flight_label(&id, &label)?;
                        Ok(Reply::Done("Tag added".into()))
                    });
                }
            });
            let names = this.sessions.iter().flat_map(|session| {
                session
                    .labels
                    .iter()
                    .chain(session.flights.iter().flat_map(|flight| &flight.labels))
                    .map(String::as_str)
            });
            let found = browse::suggest(names, &viewer.label, &viewer.flight.labels);
            suggestions(ui, &mut viewer.label, found);
            ui.separator();
            ui.heading("Figures");
            if viewer.flight.figures.is_empty() {
                ui.weak("No figures yet.");
            }
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 170.0).max(100.0))
                .show(ui, |ui| {
                    for tag in &viewer.flight.figures {
                        ui.horizontal(|ui| {
                            if ui
                                .button(format!("{}  {}", clock(tag.start), tag.label))
                                .clicked()
                            {
                                viewer.player.seek(tag.start);
                            }
                            if !tag.marker {
                                ui.label(format!("– {}", clock(tag.end)));
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text("Delete figure")
                                .clicked()
                            {
                                let id = tag.id.clone();
                                this.submit("Delete figure", true, move |library, _| {
                                    library.delete_tag(&id)?;
                                    Ok(Reply::Done("Figure deleted".into()))
                                });
                            }
                        });
                    }
                });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Mark In (I)").clicked() {
                    viewer.mark = Some(viewer.player.position());
                }
                if let Some(mark) = viewer.mark {
                    ui.label(RichText::new(format!("● Marked at {}", clock(mark))).color(ACCENT));
                    if ui.small_button("Cancel").clicked() {
                        viewer.mark = None;
                    }
                }
            });
            // The button first, from the right: "Add Range to 0:12.3" is
            // wider than "Add Point", and the field takes what is left.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let add = browse::add_figure_label(viewer.mark, viewer.player.position());
                    let clicked = ui.button(add).clicked();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut viewer.figure)
                            .hint_text("Figure name (split-s, power loop…)")
                            .desired_width(ui.available_width().max(20.0)),
                    );
                    let enter = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if (clicked || enter) && !viewer.figure.trim().is_empty() {
                        let id = viewer.flight.id.clone();
                        let label = std::mem::take(&mut viewer.figure);
                        let end = viewer.player.position();
                        let start = viewer.mark.take().unwrap_or(end);
                        this.submit("Add figure", true, move |library, _| {
                            library.add_tag(&id, &label, start, end)?;
                            Ok(Reply::Done("Figure added".into()))
                        });
                    }
                })
            });
            let names = this
                .sessions
                .iter()
                .flat_map(|session| &session.flights)
                .flat_map(|flight| &flight.figures)
                .map(|figure| figure.label.as_str());
            let found = browse::suggest(names, &viewer.figure, &[]);
            suggestions(ui, &mut viewer.figure, found);
            ui.add(
                egui::Label::new(
                    RichText::new("Mark In (or I) at the start, play to the end, name it, Enter.")
                        .weak(),
                )
                .wrap(),
            );
        };

        if available.x < 720.0 {
            show_video(ui, viewer);
            ui.separator();
            show_metadata(self, ui, viewer);
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, available.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| show_video(ui, viewer),
                );
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(side_width, available.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| show_metadata(self, ui, viewer),
                );
            });
        }
        if !ui.ctx().wants_keyboard_input() {
            if ui.input(|input| input.key_pressed(egui::Key::Space)) {
                viewer.player.pause(!viewer.player.paused());
            }
            if ui.input(|input| input.key_pressed(egui::Key::I)) {
                viewer.mark = Some(viewer.player.position());
            }
            if ui.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
                viewer
                    .player
                    .seek((viewer.player.position() - 5.0).max(0.0));
            }
            if ui.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
                viewer.player.seek(viewer.player.position() + 5.0);
            }
        }
    }

    pub(crate) fn task_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("tasks").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let shown = || self.tasks.iter().filter(|task| !task.quiet || task.failed);
                let running = shown().filter(|task| task.running).count();
                let failed = shown().filter(|task| task.failed).count();
                let text = RichText::new(crate::taskbar::summary(running, failed));
                let text = if running == 0 && failed > 0 {
                    text.color(Color32::LIGHT_RED)
                } else {
                    text
                };
                if ui.button(text).clicked() {
                    self.show_tasks = !self.show_tasks;
                }
                if let Some(progress) = crate::taskbar::overall(
                    shown()
                        .filter(|task| task.running)
                        .map(|task| (task.done, task.total)),
                ) {
                    ui.add(egui::ProgressBar::new(progress).desired_width(120.0));
                }
                if let Some(task) = self
                    .tasks
                    .iter()
                    .rev()
                    .find(|task| task.running && !task.quiet)
                {
                    ui.spinner();
                    ui.label(&task.label);
                    ui.label(&task.line);
                }
                if self.strips.running() {
                    ui.label(
                        RichText::new(format!("Hover strips · {} made", self.strips.made)).weak(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let unread = self.notices.unread();
                    let text = if unread > 0 {
                        RichText::new(format!("Notifications ({unread} new)"))
                            .color(Color32::LIGHT_RED)
                    } else {
                        RichText::new("Notifications")
                    };
                    if ui.button(text).clicked() {
                        self.show_notices = !self.show_notices;
                        self.notices.mark_read();
                    }
                });
            });
            if self.show_notices {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Notifications");
                    if !self.notices.items().is_empty() && ui.small_button("Clear all").clicked() {
                        self.notices.clear();
                    }
                });
                if self.notices.items().is_empty() {
                    ui.weak("Nothing went wrong.");
                }
                let mut dismissed = None;
                egui::ScrollArea::vertical()
                    .id_salt("notices")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (index, notice) in self.notices.items().iter().enumerate() {
                            ui.horizontal_wrapped(|ui| {
                                ui.weak(&notice.at);
                                ui.colored_label(Color32::LIGHT_RED, &notice.text);
                                if notice.times > 1 {
                                    ui.weak(format!("{} times", notice.times));
                                }
                                if ui.small_button("Dismiss").clicked() {
                                    dismissed = Some(index);
                                }
                            });
                        }
                    });
                if let Some(index) = dismissed {
                    self.notices.dismiss(index);
                }
            }
            if self.show_tasks {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Tasks");
                    if self.tasks.iter().any(|task| !task.running)
                        && ui.small_button("Clear finished").clicked()
                    {
                        self.tasks.retain(|task| task.running);
                    }
                });
                if !self.tasks.iter().any(|task| !task.quiet || task.failed) {
                    ui.weak("Nothing has run yet.");
                }
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for task in self
                            .tasks
                            .iter()
                            .rev()
                            .filter(|task| !task.quiet || task.failed)
                        {
                            ui.horizontal_wrapped(|ui| {
                                if task.running {
                                    ui.spinner();
                                } else if task.failed {
                                    ui.colored_label(Color32::LIGHT_RED, "✖");
                                } else {
                                    ui.colored_label(Color32::LIGHT_GREEN, "✔");
                                }
                                ui.label(&task.label);
                                if task.failed {
                                    ui.colored_label(Color32::LIGHT_RED, &task.line);
                                } else {
                                    ui.label(&task.line);
                                }
                                if task.running && task.total > 0 {
                                    ui.add(
                                        egui::ProgressBar::new(
                                            task.done as f32 / task.total as f32,
                                        )
                                        .desired_width(130.0)
                                        .show_percentage(),
                                    );
                                }
                                let end = task.ended.unwrap_or_else(std::time::Instant::now);
                                ui.weak(crate::taskbar::elapsed(
                                    end.saturating_duration_since(task.started).as_secs_f64(),
                                ));
                            });
                            if !task.log.is_empty() {
                                ui.collapsing(
                                    format!("Details: {} #{}", task.label, task.id),
                                    |ui| {
                                        for line in &task.log {
                                            ui.small(line);
                                        }
                                    },
                                );
                            }
                        }
                    });
            }
        });
    }
}

impl App {
    fn download_cloud(&mut self, id: String, session: bool) {
        self.submit("Download from cloud", false, move |library, progress| {
            let report = if session {
                library.download_session(&id, |step| {
                    progress.count(
                        if step.done == step.total {
                            step.files
                        } else {
                            step.file.saturating_sub(1)
                        },
                        step.files,
                        step.line,
                    )
                })?
            } else {
                library.download_flight(&id, |step| {
                    progress.count(
                        if step.done == step.total {
                            step.files
                        } else {
                            step.file.saturating_sub(1)
                        },
                        step.files,
                        step.line,
                    )
                })?
            };
            if !report.failed.is_empty() {
                return Err(fpv_library::LibraryError::Invalid(format!(
                    "{} downloaded; failed: {:?}",
                    report.downloaded, report.failed
                )));
            }
            Ok(Reply::Done(format!(
                "Downloaded {} recordings",
                report.downloaded
            )))
        });
    }
}

#[cfg(test)]
mod tests;
