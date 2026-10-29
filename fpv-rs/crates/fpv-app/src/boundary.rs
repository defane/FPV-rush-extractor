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

pub(crate) fn trim_timeline(
    ui: &mut egui::Ui,
    clip: &mut ClipView,
    textures: &[Option<egui::TextureHandle>],
    playhead: Option<i64>,
) -> Option<i64> {
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
    let (rect, strip_response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 86.0),
        egui::Sense::click_and_drag(),
    );
    let rail = rect.shrink2(egui::vec2(14.0, 8.0));
    let limit = clip.last_frame.saturating_add(1).max(1) as f64;
    let x = |frame: i64| rail.left() + (frame as f64 / limit) as f32 * rail.width();
    let left = x(clip.start_frame);
    let right = x(clip.end_frame);
    ui.painter().rect_filled(rail, 3.0, Color32::from_gray(50));
    let cells = ((rail.width() / 80.0).ceil() as usize).clamp(1, textures.len().max(1));
    for cell in 0..cells {
        let index = if cells == 1 {
            0
        } else {
            cell * textures.len().saturating_sub(1) / (cells - 1)
        };
        if let Some(Some(texture)) = textures.get(index) {
            let tile = egui::Rect::from_x_y_ranges(
                rail.left() + rail.width() * cell as f32 / cells as f32
                    ..=rail.left() + rail.width() * (cell + 1) as f32 / cells as f32,
                rail.y_range(),
            );
            let ratio =
                (tile.width() / tile.height()) / (texture.size_vec2().x / texture.size_vec2().y);
            let uv = if ratio < 1.0 {
                egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(ratio, 1.0))
            } else {
                egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(1.0, 1.0 / ratio))
            };
            ui.painter().image(texture.id(), tile, uv, Color32::WHITE);
        }
    }
    for range in [rail.left()..=left, right..=rail.right()] {
        ui.painter().rect_filled(
            egui::Rect::from_x_y_ranges(range, rail.y_range()),
            0.0,
            Color32::from_black_alpha(170),
        );
    }
    ui.painter().rect_stroke(
        egui::Rect::from_x_y_ranges(left..=right, rail.y_range()),
        0.0,
        egui::Stroke::new(2.0_f32, ACCENT),
        egui::StrokeKind::Inside,
    );
    if (strip_response.hovered() || strip_response.dragged())
        && let Some(pointer) = strip_response
            .hover_pos()
            .or_else(|| strip_response.interact_pointer_pos())
    {
        let fraction = ((pointer.x - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
        preview = Some(((fraction as f64 * limit).floor() as i64).clamp(0, clip.last_frame));
    }
    if let Some(frame) = preview.or(playhead) {
        let position = x(frame);
        ui.painter().vline(
            position,
            rail.y_range(),
            egui::Stroke::new(2.0_f32, Color32::WHITE),
        );
    }
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
        ui.small("0:00 · source start");
        if let Some(frame) = preview.or(playhead) {
            ui.small(format!(
                "Preview {:.3} s · frame {frame}",
                frame as f64 / clip.fps
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.small(format!("{} · source end", clock(limit / clip.fps)));
        });
    });
    preview
}

pub(crate) struct SourcePreview {
    player: Player,
    texture: Option<egui::TextureHandle>,
    pending_frame: Option<i64>,
    requested_frame: Option<i64>,
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
            requested_frame: Some(clip.start_frame),
            last_seek: Instant::now() - Duration::from_secs(1),
        }
    }

    pub fn seek_frame(&mut self, frame: i64) {
        if self.requested_frame != Some(frame) {
            self.requested_frame = Some(frame);
            self.pending_frame = Some(frame);
            self.player.pause(true);
        }
    }

    pub fn position_frame(&self, fps: f64) -> i64 {
        self.pending_frame
            .unwrap_or_else(|| (self.player.position() * fps).round() as i64)
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
            self.pending_frame = None;
            self.requested_frame = None;
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
        .min(ui.ctx().content_rect().height() * 0.30);
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
    fn hovering_full_source_strip_previews_inside_and_outside_selection_without_editing() {
        let ctx = egui::Context::default();
        let mut clip = clip();
        let textures: Vec<_> = [Color32::RED, Color32::BLUE]
            .into_iter()
            .enumerate()
            .map(|(i, color)| {
                Some(ctx.load_texture(
                    format!("strip-{i}"),
                    egui::ColorImage::filled([160, 120], color),
                    Default::default(),
                ))
            })
            .collect();
        let mut render = |pointer: Option<egui::Pos2>| {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 220.0),
                )),
                ..Default::default()
            };
            if let Some(pointer) = pointer {
                raw.events.push(egui::Event::PointerMoved(pointer));
            }
            let mut requested = None;
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    requested = trim_timeline(ui, &mut clip, &textures, Some(25));
                });
            });
            (output, requested)
        };
        render(None);
        let (output, _) = render(None);
        let rail = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == Color32::from_gray(50) => Some(r.rect),
                _ => None,
            })
            .unwrap();
        for (fraction, frame) in [(0.0, 0), (0.1, 10), (0.5, 50), (0.9, 90), (1.0, 99)] {
            let point = egui::pos2(rail.left() + rail.width() * fraction, rail.center().y);
            render(Some(point));
            let (output, requested) = render(Some(point));
            assert_eq!(requested, Some(frame), "full-source frame at {fraction}");
            for texture in textures.iter().flatten() {
                assert!(output.shapes.iter().any(
                    |s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == texture.id())
                ));
            }
        }
        assert_eq!(
            (clip.start_frame, clip.end_frame),
            (20, 80),
            "hover never edits selection"
        );
    }

    #[test]
    fn repeated_hover_does_not_restart_the_same_preview_and_latest_frame_is_retained() {
        let ctx = egui::Context::default();
        let clip = clip();
        let source = NativeSegment {
            segment: fpv_library::Segment {
                path: "unavailable".into(),
                start: 0.0,
                end: 4.0,
                offset: 0.0,
            },
            width: 64,
            height: 48,
            has_audio: false,
            all_intra: true,
        };
        let mut preview = SourcePreview::new(&clip, source, None, true, &ctx);
        preview.seek_frame(10);
        assert_eq!(preview.pending_frame.take(), Some(10));
        preview.seek_frame(10);
        assert_eq!(
            preview.pending_frame, None,
            "stationary pointer does not seek again"
        );
        preview.seek_frame(90);
        preview.seek_frame(99);
        assert_eq!(
            preview.pending_frame,
            Some(99),
            "coalescing preserves final hover position"
        );
        assert_eq!(preview.position_frame(clip.fps), 99);
        assert_eq!((clip.start_frame, clip.end_frame), (20, 80));
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
                        trim_timeline(ui, clip, &[], None);
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
