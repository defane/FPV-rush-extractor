use super::*;
use crate::{browse::Browse, images::Images, player::Player, worker::Worker};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

fn app(ctx: &egui::Context) -> App {
    crate::configure_style(ctx);
    let (sender, _) = mpsc::channel();
    let (_, events) = mpsc::channel();
    App {
        worker: Worker { sender, events },
        sessions: vec![],
        cards: vec![],
        config: Default::default(),
        config_directory: PathBuf::new(),
        cache_directory: PathBuf::new(),
        tools: None,
        ffmpeg_ok: true,
        ready: true,
        browse: Browse::default(),
        tasks: vec![],
        next_id: 1,
        dialog: None,
        dialog_request: None,
        viewer: None,
        cloud_wait: None,
        detached: HashSet::new(),
        strips: crate::strips::Strips::default(),
        strip_batch: None,
        images: Images::new(ctx.clone()),
        thumbnails: HashMap::new(),
        requested: HashSet::new(),
        selected: HashSet::new(),
        selecting: false,
        search: String::new(),
        notices: Default::default(),
        show_notices: false,
        show_tasks: false,
        exit_after: None,
        screenshot_requested: false,
        splash_started: None,
        splash_texture: ctx.load_texture(
            "test-logo",
            egui::ColorImage::filled([180, 194], Color32::BLACK),
            Default::default(),
        ),
        show_about: false,
        #[cfg(target_os = "macos")]
        about_menu: None,
    }
}
fn flight() -> FlightView {
    FlightView {
        id: "test-flight".into(),
        order: 1,
        title: Some("Layout test flight".into()),
        liked: false,
        duration: 60.0,
        clips: vec![],
        recordings: 1,
        action_cam: true,
        source: "GoPro".into(),
        format: "1080p · H.264 · 30 fps".into(),
        reduced: false,
        cloud: false,
        freeable: false,
        cache_key: "test-flight".into(),
        labels: vec![],
        figures: vec![],
    }
}
fn text_positions(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.job.text.clone(), t.pos)),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
                }
            }
            _ => {}
        }
    }
    let mut out = vec![];
    for s in shapes {
        walk(&s.shape, &mut out);
    }
    out
}
fn input(size: egui::Vec2) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    }
}
#[test]
fn player_fits_viewport_when_opened_and_when_the_display_shrinks() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let new_viewer = || Viewer {
        flight: flight(),
        player: Player::new(vec![], None, 0.0, ctx.clone()),
        texture: Some(ctx.load_texture(
            "test-video",
            egui::ColorImage::filled([640, 360], Color32::GRAY),
            Default::default(),
        )),
        mark: None,
        figure: String::new(),
        label: String::new(),
        volume: 1.0,
        remembered_volume: 1.0,
        playback_muted: false,
        muted: false,
        fullscreen: false,
        window: Default::default(),
        seek_time: None,
    };
    app.viewer = Some(new_viewer());
    for _ in 0..4 {
        let _ = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
            app.player_window(ctx)
        });
    }
    let initial = ctx
        .memory(|m| m.area_rect(egui::Id::new("player")))
        .unwrap();
    assert!(
        initial.width() <= 1050.0 && initial.height() <= 680.0,
        "player should open compact, not fill the viewport: {initial:?}"
    );
    assert!(!app.viewer.as_ref().unwrap().fullscreen);
    fn click_control(ctx: &egui::Context, app: &mut App, label: &str) {
        let size = egui::vec2(1280.0, 820.0);
        let output = ctx.run(input(size), |ctx| app.player_window(ctx));
        let position = text_positions(&output.shapes)
            .into_iter()
            .find(|(text, _)| text == label)
            .expect("visible player control")
            .1
            + egui::vec2(5.0, 5.0);
        for pressed in [true, false] {
            let mut raw = input(size);
            raw.events = vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run(raw, |ctx| app.player_window(ctx));
        }
        for _ in 0..4 {
            let _ = ctx.run(input(size), |ctx| app.player_window(ctx));
        }
    }
    click_control(&ctx, &mut app, "Maximize");
    let maximized = ctx
        .memory(|m| m.area_rect(egui::Id::new("player")))
        .unwrap();
    assert!(maximized.width() > initial.width() + 100.0);
    assert!(maximized.height() > initial.height() + 100.0);
    assert!(
        !app.viewer.as_ref().unwrap().fullscreen,
        "maximize must keep OS windowed"
    );
    click_control(&ctx, &mut app, "Restore");
    let restored = ctx
        .memory(|m| m.area_rect(egui::Id::new("player")))
        .unwrap();
    assert!(
        (restored.size() - initial.size()).length() < 2.0
            && restored.min.distance(initial.min) < 2.0,
        "restore should return to {initial:?}, got {restored:?}"
    );
    click_control(&ctx, &mut app, "Maximize");
    click_control(&ctx, &mut app, "×");
    assert!(
        app.viewer.is_none(),
        "close must dismiss the maximized player"
    );
    app.viewer = Some(new_viewer());
    for _ in 0..4 {
        let _ = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
            app.player_window(ctx)
        });
    }
    let reopened_rect = ctx
        .memory(|m| m.area_rect(egui::Id::new("player")))
        .unwrap();
    assert!(
        (reopened_rect.size() - initial.size()).length() < 2.0,
        "opening another video must reset maximized geometry: {reopened_rect:?}"
    );
    for size in [
        egui::vec2(1280.0, 820.0),
        egui::vec2(900.0, 600.0),
        egui::vec2(640.0, 480.0),
    ] {
        for _ in 0..4 {
            let _ = ctx.run(input(size), |ctx| app.player_window(ctx));
        }
        let rect = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("player")))
            .unwrap();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        assert!(
            screen.expand(1.0).contains_rect(rect),
            "player {rect:?} exceeds display {screen:?}"
        );
    }
    let size = egui::vec2(900.0, 600.0);
    for fullscreen in [true, false] {
        app.viewer.as_mut().unwrap().fullscreen = fullscreen;
        for _ in 0..4 {
            let _ = ctx.run(input(size), |ctx| app.player_window(ctx));
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("player")))
            .unwrap();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                .expand(1.0)
                .contains_rect(rect),
            "fullscreen transition exceeds viewport: {rect:?}"
        );
    }
    app.viewer.as_mut().unwrap().fullscreen = true;
    let mut escape = input(size);
    escape.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    let output = ctx.run(escape, |ctx| app.player_window(ctx));
    assert!(!app.viewer.as_ref().unwrap().fullscreen);
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Fullscreen(false)))
    );
}
#[test]
fn pagination_is_rendered_below_the_flight_cards() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.browse.view = View::Flights;
    app.sessions.push(SessionView {
        id: "session".into(),
        name: "Test session".into(),
        date: None,
        files: vec![],
        flights: vec![flight()],
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    });
    let mut positions = vec![];
    for _ in 0..3 {
        let output = ctx.run(input(egui::vec2(900.0, 600.0)), |ctx| app.browser(ctx));
        positions = text_positions(&output.shapes);
    }
    let showing = positions
        .iter()
        .find(|(s, _)| s.starts_with("Showing "))
        .unwrap()
        .1;
    let play = positions
        .iter()
        .find(|(s, _)| s == "Layout test flight")
        .unwrap()
        .1;
    assert!(
        showing.y > play.y,
        "pagination at {showing:?} precedes the cards at {play:?}"
    );
}
#[test]
fn about_shows_the_requested_roles_and_letzkwad_link() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.show_about = true;
    let mut positions = vec![];
    for _ in 0..3 {
        let output = ctx.run(input(egui::vec2(900.0, 700.0)), |ctx| app.about_window(ctx));
        positions = text_positions(&output.shapes);
    }
    let mut previous = 0.0;
    for expected in [
        "Coded by",
        "• Claude",
        "• Codex",
        "Driven by",
        "• Defane",
        "• Thanat0s",
    ] {
        let y = positions
            .iter()
            .find(|(text, _)| text == expected)
            .unwrap_or_else(|| panic!("missing contributor line {expected}"))
            .1
            .y;
        assert!(
            y > previous,
            "credits must appear in role order on separate lines"
        );
        previous = y;
    }
    assert!(positions.iter().any(|(s, _)| s.contains("letzkwad.lu")));
}
#[test]
fn splash_has_no_close_button_and_grows_while_fading() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.splash_started = Some(std::time::Instant::now());
    for _ in 0..3 {
        let output = ctx.run(input(egui::vec2(900.0, 700.0)), |ctx| {
            app.splash_window(ctx)
        });
        assert!(
            !text_positions(&output.shapes)
                .iter()
                .any(|(s, _)| s == "Close")
        );
    }
}

#[test]
fn video_progress_times_are_drawn_inside_the_video_image() {
    fn image_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(egui::Rect::from_points(
                &mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| image_bounds(s, texture)),
            _ => None,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let texture = ctx.load_texture(
        "overlay-test-video",
        egui::ColorImage::filled([640, 360], Color32::GRAY),
        Default::default(),
    );
    let texture_id = texture.id();
    app.viewer = Some(Viewer {
        flight: flight(),
        player: Player::new(vec![], None, 0.0, ctx.clone()),
        texture: Some(texture),
        mark: None,
        figure: String::new(),
        label: String::new(),
        volume: 1.0,
        remembered_volume: 1.0,
        playback_muted: false,
        muted: false,
        fullscreen: false,
        window: Default::default(),
        seek_time: None,
    });
    let mut shapes = vec![];
    for _ in 0..4 {
        shapes = ctx
            .run(input(egui::vec2(1100.0, 760.0)), |ctx| {
                app.player_window(ctx)
            })
            .shapes;
    }
    let image = shapes
        .iter()
        .find_map(|s| image_bounds(&s.shape, texture_id))
        .unwrap();
    let times: Vec<_> = text_positions(&shapes)
        .into_iter()
        .filter(|(text, _)| text == &clock(0.0))
        .collect();
    assert_eq!(times.len(), 2);
    for (_, position) in times {
        assert!(
            image.contains(position),
            "time at {position:?} lies outside the video {image:?}"
        );
    }
}

#[test]
fn import_speaker_button_toggles_the_same_state_as_the_no_sound_checkbox() {
    let ctx = egui::Context::default();
    let mut muted = false;
    let mut icon = egui::Rect::NOTHING;
    for _ in 0..2 {
        let _ = ctx.run(input(egui::vec2(400.0, 150.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                icon = no_sound_choice(ui, &mut muted).rect;
            });
        });
    }
    for expected in [true, false] {
        for pressed in [true, false] {
            let mut input = input(egui::vec2(400.0, 150.0));
            input.events = vec![
                egui::Event::PointerMoved(icon.center()),
                egui::Event::PointerButton {
                    pos: icon.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = no_sound_choice(ui, &mut muted);
                });
            });
        }
        assert_eq!(muted, expected);
    }
}

#[test]
fn splash_visibly_reaches_twice_its_initial_size_before_becoming_transparent() {
    fn mesh(shapes: &[egui::epaint::ClippedShape], texture: egui::TextureId) -> (egui::Rect, u8) {
        fn find(shape: &egui::Shape, texture: egui::TextureId) -> Option<(egui::Rect, u8)> {
            match shape {
                egui::Shape::Rect(r)
                    if r.brush
                        .as_ref()
                        .is_some_and(|b| b.fill_texture_id == texture) =>
                {
                    Some((r.rect, r.fill.a()))
                }
                egui::Shape::Mesh(m) if m.texture_id == texture => Some((
                    egui::Rect::from_points(&m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()),
                    m.vertices[0].color.a(),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, texture)),
                _ => None,
            }
        }
        shapes.iter().find_map(|s| find(&s.shape, texture)).unwrap()
    }
    let ctx = egui::Context::default();
    let app = app(&ctx);
    let mut shapes = vec![];
    for _ in 0..12 {
        shapes = ctx
            .run(input(egui::vec2(900.0, 700.0)), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| splash_logo(ui, &app.splash_texture, 0.0));
            })
            .shapes;
    }
    let initial = mesh(&shapes, app.splash_texture.id());
    shapes = ctx
        .run(input(egui::vec2(900.0, 700.0)), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| splash_logo(ui, &app.splash_texture, 0.7));
        })
        .shapes;
    let leaving = mesh(&shapes, app.splash_texture.id());
    assert!(leaving.0.width() >= initial.0.width() * 2.0);
    assert!(leaving.0.height() >= initial.0.height() * 2.0);
    assert!(leaving.1 > 0 && leaving.1 < initial.1);
}

#[test]
fn screenshot_reference_navigation_shows_tab_counts() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut flight = flight();
    flight.figures.push(fpv_library::TagView {
        id: "tag".into(),
        label: "split-S".into(),
        note: None,
        start: 1.0,
        end: 2.0,
        marker: false,
    });
    app.sessions.push(SessionView {
        id: "s".into(),
        name: "Reference session".into(),
        date: None,
        files: vec![],
        flights: vec![flight],
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    });
    let mut positions = vec![];
    for _ in 0..3 {
        positions = text_positions(
            &ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| app.sidebar(ctx))
                .shapes,
        );
    }
    for expected in ["Sessions\n1", "All flights\n1", "Figures\n1"] {
        assert!(
            positions.iter().any(|(text, _)| text == expected),
            "missing tab {expected:?}"
        );
    }
}

#[test]
fn screenshot_reference_figure_cards_keep_the_flight_title_and_overlay_the_figure_name() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.browse.view = View::Figures;
    let mut flight = flight();
    flight.figures.push(fpv_library::TagView {
        id: "tag".into(),
        label: "split-S".into(),
        note: None,
        start: 1.0,
        end: 2.0,
        marker: false,
    });
    app.sessions.push(SessionView {
        id: "s".into(),
        name: "Reference session".into(),
        date: None,
        files: vec![],
        flights: vec![flight],
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    });
    let mut positions = vec![];
    for _ in 0..3 {
        positions = text_positions(
            &ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| app.browser(ctx))
                .shapes,
        );
    }
    let figure = positions
        .iter()
        .find(|(text, _)| text == "split-S")
        .expect("figure badge")
        .1;
    let flight = positions
        .iter()
        .find(|(text, _)| text == "Layout test flight")
        .expect("parent flight title")
        .1;
    assert!(
        figure.y < flight.y,
        "figure badge at {figure:?} must precede the flight caption at {flight:?}"
    );
}

#[test]
fn pagination_stays_inside_a_narrow_view_with_many_pages() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    app.browse.view = View::Flights;
    app.browse.page = 10;
    app.sessions.push(SessionView {
        id: "s".into(),
        name: "Large session".into(),
        date: None,
        files: vec![],
        flights: vec![flight(); 500],
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    });
    let mut positions = vec![];
    for _ in 0..4 {
        positions = text_positions(
            &ctx.run(input(egui::vec2(900.0, 600.0)), |ctx| {
                app.sidebar(ctx);
                app.browser(ctx);
            })
            .shapes,
        );
    }
    for label in ["Showing ", "Per page"] {
        let position = positions
            .iter()
            .find(|(text, _)| text.starts_with(label))
            .unwrap()
            .1;
        assert!(
            position.x >= 0.0
                && position.x < 900.0
                && position.y > 500.0
                && position.y + 24.0 <= 600.0,
            "footer {label} lies outside its bottom row: {position:?}"
        );
    }
}

fn texture_rect(
    shapes: &[egui::epaint::ClippedShape],
    texture: egui::TextureId,
) -> Option<egui::Rect> {
    fn find(shape: &egui::Shape, texture: egui::TextureId) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Rect(r)
                if r.brush
                    .as_ref()
                    .is_some_and(|b| b.fill_texture_id == texture) =>
            {
                Some(r.rect)
            }
            egui::Shape::Mesh(m) if m.texture_id == texture => Some(egui::Rect::from_points(
                &m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, texture)),
            _ => None,
        }
    }
    shapes.iter().find_map(|s| find(&s.shape, texture))
}

#[test]
fn splash_moves_from_the_first_frames_without_a_hold() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut widths = vec![];
    for elapsed in [0.05, 0.3] {
        let mut output = None;
        for _ in 0..4 {
            app.splash_started = Some(std::time::Instant::now() - Duration::from_secs_f32(elapsed));
            output = Some(ctx.run(input(egui::vec2(1280.0, 900.0)), |ctx| {
                app.splash_window(ctx)
            }));
        }
        widths.push(
            texture_rect(&output.unwrap().shapes, app.splash_texture.id())
                .unwrap()
                .width(),
        );
    }
    assert!(
        widths[0] < 100.0,
        "splash should start in the distance: {widths:?}"
    );
    assert!(
        widths[1] > widths[0] * 1.2,
        "splash must move immediately: {widths:?}"
    );
}

#[test]
fn thumbnail_hearts_have_no_background_box() {
    fn boxes(shape: &egui::Shape, point: egui::Pos2) -> bool {
        match shape {
            egui::Shape::Rect(r) => {
                r.rect.contains(point)
                    && r.rect.width() < 45.0
                    && r.rect.height() < 45.0
                    && (r.fill.a() > 0 || r.stroke.width > 0.0)
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| boxes(s, point)),
            _ => false,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let session = SessionView {
        id: "s".into(),
        name: "Test".into(),
        date: None,
        files: vec![],
        flights: vec![],
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    };
    let mut flight = flight();
    for liked in [false, true] {
        flight.liked = liked;
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(input(egui::vec2(600.0, 500.0)), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.flight_card(ui, &session, &flight, None));
            }));
        }
        let output = output.unwrap();
        let heart = text_positions(&output.shapes)
            .into_iter()
            .find(|(s, _)| s == if liked { "♥" } else { "♡" })
            .unwrap()
            .1
            + egui::vec2(4.0, 4.0);
        assert!(
            !output.shapes.iter().any(|s| boxes(&s.shape, heart)),
            "heart must not have a painted button box"
        );
    }
}

fn test_viewer(ctx: &egui::Context) -> Viewer {
    Viewer {
        flight: flight(),
        player: Player::new(vec![], None, 0.0, ctx.clone()),
        texture: None,
        mark: None,
        figure: String::new(),
        label: String::new(),
        volume: 0.6,
        remembered_volume: 0.6,
        playback_muted: false,
        muted: false,
        fullscreen: false,
        window: Default::default(),
        seek_time: None,
    }
}

#[test]
fn player_speaker_click_mutes_and_restores_the_audible_gain() {
    let ctx = egui::Context::default();
    let mut viewer = test_viewer(&ctx);
    viewer.player.set_volume(viewer.volume);
    let mut rect = egui::Rect::NOTHING;
    let render = |ctx: &egui::Context, viewer: &mut Viewer, rect: &mut egui::Rect, events| {
        let mut raw = input(egui::vec2(300.0, 150.0));
        raw.events = events;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                *rect = player_speaker(ui, viewer).rect;
            });
        });
    };
    for _ in 0..3 {
        render(&ctx, &mut viewer, &mut rect, vec![]);
    }
    for expected in [0.0, 0.6] {
        for pressed in [true, false] {
            let pos = rect.center();
            render(
                &ctx,
                &mut viewer,
                &mut rect,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(
            (viewer.player.volume() - expected).abs() < 0.001,
            "speaker click did not update gain"
        );
    }
    viewer.volume = 0.0;
    viewer.player.set_volume(0.0);
    for pressed in [true, false] {
        let pos = rect.center();
        render(
            &ctx,
            &mut viewer,
            &mut rect,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert!(
        (viewer.player.volume() - 0.6).abs() < 0.001,
        "zero-volume unmute restores the last audible level"
    );
    viewer.player.seek(0.0);
    assert!(
        (viewer.player.volume() - 0.6).abs() < 0.001,
        "seeking preserves gain"
    );
    viewer.muted = true;
    viewer.player.set_volume(0.0);
    for pressed in [true, false] {
        let pos = rect.center();
        render(
            &ctx,
            &mut viewer,
            &mut rect,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(
        viewer.player.volume(),
        0.0,
        "speaker must not undo forced no-audio policy"
    );
}

#[test]
fn volume_is_vertical_on_the_right_and_increases_upwards() {
    let ctx = egui::Context::default();
    let mut viewer = test_viewer(&ctx);
    let mut slider = egui::Rect::NOTHING;
    let video = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(500.0, 320.0));
    let render = |viewer: &mut Viewer, slider: &mut egui::Rect, events| {
        let mut raw = input(egui::vec2(600.0, 400.0));
        raw.events = events;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(response) = player_volume_overlay(ui, viewer, video) {
                    *slider = response.rect;
                }
            });
        });
    };
    for _ in 0..3 {
        render(&mut viewer, &mut slider, vec![]);
    }
    assert!(
        slider.height() > slider.width() * 3.0,
        "volume should be vertical: {slider:?}"
    );
    assert!(video.contains_rect(slider) && slider.right() > video.right() - 45.0);
    let mut gains = vec![];
    for y in [slider.bottom() - 8.0, slider.top() + 8.0] {
        for pressed in [true, false] {
            let pos = egui::pos2(slider.center().x, y);
            render(
                &mut viewer,
                &mut slider,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        gains.push(viewer.player.volume());
    }
    assert!(
        gains[1] > gains[0] + 0.5,
        "volume increases upwards: {gains:?}"
    );
    viewer.muted = true;
    let _ = ctx.run(input(egui::vec2(600.0, 400.0)), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            assert!(player_volume_overlay(ui, &mut viewer, video).is_none())
        });
    });
}

#[test]
fn video_controls_do_not_paint_a_background_strip() {
    fn has_strip(shape: &egui::Shape, image: egui::Rect) -> bool {
        match shape {
            egui::Shape::Rect(r) => {
                image.expand(1.0).contains_rect(r.rect)
                    && r.rect.width() > image.width() * 0.7
                    && r.rect.height() > 40.0
                    && r.rect.height() < image.height() * 0.8
                    && r.fill.a() > 0
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| has_strip(s, image)),
            _ => false,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut viewer = test_viewer(&ctx);
    let texture = ctx.load_texture(
        "transparent-controls",
        egui::ColorImage::filled([640, 360], Color32::WHITE),
        Default::default(),
    );
    let id = texture.id();
    viewer.texture = Some(texture);
    app.viewer = Some(viewer);
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
            app.player_window(ctx)
        }));
    }
    let shapes = output.unwrap().shapes;
    let image = texture_rect(&shapes, id).unwrap();
    assert!(
        !shapes.iter().any(|s| has_strip(&s.shape, image)),
        "controls must be transparent over the video"
    );
}

fn player_input(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut raw = input(egui::vec2(1280.0, 820.0));
    raw.events = events;
    ctx.run(raw, |ctx| app.player_window(ctx))
}
fn click_player(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) {
    for pressed in [true, false] {
        player_input(
            ctx,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}
#[test]
fn video_click_buttons_and_space_share_playback_state() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut viewer = test_viewer(&ctx);
    let texture = ctx.load_texture(
        "clickable-video",
        egui::ColorImage::filled([640, 360], Color32::GRAY),
        Default::default(),
    );
    let id = texture.id();
    viewer.texture = Some(texture);
    app.viewer = Some(viewer);
    let mut output = None;
    for _ in 0..4 {
        output = Some(player_input(&ctx, &mut app, vec![]));
    }
    let image = texture_rect(&output.unwrap().shapes, id).unwrap();
    click_player(&ctx, &mut app, image.center());
    assert!(
        app.viewer.as_ref().unwrap().player.paused(),
        "video click must pause playback"
    );
    let output = player_input(&ctx, &mut app, vec![]);
    let play = text_positions(&output.shapes)
        .into_iter()
        .find(|(s, _)| s == "Play")
        .expect("retained Play button")
        .1;
    assert!(
        play.y > image.bottom(),
        "playback buttons belong below the video"
    );
    click_player(&ctx, &mut app, play + egui::vec2(6.0, 6.0));
    assert!(
        !app.viewer.as_ref().unwrap().player.paused(),
        "Play button must resume once"
    );
    click_player(
        &ctx,
        &mut app,
        egui::pos2(image.right() - 31.0, image.bottom() - 180.0),
    );
    assert!(
        !app.viewer.as_ref().unwrap().player.paused(),
        "volume clicks must not toggle video playback"
    );
    // Give focus back to the video before testing the global shortcut.
    click_player(&ctx, &mut app, image.center());
    assert!(app.viewer.as_ref().unwrap().player.paused());
    ctx.memory_mut(|m| m.surrender_focus(m.focused().unwrap_or(egui::Id::NULL)));
    player_input(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(
        !app.viewer.as_ref().unwrap().player.paused(),
        "Space must resume"
    );
    let output = player_input(&ctx, &mut app, vec![]);
    let field = text_positions(&output.shapes)
        .into_iter()
        .find(|(s, _)| s == "Add a tag…")
        .unwrap()
        .1;
    click_player(&ctx, &mut app, field + egui::vec2(8.0, 6.0));
    player_input(
        &ctx,
        &mut app,
        vec![
            egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
            egui::Event::Text(" ".into()),
        ],
    );
    assert!(
        !app.viewer.as_ref().unwrap().player.paused(),
        "typing spaces must not pause playback"
    );
}

#[test]
fn boundary_editor_places_start_left_and_end_right() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut flight = flight();
    flight.clips.push(fpv_library::ClipView {
        id: "clip".into(),
        file_id: "source".into(),
        file_name: "DVR.avi".into(),
        start_frame: 10,
        end_frame: 80,
        last_frame: 99,
        fps: 25.0,
        file_duration: 4.0,
        entry: "".into(),
        exit: "".into(),
        thumbnail: None,
    });
    app.dialog = Some(Dialog::editor(&flight));
    let mut output = None;
    for _ in 0..4 {
        output = Some(ctx.run(input(egui::vec2(1280.0, 900.0)), |ctx| {
            app.dialog_window(ctx)
        }));
    }
    let texts = text_positions(&output.unwrap().shapes);
    let start = texts
        .iter()
        .find(|(s, _)| s == "Start")
        .unwrap_or_else(|| panic!("start handle label missing: {texts:?}"))
        .1;
    let end = texts
        .iter()
        .find(|(s, _)| s == "End (exclusive)")
        .expect("end handle label")
        .1;
    assert!(
        end.x > start.x + 200.0 && (end.y - start.y).abs() < 10.0,
        "start should be aligned left and end right on the same trim rail: {start:?}, {end:?}"
    );
    for size in [egui::vec2(800.0, 600.0), egui::vec2(640.0, 480.0)] {
        for _ in 0..4 {
            let _ = ctx.run(input(size), |ctx| app.dialog_window(ctx));
        }
        let rect = ctx.memory(|m| m.area_rect(egui::Id::new("form"))).unwrap();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                .expand(1.0)
                .contains_rect(rect),
            "boundary editor must fit smaller displays: {rect:?} in {size:?}"
        );
    }
}

#[test]
fn splash_has_no_white_card_or_border() {
    fn white_panel(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Rect(r) => {
                r.brush.is_none()
                    && r.fill.a() > 0
                    && r.fill.r() == r.fill.a()
                    && r.fill.g() == r.fill.a()
                    && r.fill.b() == r.fill.a()
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(white_panel),
            _ => false,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut output = None;
    for _ in 0..4 {
        app.splash_started = Some(std::time::Instant::now() - Duration::from_millis(400));
        output = Some(ctx.run(input(egui::vec2(900.0, 700.0)), |ctx| {
            app.splash_window(ctx)
        }));
    }
    let shapes = output.unwrap().shapes;
    assert!(
        texture_rect(&shapes, app.splash_texture.id()).is_some(),
        "logo must remain visible"
    );
    assert!(
        !shapes.iter().any(|s| white_panel(&s.shape)),
        "transparent splash must not paint a white card or border"
    );
}

#[test]
fn fullscreen_player_covers_the_entire_library_with_an_opaque_surface() {
    fn covers(shape: &egui::Shape, viewport: egui::Rect) -> bool {
        match shape {
            egui::Shape::Rect(r) => r.fill == Color32::BLACK && r.rect.contains_rect(viewport),
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| covers(s, viewport)),
            _ => false,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut viewer = test_viewer(&ctx);
    viewer.fullscreen = true;
    app.viewer = Some(viewer);
    for size in [
        egui::vec2(1280.0, 820.0),
        egui::vec2(900.0, 600.0),
        egui::vec2(1600.0, 1000.0),
    ] {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
        for _ in 0..4 {
            let output = ctx.run(input(size), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(Color32::RED))
                    .show(ctx, |ui| {
                        ui.label("Library beneath the player");
                    });
                app.player_window(ctx);
            });
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|s| s.clip_rect.contains_rect(viewport.shrink(0.1))
                        && covers(&s.shape, viewport.shrink(0.1))),
                "fullscreen must cover every gap immediately at {size:?}"
            );
        }
    }
}

#[test]
fn play_pause_buttons_are_hidden_only_in_fullscreen() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut viewer = test_viewer(&ctx);
    let texture = ctx.load_texture(
        "fullscreen-playback",
        egui::ColorImage::filled([640, 360], Color32::GRAY),
        Default::default(),
    );
    let id = texture.id();
    viewer.texture = Some(texture);
    app.viewer = Some(viewer);
    for fullscreen in [false, true, false] {
        app.viewer.as_mut().unwrap().fullscreen = fullscreen;
        for paused in [false, true] {
            app.viewer.as_ref().unwrap().player.pause(paused);
            let mut output = None;
            for _ in 0..4 {
                output = Some(player_input(&ctx, &mut app, vec![]));
            }
            let output = output.unwrap();
            let texts = text_positions(&output.shapes);
            assert_eq!(
                texts
                    .iter()
                    .any(|(text, _)| text == "Play" || text == "Pause"),
                !fullscreen,
                "Play/Pause visibility must follow fullscreen state"
            );
            assert!(texts.iter().any(|(text, _)| text
                == if fullscreen {
                    "Exit full screen"
                } else {
                    "Full screen"
                }));
            assert!(
                texts.iter().any(|(text, _)| text == "Replay"),
                "independent Replay control must remain"
            );
            if fullscreen {
                let image = texture_rect(&output.shapes, id).unwrap();
                click_player(&ctx, &mut app, image.center());
                assert_eq!(
                    app.viewer.as_ref().unwrap().player.paused(),
                    !paused,
                    "video click works without Play/Pause"
                );
                for pressed in [true, false] {
                    player_input(
                        &ctx,
                        &mut app,
                        vec![egui::Event::Key {
                            key: egui::Key::Space,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    );
                }
                assert_eq!(
                    app.viewer.as_ref().unwrap().player.paused(),
                    paused,
                    "Space toggles playback exactly once"
                );
            }
        }
    }
}

#[test]
fn resized_player_uses_the_largest_aspect_preserving_video_rectangle() {
    for source in [[1600, 900], [400, 300], [450, 800]] {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let mut viewer = test_viewer(&ctx);
        let texture = ctx.load_texture(
            "resizable-source",
            egui::ColorImage::filled(source, Color32::GRAY),
            Default::default(),
        );
        let id = texture.id();
        viewer.texture = Some(texture);
        viewer.window.maximized = true;
        app.viewer = Some(viewer);
        for fullscreen in [false, true] {
            app.viewer.as_mut().unwrap().fullscreen = fullscreen;
            let mut previous = 0.0;
            for size in [
                egui::vec2(1280.0, 1000.0),
                egui::vec2(1920.0, 1200.0),
                egui::vec2(900.0, 600.0),
            ] {
                let mut output = None;
                for _ in 0..4 {
                    output = Some(ctx.run(input(size), |ctx| app.player_window(ctx)));
                }
                let shapes = output.unwrap().shapes;
                let image = texture_rect(&shapes, id).unwrap();
                let start = text_positions(&shapes)
                    .into_iter()
                    .find(|(s, _)| s == "Tags")
                    .unwrap()
                    .1;
                let window = ctx
                    .memory(|m| m.area_rect(egui::Id::new("player")))
                    .unwrap();
                // The actual video area ends before metadata; controls and window
                // decorations consume less than this tolerance, with no fixed cap.
                let width = start.x - window.left() - 20.0;
                let height = window.height() - 145.0;
                let scale = (width / source[0] as f32).min(height / source[1] as f32);
                assert!(
                    image.width() >= source[0] as f32 * scale - 2.0
                        && image.height() >= source[1] as f32 * scale - 2.0,
                    "video underuses available player area: source {source:?}, image {image:?}, window {window:?}, expected scale {scale}"
                );
                assert!(
                    (image.width() / image.height() - source[0] as f32 / source[1] as f32).abs()
                        < 0.002
                );
                if size.x == 1920.0 {
                    assert!(
                        image.height() > previous + 30.0,
                        "enlarging must grow video"
                    );
                }
                previous = image.height();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(image));
            }
        }
    }
}

#[test]
fn fullscreen_labels_remain_readable_with_a_light_system_theme() {
    fn label_is_readable(shape: &egui::Shape) -> Option<bool> {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == "Layout test flight" => {
                Some(t.galley.job.sections.iter().all(|section| {
                    let c = t.override_text_color.unwrap_or_else(|| {
                        if section.format.color == Color32::PLACEHOLDER {
                            t.fallback_color
                        } else {
                            section.format.color
                        }
                    });
                    c.r() > 150 && c.g() > 150 && c.b() > 150
                }))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(label_is_readable),
            _ => None,
        }
    }
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    ctx.set_visuals(egui::Visuals::light());
    let mut viewer = test_viewer(&ctx);
    viewer.fullscreen = true;
    app.viewer = Some(viewer);
    let mut output = None;
    for _ in 0..4 {
        output = Some(player_input(&ctx, &mut app, vec![]));
    }
    assert_eq!(
        output
            .unwrap()
            .shapes
            .iter()
            .find_map(|s| label_is_readable(&s.shape)),
        Some(true),
        "fullscreen's opaque black backing needs readable foreground text in light OS theme"
    );
    assert!(
        !ctx.style().visuals.dark_mode,
        "fullscreen must not change the rest of the application's theme"
    );
}

fn session_of(flights: Vec<FlightView>) -> SessionView {
    SessionView {
        id: "session".into(),
        name: "Test session".into(),
        date: None,
        files: vec![],
        flights,
        segmentable: false,
        reducible: false,
        detection_profile: "standard".into(),
        location: None,
        notes: None,
        labels: vec![],
    }
}

/// The app with a worker the test plays: jobs it was handed, events it sends.
fn app_with_worker(
    ctx: &egui::Context,
) -> (
    App,
    mpsc::Receiver<crate::worker::Job>,
    mpsc::Sender<crate::worker::Event>,
) {
    let mut app = app(ctx);
    let (sender, jobs) = mpsc::channel();
    let (events, receiver) = mpsc::channel();
    app.worker = Worker {
        sender,
        events: receiver,
    };
    (app, jobs, events)
}

/// Clicks the text `label` drawn by `draw`.
fn click_text(ctx: &egui::Context, app: &mut App, label: &str, draw: fn(&mut App, &egui::Context)) {
    let size = egui::vec2(1280.0, 820.0);
    let mut output = ctx.run(input(size), |ctx| draw(app, ctx));
    for _ in 0..2 {
        output = ctx.run(input(size), |ctx| draw(app, ctx));
    }
    let position = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("{label} is not on screen"))
        .1
        + egui::vec2(5.0, 5.0);
    for pressed in [true, false] {
        let mut raw = input(size);
        raw.events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(raw, |ctx| draw(app, ctx));
    }
}

fn shown_texts(
    ctx: &egui::Context,
    app: &mut App,
    draw: fn(&mut App, &egui::Context),
) -> Vec<String> {
    // A new window is laid out over a few frames before it is drawn.
    let mut texts = vec![];
    for _ in 0..3 {
        let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(app, ctx));
        texts = text_positions(&output.shapes)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
    }
    texts
}

#[test]
fn a_flight_in_the_cloud_waits_for_its_videos_then_plays() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let in_cloud = FlightView {
        cloud: true,
        ..flight()
    };
    app.sessions = vec![session_of(vec![in_cloud.clone()])];
    app.play(&in_cloud, 0.0);
    let job = jobs.try_recv().expect("the download is queued");
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.cloud_wait_window(ctx)
    };
    events
        .send(crate::worker::Event::Progress {
            id: job.id,
            step: fpv_library::Step {
                line: "Downloading GH010042.MP4 (1/2)…".into(),
                done: 0,
                total: 2,
            },
        })
        .unwrap();
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "Downloading from the cloud"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Video 1 of 2"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("each one arrives whole")),
        "{texts:?}"
    );

    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(crate::worker::Reply::Playback {
                flight: in_cloud.id.clone(),
                at: 0.0,
                parts: vec![],
            }),
        })
        .unwrap();
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        !texts.iter().any(|t| t == "Downloading from the cloud"),
        "the wait is over: {texts:?}"
    );
    assert!(
        app.viewer.is_some(),
        "the player opens once the videos are here"
    );
}

#[test]
fn continuing_in_the_background_leaves_the_flight_unplayed() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let in_cloud = FlightView {
        cloud: true,
        ..flight()
    };
    app.sessions = vec![session_of(vec![in_cloud.clone()])];
    app.play(&in_cloud, 0.0);
    let job = jobs.try_recv().unwrap();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.cloud_wait_window(ctx)
    };
    click_text(&ctx, &mut app, "Continue in Background", draw);
    assert!(app.cloud_wait.is_none());
    assert!(
        app.tasks.iter().any(|t| t.id == job.id && t.running),
        "the download goes on"
    );
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(crate::worker::Reply::Playback {
                flight: in_cloud.id.clone(),
                at: 0.0,
                parts: vec![],
            }),
        })
        .unwrap();
    let _ = shown_texts(&ctx, &mut app, draw);
    assert!(
        app.viewer.is_none(),
        "nothing opens behind the viewer's back"
    );

    // A flight already here opens without a wait.
    let here = flight();
    app.play(&here, 0.0);
    assert!(app.cloud_wait.is_none());
}

#[test]
fn hover_strips_are_made_in_quiet_batches_after_a_task_changes_the_library() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let finish = |app: &mut App, id: u64, reply: crate::worker::Reply| {
        events
            .send(crate::worker::Event::Finished {
                id,
                result: Ok(reply),
            })
            .unwrap();
        app.receive(&ctx);
    };
    let strips_queued = |app: &App, jobs: &mpsc::Receiver<crate::worker::Job>| {
        jobs.try_iter()
            .filter(|job| {
                app.tasks
                    .iter()
                    .any(|t| t.id == job.id && t.label == "Hover strips")
            })
            .map(|job| job.id)
            .collect::<Vec<_>>()
    };
    let import = app.submit("Import “Alpes”", false, |_, _| unreachable!());
    let _ = jobs.try_recv();
    finish(
        &mut app,
        import,
        crate::worker::Reply::Done("3 imported".into()),
    );
    let batch = strips_queued(&app, &jobs);
    assert_eq!(batch.len(), 1, "an import that ended asks for its strips");
    assert!(!app.busy(), "a strips batch never disables the forms");

    let prepared = |made, more| {
        crate::worker::Reply::Strips(fpv_library::StripsPrepared {
            made,
            more,
            ..Default::default()
        })
    };
    finish(&mut app, batch[0], prepared(24, true));
    let next = strips_queued(&app, &jobs);
    assert_eq!(next.len(), 1, "frames left over: the next batch");
    finish(&mut app, next[0], prepared(3, false));
    assert!(
        strips_queued(&app, &jobs).is_empty(),
        "all made: the job ends"
    );
    assert_eq!(app.strips.made, 27);
}

#[test]
fn errors_show_as_unread_notifications_and_opening_the_list_reads_them() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, events) = app_with_worker(&ctx);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.task_bar(ctx)
    };
    for _ in 0..2 {
        events
            .send(crate::worker::Event::Fatal(
                "Cannot read PICT0032.AVI".into(),
            ))
            .unwrap();
    }
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "Notifications (1 new)"),
        "{texts:?}"
    );

    click_text(&ctx, &mut app, "Notifications (1 new)", draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "Cannot read PICT0032.AVI"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "2 times"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t.ends_with("new)")),
        "read once seen: {texts:?}"
    );

    click_text(&ctx, &mut app, "Dismiss", draw);
    assert!(app.notices.items().is_empty());
}

#[test]
fn a_re_segment_names_what_it_loses_and_waits_for_consent_but_a_first_one_does_not() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.dialog_window(ctx);

    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let segmented = session_of(vec![FlightView {
        action_cam: false,
        title: Some("Bando".into()),
        ..flight()
    }]);
    app.dialog = Some(crate::dialogs::Dialog::segment(&segmented));
    app.sessions = vec![segmented];
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "• 1 title"), "{texts:?}");
    click_text(&ctx, &mut app, "Save and segment", draw);
    assert!(
        jobs.try_recv().is_err(),
        "nothing runs before the pilot agrees"
    );
    click_text(
        &ctx,
        &mut app,
        "I accept replacing the existing DVR segmentation",
        draw,
    );
    click_text(&ctx, &mut app, "Save and segment", draw);
    assert!(jobs.try_recv().is_ok(), "agreed: it runs");

    // Only takes so far: a first segmentation loses nothing.
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let fresh = session_of(vec![flight()]);
    app.dialog = Some(crate::dialogs::Dialog::segment(&fresh));
    app.sessions = vec![fresh];
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        !texts.iter().any(|t| t.starts_with("I accept")),
        "{texts:?}"
    );
    click_text(&ctx, &mut app, "Save and segment", draw);
    assert!(jobs.try_recv().is_ok());
}

fn viewer_of(ctx: &egui::Context, flight: FlightView) -> Viewer {
    Viewer {
        flight,
        player: Player::new(vec![], None, 0.0, ctx.clone()),
        texture: None,
        mark: None,
        figure: String::new(),
        label: String::new(),
        volume: 1.0,
        remembered_volume: 1.0,
        playback_muted: false,
        muted: false,
        fullscreen: false,
        window: Default::default(),
        seek_time: None,
    }
}

#[test]
fn the_player_suggests_tags_and_figures_the_library_already_uses() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let tagged = FlightView {
        id: "other".into(),
        labels: vec!["Power gates".into(), "freestyle".into()],
        ..flight()
    };
    let mut session = session_of(vec![tagged, flight()]);
    session.labels = vec!["Gates".into()];
    app.sessions = vec![session];
    app.viewer = Some(viewer_of(&ctx, flight()));
    app.viewer.as_mut().unwrap().label = "ga".into();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.player_window(ctx);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "Gates"),
        "a session's tag: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "Power gates"),
        "a flight's tag: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "freestyle"), "{texts:?}");
    click_text(&ctx, &mut app, "Power gates", draw);
    assert_eq!(app.viewer.as_ref().unwrap().label, "Power gates");

    // Figures: the names marked on any flight.
    app.sessions[0].flights[0]
        .figures
        .push(fpv_library::TagView {
            id: "tag".into(),
            label: "Power loop".into(),
            note: None,
            start: 1.0,
            end: 2.0,
            marker: false,
        });
    app.viewer.as_mut().unwrap().figure = "pow".into();
    click_text(&ctx, &mut app, "Power loop", draw);
    assert_eq!(app.viewer.as_ref().unwrap().figure, "Power loop");
}

#[test]
fn a_recording_deleted_with_its_flight_loads_only_when_restore_is_ticked() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let mut dialog = crate::dialogs::Dialog::load();
    dialog.reply(crate::worker::Reply::Folders(vec![
        fpv_library::FolderView {
            name: "2026-10-23 Bench DJI".into(),
            session: "Bench".into(),
            date: Some("2026-10-23".into()),
            recordings: 2,
            new: 0,
            deleted: vec!["DJI_0100.MP4".into()],
        },
    ]));
    app.dialog = Some(dialog);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.dialog_window(ctx);
    click_text(
        &ctx,
        &mut app,
        "2026-10-23 Bench DJI · 0 new / 2 recordings",
        draw,
    );
    click_text(&ctx, &mut app, "Load selected folders", draw);
    assert!(
        jobs.try_recv().is_err(),
        "nothing new: the folder cannot be picked"
    );

    let restore = "Restore 1 recording(s) previously deleted with their flight";
    click_text(&ctx, &mut app, restore, draw);
    // Ticking it picks the folder: Load now runs.
    click_text(&ctx, &mut app, "Load selected folders", draw);
    assert!(jobs.try_recv().is_ok());
}

#[test]
fn progress_reaches_only_its_own_task_and_a_failure_is_notified() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, events) = app_with_worker(&ctx);
    let segment = app.submit("Segment “Alpes”", false, |_, _| unreachable!());
    let export = app.submit("Export Flight 1", false, |_, _| unreachable!());
    events
        .send(crate::worker::Event::Progress {
            id: segment,
            step: fpv_library::Step {
                line: "Detecting PICT0003.AVI (1/2)…".into(),
                done: 1,
                total: 2,
            },
        })
        .unwrap();
    events
        .send(crate::worker::Event::Finished {
            id: export,
            result: Err("disk full".into()),
        })
        .unwrap();
    app.receive(&ctx);
    let task = |id| app.tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!((task(segment).done, task(segment).total), (1, 2));
    assert!(task(segment).running && !task(segment).failed);
    assert_eq!(
        (task(export).done, task(export).total),
        (0, 0),
        "not the other job's progress"
    );
    assert!(!task(export).running && task(export).failed);
    assert_eq!(task(export).line, "disk full");
    assert_eq!(app.notices.items()[0].text, "disk full");
}

#[test]
fn the_session_list_and_the_cards_say_what_is_only_in_the_cloud() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let file = |cloud| fpv_library::FileView {
        id: "f".into(),
        name: "GH010042.MP4".into(),
        width: 1920,
        height: 1080,
        fps: 30.0,
        duration: 60.0,
        codec: "h264".into(),
        action_cam: true,
        source: "GoPro".into(),
        format: String::new(),
        reduced: false,
        elsewhere: None,
        cloud,
        freeable: false,
    };
    let mut session = session_of(vec![FlightView {
        cloud: true,
        ..flight()
    }]);
    session.files = vec![file(true), file(false)];
    app.browse.session = Some(session.id.clone());
    app.sessions = vec![session];
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.sidebar(ctx);
        app.browser(ctx)
    };
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "partly in the cloud"),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "In the cloud"),
        "the card: {texts:?}"
    );
}

#[test]
fn a_flight_opened_muted_by_the_setting_can_be_unmuted_from_the_speaker() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, events) = app_with_worker(&ctx);
    app.config.mute_videos = true;
    app.sessions = vec![session_of(vec![flight()])];
    let part = fpv_library::NativeSegment {
        segment: fpv_library::Segment {
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
    events
        .send(crate::worker::Event::Finished {
            id: 99,
            result: Ok(crate::worker::Reply::Playback {
                flight: flight().id,
                at: 0.0,
                parts: vec![part],
            }),
        })
        .unwrap();
    app.receive(&ctx);
    let viewer = app.viewer.as_mut().unwrap();
    assert!(!viewer.muted, "it has sound: the speaker stays usable");
    assert!(viewer.playback_muted && !viewer.player.audio_enabled());
    super::toggle_mute(viewer);
    assert!(!viewer.playback_muted);
    assert!(viewer.player.audio_enabled(), "the sound comes back");
}
