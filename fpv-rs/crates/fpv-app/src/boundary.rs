//! Source-frame trim handles and a bounded native source preview.
use crate::{ACCENT, browse::clock, player::Player};
use eframe::egui::{self, Color32};
use fpv_library::{ClipView, NativeSegment};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Start,
    End,
}

fn set_edge(clip: &mut ClipView, edge: Edge, frame: i64) -> i64 {
    let limit = clip.last_frame.saturating_add(1).max(1);
    match edge {
        Edge::Start => {
            clip.start_frame = frame.clamp(0, clip.end_frame.saturating_sub(1).min(limit - 1));
            clip.start_frame
        }
        Edge::End => {
            clip.end_frame = frame.clamp(clip.start_frame.saturating_add(1), limit);
            clip.end_frame - 1
        }
    }
}

/// Moves one edge by `frames` (negative: earlier), within the recording and
/// never across the other edge; returns the frame to preview.
pub(crate) fn nudge(clip: &mut ClipView, edge: Edge, frames: i64) -> i64 {
    let at = match edge {
        Edge::Start => clip.start_frame,
        Edge::End => clip.end_frame,
    };
    set_edge(clip, edge, at.saturating_add(frames))
}

/// The steps the nudge buttons take, in frames at the recording's own rate.
pub(crate) fn nudge_steps(fps: f64) -> [(&'static str, i64); 6] {
    let frames = |seconds: f64| ((seconds * fps).round() as i64).max(1);
    [
        ("−1 s", -frames(1.0)),
        ("−¼ s", -frames(0.25)),
        ("−1 fr", -1),
        ("+1 fr", 1),
        ("+¼ s", frames(0.25)),
        ("+1 s", frames(1.0)),
    ]
}

/// The edge being nudged, its exact time and frame, the buttons, and Reset to
/// what is saved — the Tauri editor's controls. Returns a frame to preview.
pub(crate) fn nudge_controls(
    ui: &mut egui::Ui,
    clip: &mut ClipView,
    edge: &mut Edge,
    saved: Option<(i64, i64)>,
) -> Option<i64> {
    let mut preview = None;
    ui.horizontal(|ui| {
        ui.selectable_value(edge, Edge::Start, "Start");
        ui.selectable_value(edge, Edge::End, "End");
        let frame = match edge {
            Edge::Start => clip.start_frame,
            Edge::End => clip.end_frame,
        };
        ui.label(format!("{:.3} s · frame {frame}", frame as f64 / clip.fps));
    });
    ui.horizontal(|ui| {
        for (label, frames) in nudge_steps(clip.fps) {
            if ui.button(label).clicked() {
                preview = Some(nudge(clip, *edge, frames));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.small(format!(
            "{:.3} s to {:.3} s at {:.3} fps",
            clip.start_frame as f64 / clip.fps,
            clip.end_frame as f64 / clip.fps,
            clip.fps
        ));
        if let Some((start, end)) = saved
            && (start, end) != (clip.start_frame, clip.end_frame)
        {
            ui.colored_label(ACCENT, "changed — not saved");
            if ui.small_button("Reset").clicked() {
                clip.start_frame = start;
                clip.end_frame = end;
                preview = Some(start);
            }
        }
    });
    ui.small("Detected on a 0.25 s grid — nudge to the exact frame.");
    preview
}

pub(crate) fn trim_timeline(ui: &mut egui::Ui, clip: &mut ClipView) -> Option<i64> {
    let mut preview = None;
    ui.horizontal(|ui| {
        ui.label("Start");
        if ui
            .add(egui::DragValue::new(&mut clip.start_frame).range(0..=clip.end_frame - 1))
            .changed()
        {
            preview = Some(clip.start_frame);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut clip.end_frame)
                        .range(clip.start_frame + 1..=clip.last_frame.saturating_add(1)),
                )
                .changed()
            {
                preview = Some(clip.end_frame - 1);
            }
            ui.label("End (exclusive)");
        });
    });
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 54.0), egui::Sense::hover());
    let rail = rect.shrink2(egui::vec2(14.0, 8.0));
    let limit = clip.last_frame.saturating_add(1).max(1) as f64;
    let x = |frame: i64| rail.left() + (frame as f64 / limit) as f32 * rail.width();
    let left = x(clip.start_frame);
    let right = x(clip.end_frame);
    ui.painter().rect_filled(rail, 3.0, Color32::from_gray(50));
    ui.painter().rect_filled(
        egui::Rect::from_x_y_ranges(left..=right, rail.y_range()),
        2.0,
        ACCENT.gamma_multiply(0.5),
    );
    for (edge, position) in [(Edge::Start, left), (Edge::End, right)] {
        // Outward handles remain individually reachable even for a one-frame range.
        let handle = egui::Rect::from_min_max(
            egui::pos2(
                position + if edge == Edge::Start { -12.0 } else { 0.0 },
                rail.top() - 3.0,
            ),
            egui::pos2(
                position + if edge == Edge::Start { 0.0 } else { 12.0 },
                rail.bottom() + 3.0,
            ),
        );
        let response = ui
            .interact(
                handle,
                ui.id().with(("trim-edge", edge == Edge::End)),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(if edge == Edge::Start {
                "Drag start boundary"
            } else {
                "Drag end boundary"
            });
        ui.painter().rect_filled(
            handle,
            2.0,
            if response.hovered() || response.dragged() {
                Color32::WHITE
            } else {
                ACCENT
            },
        );
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            // Keep the grab offset so pressing a handle never jumps the boundary.
            let offset_id = response.id.with("grab-offset");
            let offset = if response.drag_started() || response.clicked() {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
                let offset = origin.x - position;
                ui.data_mut(|d| d.insert_temp(offset_id, offset));
                offset
            } else {
                ui.data(|d| d.get_temp::<f32>(offset_id)).unwrap_or(0.0)
            };
            let fraction =
                ((pointer.x - offset - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
            preview = Some(set_edge(
                clip,
                edge,
                (fraction as f64 * limit).round() as i64,
            ));
        }
    }
    ui.horizontal(|ui| {
        ui.small(clock(clip.start_frame as f64 / clip.fps));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.small(clock(clip.end_frame as f64 / clip.fps));
        });
    });
    preview
}

pub(crate) struct SourcePreview {
    player: Player,
    texture: Option<egui::TextureHandle>,
    pending_frame: Option<i64>,
    last_seek: Instant,
}

impl SourcePreview {
    pub fn new(
        clip: &ClipView,
        source: NativeSegment,
        tools: Option<PathBuf>,
        muted: bool,
        ctx: &egui::Context,
    ) -> Self {
        let player = Player::new_source_preview(
            vec![source],
            tools,
            clip.start_frame as f64 / clip.fps,
            ctx.clone(),
            if muted { 0.0 } else { 1.0 },
            clip.fps,
        );
        player.pause(true);
        Self {
            player,
            texture: None,
            pending_frame: None,
            last_seek: Instant::now() - Duration::from_secs(1),
        }
    }

    pub fn seek_frame(&mut self, frame: i64) {
        self.pending_frame = Some(frame);
        self.player.pause(true);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, clip: &ClipView) -> Vec<String> {
        // Coalesce rapid handle movement: at most one decoder restart per 125 ms,
        // always retaining the final requested source frame after release.
        if self.pending_frame.is_some() && self.last_seek.elapsed() >= Duration::from_millis(125) {
            let frame = self.pending_frame.take().unwrap().clamp(0, clip.last_frame);
            self.player.seek(frame as f64 / clip.fps);
            self.last_seek = Instant::now();
        }
        if !self.player.paused() && self.player.position() >= clip.end_frame as f64 / clip.fps {
            self.player.pause(true);
            self.seek_frame(clip.end_frame - 1);
        }
        if let Some(frame) = self.player.frame() {
            let image = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb);
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "boundary-source",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        let clicked = preview_canvas(ui, self.texture.as_ref()).clicked();
        let button = ui
            .button(if self.player.paused() {
                "Play preview"
            } else {
                "Pause preview"
            })
            .clicked();
        if clicked || button {
            if self.player.paused()
                && (self.player.position() >= clip.end_frame as f64 / clip.fps
                    || self.player.position() < clip.start_frame as f64 / clip.fps)
            {
                self.player.seek(clip.start_frame as f64 / clip.fps);
            }
            self.player.pause(!self.player.paused());
        }
        ui.small(format!(
            "Source frame {} · {}",
            (self.player.position() * clip.fps).round() as i64,
            clock(self.player.position())
        ));
        if self.pending_frame.is_some() || !self.player.paused() || self.player.buffering() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        self.player.warnings()
    }
}

pub(crate) fn preview_canvas(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
) -> egui::Response {
    let height = (ui.available_width() * 9.0 / 16.0)
        .min(400.0)
        .min(ui.ctx().content_rect().height() * 0.42);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
    if let Some(texture) = texture {
        let scale =
            (rect.width() / texture.size_vec2().x).min(rect.height() / texture.size_vec2().y);
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(rect.center(), texture.size_vec2() * scale),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    response
}

#[cfg(test)]
mod tests {

    fn long_clip() -> ClipView {
        ClipView {
            id: "c".into(),
            file_id: "f".into(),
            file_name: "PICT0032.AVI".into(),
            start_frame: 100,
            end_frame: 200,
            fps: 24.486,
            last_frame: 1000,
            file_duration: 40.8,
            entry: "unknown".into(),
            exit: "nosignal".into(),
            thumbnail: None,
        }
    }

    #[test]
    fn nudges_an_edge_by_seconds_or_frames_at_the_recordings_own_rate() {
        let steps = nudge_steps(24.486);
        assert_eq!(steps.map(|(_, frames)| frames), [-24, -6, -1, 1, 6, 24]);
        let mut cut = long_clip();
        assert_eq!(nudge(&mut cut, Edge::Start, -1), 99);
        assert_eq!(
            nudge(&mut cut, Edge::End, 24),
            223,
            "the end previews its last frame"
        );
        assert_eq!(cut.end_frame, 224);
        // Never past the recording, never across the other edge.
        assert_eq!(nudge(&mut cut, Edge::Start, -10_000), 0);
        cut.start_frame = 220;
        assert_eq!(nudge(&mut cut, Edge::Start, 24), 223);
        assert_eq!(nudge(&mut cut, Edge::End, 10_000), 1000);
        assert_eq!(
            cut.end_frame, 1001,
            "end is exclusive: one past the last frame"
        );
    }

    use super::*;
    fn clip() -> ClipView {
        ClipView {
            id: "clip".into(),
            file_id: "file".into(),
            file_name: "DVR.avi".into(),
            start_frame: 20,
            end_frame: 80,
            last_frame: 99,
            fps: 25.0,
            file_duration: 4.0,
            entry: "".into(),
            exit: "".into(),
            thumbnail: None,
        }
    }
    #[test]
    fn edges_stay_inside_source_and_cannot_cross() {
        let mut clip = clip();
        assert_eq!(set_edge(&mut clip, Edge::Start, -100), 0);
        assert_eq!(clip.end_frame, 80);
        assert_eq!(set_edge(&mut clip, Edge::Start, 1000), 79);
        assert_eq!(set_edge(&mut clip, Edge::End, -100), 79);
        assert_eq!((clip.start_frame, clip.end_frame), (79, 80));
        assert_eq!(set_edge(&mut clip, Edge::End, 1000), 99);
        assert_eq!(
            clip.end_frame, 100,
            "exclusive end includes the final source frame"
        );
    }
    #[test]
    fn dragging_each_handle_changes_only_its_own_boundary() {
        for (edge, target) in [(Edge::Start, 35), (Edge::End, 65)] {
            let ctx = egui::Context::default();
            let mut clip = clip();
            let render = |clip: &mut ClipView, events| {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 180.0),
                    )),
                    events,
                    ..Default::default()
                };
                ctx.run(raw, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        trim_timeline(ui, clip);
                    });
                })
            };
            let mut output = None;
            for _ in 0..3 {
                output = Some(render(&mut clip, vec![]));
            }
            let rail = output
                .unwrap()
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.fill == Color32::from_gray(50) => Some(r.rect),
                    _ => None,
                })
                .unwrap();
            let offset = if edge == Edge::Start { -6.0 } else { 6.0 };
            let frame = if edge == Edge::Start { 20.0 } else { 80.0 };
            let from = egui::pos2(
                rail.left() + rail.width() * frame / 100.0 + offset,
                rail.center().y,
            );
            let to = egui::pos2(
                rail.left() + rail.width() * target as f32 / 100.0 + offset,
                rail.center().y,
            );
            render(
                &mut clip,
                vec![
                    egui::Event::PointerMoved(from),
                    egui::Event::PointerButton {
                        pos: from,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            render(&mut clip, vec![egui::Event::PointerMoved(to)]);
            render(
                &mut clip,
                vec![egui::Event::PointerButton {
                    pos: to,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            assert_eq!(
                (clip.start_frame, clip.end_frame),
                if edge == Edge::Start {
                    (35, 80)
                } else {
                    (20, 65)
                }
            );
        }
    }
}
