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
        export_dialogs: Default::default(),
        dialog_request: None,
        viewer: None,
        cloud_wait: None,
        detached: HashSet::new(),
        strips: crate::strips::Strips::default(),
        strip_batch: None,
        title_edit: None,
        followed_session: None,
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
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
        app.player_window(ctx)
    });
    assert!(
        !text_positions(&output.shapes)
            .iter()
            .any(|(text, _)| text == "×"),
        "the title bar's × is the player's only one"
    );
    // The title bar's close button: egui draws it as lines, so it is found
    // as the square widget at the top right of the player's layer.
    let player = ctx
        .memory(|m| m.area_rect(egui::Id::new("player")))
        .unwrap();
    let close = ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .get_layer(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("player"),
            ))
            .filter(|w| w.sense.senses_click() && w.rect.width() == w.rect.height())
            .filter(|w| w.rect.top() < player.top() + 40.0)
            .max_by(|a, b| a.rect.right().total_cmp(&b.rect.right()))
            .map(|w| w.rect.center())
            .expect("a close button in the player's title bar")
    });
    for pressed in [true, false] {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![
            egui::Event::PointerMoved(close),
            egui::Event::PointerButton {
                pos: close,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(raw, |ctx| app.player_window(ctx));
    }
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
    // Each tab: its name, and its count just below it.
    for name in ["Sessions", "All flights", "Figures"] {
        let at = positions
            .iter()
            .find(|(text, _)| text == name)
            .unwrap_or_else(|| panic!("missing tab {name:?}"))
            .1;
        assert!(
            positions
                .iter()
                .any(|(text, pos)| text == "1" && pos.y > at.y && (pos.y - at.y) < 30.0),
            "{name}'s count below it"
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
        .find(|(s, _)| s == "Tag the flight: freestyle, gates…")
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
                // 30: the window's 16 px margin (2026-10-28) and the gap to the tags.
                let width = start.x - window.left() - 30.0;
                // 185: the roomier rows (2026-10-28) under the video take ~30 px more.
                let height = window.height() - 185.0;
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
    for _ in 0..7 {
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
    // A new window is laid out over a few frames before it is drawn, and a
    // scroll area grows to its content over a few more.
    let mut texts = vec![];
    for _ in 0..8 {
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
                playback_mute: None,
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
                playback_mute: None,
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
        "I accept replacing the existing segmentation",
        draw,
    );
    click_text(&ctx, &mut app, "Save and segment", draw);
    assert!(jobs.try_recv().is_ok(), "agreed: it runs");

    // No flights yet: a first segmentation loses nothing.
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let fresh = session_of(vec![]);
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
    let texts = shown_texts(&ctx, &mut app, draw);
    for expected in ["Nothing new", "Loads into Bench · 2026-10-23"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Recordings already in ")),
        "{texts:?}"
    );
    click_text(&ctx, &mut app, "2026-10-23 Bench DJI", draw);
    click_text(&ctx, &mut app, "Load", draw);
    assert!(
        jobs.try_recv().is_err(),
        "nothing new: the folder cannot be picked"
    );

    let restore = "1 recording deleted with its flight (DJI_0100.MP4) — load again into Bench";
    click_text(&ctx, &mut app, restore, draw);
    // Ticking it picks the folder: Load now runs.
    click_text(&ctx, &mut app, "Load", draw);
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
                playback_mute: None,
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

#[test]
fn a_long_task_only_holds_back_what_it_conflicts_with() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let mut other = session_of(vec![FlightView {
        id: "other-flight".into(),
        ..flight()
    }]);
    other.id = "other".into();
    app.sessions = vec![session_of(vec![flight()]), other];
    app.browse.session = Some("session".into());
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    let likes = |app: &App, jobs: &mpsc::Receiver<crate::worker::Job>| {
        jobs.try_iter()
            .filter(|job| {
                app.tasks
                    .iter()
                    .any(|t| t.id == job.id && t.label == "Like flight")
            })
            .count()
    };

    // An export changes nothing anyone waits on: the batch export dialog,
    // with no scope of its own, stays usable.
    app.submit("Export flights", false, |_, _| unreachable!());
    let _ = jobs.try_recv();
    assert!(!app.busy_with(&crate::Scope::None));

    // An import runs: a like on a flight has nothing to wait for.
    app.submit_in(
        crate::Scope::Library,
        "Import recordings",
        false,
        |_, _| unreachable!(),
    );
    let _ = jobs.try_recv();
    click_text(&ctx, &mut app, "♡", draw);
    assert_eq!(
        likes(&app, &jobs),
        1,
        "the library stays usable during an import"
    );

    // Another session is re-segmented: still nothing to wait for.
    app.submit_in(
        crate::Scope::Session("other".into()),
        "Segment session",
        false,
        |_, _| unreachable!(),
    );
    let _ = jobs.try_recv();
    click_text(&ctx, &mut app, "♡", draw);
    assert_eq!(likes(&app, &jobs), 1);

    // This flight's own session is being remade: its like waits.
    app.submit_in(
        crate::Scope::Session("session".into()),
        "Segment session",
        false,
        |_, _| unreachable!(),
    );
    let _ = jobs.try_recv();
    click_text(&ctx, &mut app, "♡", draw);
    assert_eq!(likes(&app, &jobs), 0, "the flight is being remade");
}

#[test]
fn the_tag_filters_belong_to_all_flights_not_to_a_session() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![FlightView {
        labels: vec!["gates".into()],
        ..flight()
    }])];
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.sidebar(ctx);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        !texts.iter().any(|t| t == "gates"),
        "no tag filter in a session: {texts:?}"
    );
    app.browse.view = crate::browse::View::Flights;
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Match all tags"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "gates (1)"), "{texts:?}");
}

#[test]
fn escape_closes_a_dialog_then_the_player_then_the_panels() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let press = |app: &mut App| {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }];
        let _ = ctx.run(raw, |ctx| app.escape(ctx));
    };
    app.sessions = vec![session_of(vec![flight()])];
    app.viewer = Some(viewer_of(&ctx, flight()));
    app.dialog = Some(crate::dialogs::Dialog::segment(&app.sessions[0]));
    app.show_notices = true;
    press(&mut app);
    assert!(
        app.dialog.is_none() && app.viewer.is_some(),
        "the dialog first"
    );
    press(&mut app);
    assert!(app.viewer.is_none() && app.show_notices, "then the player");
    press(&mut app);
    assert!(!app.show_notices, "then the lists");
    app.show_about = true;
    press(&mut app);
    assert!(!app.show_about, "About closes like a dialog");
}

#[test]
fn an_export_across_rates_asks_how_instead_of_failing() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.dialog_window(ctx)
    };
    for action_cam in [false, true] {
        let (mut app, jobs, events) = app_with_worker(&ctx);
        let dvr = FlightView {
            action_cam,
            recordings: 2,
            ..flight()
        };
        app.sessions = vec![session_of(vec![dvr.clone()])];
        let mut dialog = crate::dialogs::Dialog::export("Test session", &dvr);
        dialog.reply(crate::worker::Reply::Picked(Some("/tmp/out.avi".into())));
        app.dialog = Some(dialog);
        click_text(&ctx, &mut app, "Export", draw);
        let export = jobs.try_recv().expect("a lossless export first");
        assert!(app.dialog.is_none(), "accepted export closes immediately");
        events
            .send(crate::worker::Event::Finished {
                id: export.id,
                result: Ok(crate::worker::Reply::Mismatch(
                    "the recordings run at 29.887 and 24.486 fps".into(),
                )),
            })
            .unwrap();
        let texts = shown_texts(&ctx, &mut app, draw);
        assert!(app.dialog.is_some(), "the dialog stays to ask");
        assert!(
            texts.iter().any(|t| t == "Different frame rates"),
            "{texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("29.887")), "{texts:?}");
        assert!(
            texts.iter().any(|t| t == "Re-encode into one file"),
            "explicit re-encoding works for every codec: {texts:?}"
        );
        click_text(&ctx, &mut app, "Export 2 parts separately", draw);
        assert!(jobs.try_recv().is_ok(), "exported in parts");
    }
}

/// Right-clicks at `position`, as `click_text` clicks.
fn right_click(
    ctx: &egui::Context,
    app: &mut App,
    position: egui::Pos2,
    draw: fn(&mut App, &egui::Context),
) {
    for pressed in [true, false] {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(raw, |ctx| draw(app, ctx));
    }
}

#[test]
fn a_right_click_opens_a_sessions_menu_and_a_flights() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.sidebar(ctx);
        app.browser(ctx)
    };
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let row = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == "Test session")
        .unwrap()
        .1;
    right_click(&ctx, &mut app, row + egui::vec2(5.0, 5.0), draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Edit session…"), "{texts:?}");

    // Open the session, then right-click its flight's picture.
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    app.browse.session = Some("session".into());
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let title = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == "Layout test flight")
        .unwrap()
        .1;
    right_click(&ctx, &mut app, title - egui::vec2(-20.0, 60.0), draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Export…"), "{texts:?}");
}

#[test]
fn a_session_row_names_its_sources_its_reduction_and_an_earlier_folder() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let file = |source: &str, action_cam, reduced, elsewhere: Option<&str>| fpv_library::FileView {
        id: "f".into(),
        name: "PICT0032.AVI".into(),
        width: 640,
        height: 480,
        fps: 29.887,
        duration: 60.0,
        codec: "mjpeg".into(),
        action_cam,
        source: source.into(),
        format: String::new(),
        reduced,
        elsewhere: elsewhere.map(Into::into),
        cloud: false,
        freeable: false,
    };
    let mut session = session_of(vec![flight()]);
    session.files = vec![
        file("DVR", false, true, None),
        file("DVR", false, false, Some("/Volumes/Old")),
        file("GoPro HERO11", true, false, None),
    ];
    app.sessions = vec![session];
    let texts = shown_texts(&ctx, &mut app, |app, ctx| app.sidebar(ctx));
    for expected in ["DVR · partly reduced", "GoPro HERO11", "elsewhere"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
}

#[test]
fn a_card_says_chapters_marks_session_tags_and_scrubs_with_the_time() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let take = FlightView {
        recordings: 2,
        labels: vec!["gates".into()],
        ..flight()
    };
    let mut session = session_of(vec![take]);
    session.labels = vec!["race".into()];
    app.sessions = vec![session];
    app.browse.session = Some("session".into());
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    let texts = shown_texts(&ctx, &mut app, draw);
    for expected in ["2 chapters", "gates", "race", "1:00.0"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }

    // The pointer halfway across the picture: that frame, that time.
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let title = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == "Layout test flight")
        .unwrap()
        .1;
    let pointer = egui::pos2(title.x + 60.0, title.y - 60.0);
    let mut texts = vec![];
    for _ in 0..3 {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![egui::Event::PointerMoved(pointer)];
        let output = ctx.run(raw, |ctx| draw(&mut app, ctx));
        texts = text_positions(&output.shapes)
            .into_iter()
            .map(|(t, _)| t)
            .collect();
    }
    assert!(
        app.requested
            .iter()
            .any(|key| key.starts_with("strip:test-flight:")),
        "the frame under the pointer: {:?}",
        app.requested
    );
    assert!(
        !texts.iter().any(|t| t == "1:00.0"),
        "the time under the pointer: {texts:?}"
    );
}

#[test]
fn a_figure_card_shows_the_frame_at_its_moment() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let mut figured = flight();
    figured.figures.push(fpv_library::TagView {
        id: "tag".into(),
        label: "power loop".into(),
        note: None,
        start: 30.0,
        end: 32.0,
        marker: false,
    });
    app.sessions = vec![session_of(vec![figured])];
    app.browse.view = crate::browse::View::Figures;
    let _ = shown_texts(&ctx, &mut app, |app, ctx| app.browser(ctx));
    assert!(
        app.requested.contains("strip:test-flight:12"),
        "30 s into 60: frame 12 of 24, not the poster: {:?}",
        app.requested
    );
}

#[test]
fn a_cards_title_is_renamed_in_place_and_its_export_is_a_click_away() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    app.browse.session = Some("session".into());
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let title = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == "Layout test flight")
        .unwrap()
        .1
        + egui::vec2(5.0, 5.0);
    // A double-click: two clicks in quick succession.
    for pressed in [true, false, true, false] {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![
            egui::Event::PointerMoved(title),
            egui::Event::PointerButton {
                pos: title,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(raw, |ctx| draw(&mut app, ctx));
    }
    assert!(app.title_edit.is_some(), "editing in place");
    for events in [
        vec![egui::Event::Text(" II".into())],
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    ] {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = events;
        let _ = ctx.run(raw, |ctx| draw(&mut app, ctx));
    }
    assert!(app.title_edit.is_none());
    let saved: Vec<_> = jobs
        .try_iter()
        .filter(|job| {
            app.tasks
                .iter()
                .any(|t| t.id == job.id && t.label == "Edit flight title")
        })
        .collect();
    assert_eq!(saved.len(), 1, "Enter saves it");

    // The export icon sits between the duration and the "…".
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let texts = text_positions(&output.shapes);
    let at = |label: &str| texts.iter().find(|(text, _)| text == label).unwrap().1;
    let (duration, dots) = (at("1:00.0"), at("…"));
    // The "…" button's padding and the row's spacing, then half the icon.
    let icon = egui::pos2(dots.x - 30.0, dots.y + 7.0);
    assert!(icon.x > duration.x, "right of the duration");
    for pressed in [true, false] {
        let mut raw = input(egui::vec2(1280.0, 820.0));
        raw.events = vec![
            egui::Event::PointerMoved(icon),
            egui::Event::PointerButton {
                pos: icon,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(raw, |ctx| draw(&mut app, ctx));
    }
    assert!(app.dialog.is_some(), "the export dialog");
}

#[test]
fn the_player_says_its_chapter_and_how_to_mark_a_figure() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let chapter = |offset: f64| fpv_library::NativeSegment {
        segment: fpv_library::Segment {
            path: "/nonexistent/GH010042.MP4".into(),
            start: 0.0,
            end: 30.0,
            offset,
        },
        width: 1920,
        height: 1080,
        has_audio: false,
        all_intra: false,
    };
    let mut viewer = viewer_of(&ctx, flight());
    viewer.player = Player::new(vec![chapter(0.0), chapter(30.0)], None, 0.0, ctx.clone());
    viewer.player.pause(true);
    app.viewer = Some(viewer);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.player_window(ctx);
    let texts = shown_texts(&ctx, &mut app, draw);
    for expected in [
        "chapter 1/2",
        "No tags yet.",
        "No figures yet.",
        "Add Point",
        "Mark In (or I) at the start, play to the end, name it, Enter.",
        "Figure name (split-s, power loop…)",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    app.viewer.as_mut().unwrap().mark = Some(0.0);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Marked at 0:00.0"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.starts_with("Add Range to ")),
        "{texts:?}"
    );
}

#[test]
fn the_boundary_editor_nudges_by_frame_and_resets_to_what_is_saved() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let mut cut = flight();
    cut.clips = vec![fpv_library::ClipView {
        id: "clip".into(),
        file_id: "file".into(),
        file_name: "PICT0032.AVI".into(),
        start_frame: 100,
        end_frame: 200,
        fps: 25.0,
        last_frame: 999,
        file_duration: 40.0,
        entry: "unknown".into(),
        exit: "nosignal".into(),
        thumbnail: None,
    }];
    app.sessions = vec![session_of(vec![cut.clone()])];
    app.dialog = Some(crate::dialogs::Dialog::editor(&cut));
    // Tall enough to show the editor whole, below its preview.
    let size = egui::vec2(1280.0, 1400.0);
    let texts = |app: &mut App| {
        let mut texts = vec![];
        for _ in 0..3 {
            let output = ctx.run(input(size), |ctx| app.dialog_window(ctx));
            texts = text_positions(&output.shapes);
        }
        texts
    };
    let click = |app: &mut App, label: &str| {
        let at = texts(app)
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1
            + egui::vec2(5.0, 5.0);
        for pressed in [true, false] {
            let mut raw = input(size);
            raw.events = vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run(raw, |ctx| app.dialog_window(ctx));
        }
    };
    let shown = |app: &mut App, label: &str| texts(app).iter().any(|(text, _)| text == label);
    for expected in [
        "−1 s",
        "+1 fr",
        "4.000 s · frame 100",
        "4.000 s to 8.000 s at 25.000 fps",
        "Detected on a 0.25 s grid — nudge to the exact frame.",
    ] {
        assert!(shown(&mut app, expected), "{expected}");
    }
    assert!(!shown(&mut app, "changed — not saved"));
    click(&mut app, "+1 fr");
    assert!(shown(&mut app, "4.040 s · frame 101"));
    assert!(shown(&mut app, "changed — not saved"));
    click(&mut app, "Reset");
    assert!(
        shown(&mut app, "4.000 s · frame 100"),
        "back to the saved start"
    );
    assert!(!shown(&mut app, "changed — not saved"));
}

#[test]
fn settings_show_each_cache_kind_and_ask_before_emptying_one() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let config = fpv_library::Config {
        folder_pattern: "{session}".into(),
        ..Default::default()
    };
    let mut dialog = crate::dialogs::Dialog::settings(&config);
    dialog.reply(crate::worker::Reply::Cache(fpv_library::CacheUsage {
        proxies: 257_000_000,
        strips: 7_500_000,
        posters: 548_000,
        previews: 0,
    }));
    app.dialog = Some(dialog);
    // Tall enough to show the dialog whole.
    let size = egui::vec2(1280.0, 1400.0);
    let texts = |app: &mut App| {
        let mut texts = vec![];
        for _ in 0..8 {
            let output = ctx.run(input(size), |ctx| app.dialog_window(ctx));
            texts = text_positions(&output.shapes);
        }
        texts
    };
    let click = |app: &mut App, label: &str| {
        let at = texts(app)
            .into_iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("{label} is not on screen"))
            .1
            + egui::vec2(5.0, 5.0);
        for pressed in [true, false] {
            let mut raw = input(size);
            raw.events = vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run(raw, |ctx| app.dialog_window(ctx));
        }
    };
    let shown = |app: &mut App, label: &str| texts(app).iter().any(|(text, _)| text == label);
    for expected in [
        "   Hover strips",
        "7.5 MB",
        "548 KB",
        "257.0 MB",
        "Empty All (265.0 MB)…",
    ] {
        assert!(shown(&mut app, expected), "{expected}");
    }
    click(&mut app, "Empty…");
    assert!(jobs.try_recv().is_err(), "it asks first");
    assert!(shown(
        &mut app,
        "Empty hover strips (7.5 MB)? Nothing is lost: they are made again when needed."
    ));
    click(&mut app, "Empty");
    assert!(jobs.try_recv().is_ok(), "emptied once agreed");

    click(&mut app, "Default");
    assert!(
        shown(&mut app, "{date} {session} {source}"),
        "the default pattern is back"
    );
}

#[test]
fn the_task_bar_counts_marks_times_and_clears_what_is_finished() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, events) = app_with_worker(&ctx);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.task_bar(ctx)
    };
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "No task running")
    );
    app.show_tasks = true;
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "Nothing has run yet.")
    );

    let import = app.submit("Import recordings", false, |_, _| unreachable!());
    let export = app.submit("Export Flight 1", false, |_, _| unreachable!());
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "2 tasks running"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "0 s"),
        "each task's time: {texts:?}"
    );

    events
        .send(crate::worker::Event::Finished {
            id: import,
            result: Ok(crate::worker::Reply::Done("3 imported".into())),
        })
        .unwrap();
    events
        .send(crate::worker::Event::Finished {
            id: export,
            result: Err("disk full".into()),
        })
        .unwrap();
    let texts = shown_texts(&ctx, &mut app, draw);
    for expected in ["1 failed", "✔", "✖", "3 imported", "disk full"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    click_text(&ctx, &mut app, "Clear finished", draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "No task running"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Nothing has run yet."),
        "{texts:?}"
    );
}

#[test]
fn an_empty_grid_says_why_and_offers_the_way_out() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);

    // Nothing open yet.
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "No session selected"),
        "{texts:?}"
    );
    click_text(&ctx, &mut app, "Import Folder…", draw);
    assert!(app.dialog.is_some(), "the import dialog");
    app.dialog = None;
    click_text(&ctx, &mut app, "change", draw);
    assert!(app.dialog.is_some(), "Settings, to change where imports go");

    // A session never segmented.
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let mut fresh = session_of(vec![]);
    fresh.segmentable = true;
    app.sessions = vec![fresh];
    app.browse.session = Some("session".into());
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "No flights yet")
    );
    click_text(&ctx, &mut app, "Segment", draw);
    assert!(app.dialog.is_some(), "the segment dialog");

    // Filters that match nothing.
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    app.browse.view = crate::browse::View::Flights;
    app.browse.flights.labels = vec!["race".into()];
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "No flights match these filters"),
        "{texts:?}"
    );
    click_text(&ctx, &mut app, "Reset filters", draw);
    assert!(app.browse.flights.labels.is_empty());
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "Layout test flight")
    );
}

#[test]
fn the_sidebar_pages_its_sessions_and_follows_the_one_opened() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.sidebar(ctx);
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "No sessions yet.")
    );
    app.sessions = (0..12)
        .map(|n| {
            let mut session = session_of(vec![]);
            session.id = format!("s{n}");
            session.name = format!("Session {n}");
            session
        })
        .collect();
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Session 0"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "Session 10"),
        "ten a page: {texts:?}"
    );
    click_text(&ctx, &mut app, "2", draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Session 11"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Session 0"), "{texts:?}");
    // A session opened from elsewhere brings its page back.
    app.browse.session = Some("s3".into());
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Session 3"), "{texts:?}");
}

#[test]
fn importing_a_folder_asks_for_it_names_the_session_after_it_and_says_when_it_joins_one() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let mut alpes = session_of(vec![]);
    alpes.name = "Alpes".into();
    app.sessions = vec![alpes];
    app.open_import(None, String::new());
    let pick = jobs.try_recv().expect("the folder picker opens at once");
    assert!(
        app.tasks
            .iter()
            .any(|t| t.id == pick.id && t.label == "Choose path")
    );
    events
        .send(crate::worker::Event::Finished {
            id: pick.id,
            result: Ok(crate::worker::Reply::Picked(Some(
                "/media/pilot/Alpes".into(),
            ))),
        })
        .unwrap();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.dialog_window(ctx)
    };
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "Alpes"),
        "named after its folder: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Joins the existing session")),
        "Alpes exists: {texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Change…"), "{texts:?}");
}

#[test]
fn loading_folders_reads_pages_and_selects_all_new_at_once() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.dialog_window(ctx);
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.dialog = Some(crate::dialogs::Dialog::load());
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "Reading the import folder…")
    );
    app.dialog
        .as_mut()
        .unwrap()
        .reply(crate::worker::Reply::Folders(vec![]));
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "The import folder holds no folders.")
    );

    let folders = (0..12)
        .map(|n| fpv_library::FolderView {
            name: format!("2026-10-{:02} Spot {n} DJI", n + 1),
            session: format!("Spot {n}"),
            date: None,
            recordings: 1,
            new: 1,
            deleted: vec![],
        })
        .collect();
    app.dialog
        .as_mut()
        .unwrap()
        .reply(crate::worker::Reply::Folders(folders));
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "2026-10-01 Spot 0 DJI"),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "2026-10-11 Spot 10 DJI"),
        "ten a page: {texts:?}"
    );
    click_text(&ctx, &mut app, "Select all new", draw);
    assert!(
        shown_texts(&ctx, &mut app, draw)
            .iter()
            .any(|t| t == "Load 12 folders")
    );
    click_text(&ctx, &mut app, "2", draw);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t == "2026-10-11 Spot 10 DJI"),
        "page two: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "2026-10-01 Spot 0 DJI"),
        "{texts:?}"
    );
}

#[test]
fn confirmations_say_what_they_do_and_offer_nothing_pointless() {
    let ctx = egui::Context::default();
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.dialog_window(ctx);
    for button in ["Keep the Videos", "Also Trash the Videos"] {
        let (mut app, jobs, _events) = app_with_worker(&ctx);
        app.sessions = vec![session_of(vec![flight()])];
        app.dialog = Some(crate::dialogs::Dialog::remove_session(&app.sessions[0]));
        click_text(&ctx, &mut app, button, draw);
        assert!(jobs.try_recv().is_ok(), "{button} removes it");
    }

    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let mut reduce = crate::dialogs::Dialog::reduce(&app.sessions[0]);
    reduce.reply(crate::worker::Reply::Reduce(Default::default()));
    app.dialog = Some(reduce);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts.iter().any(|t| t.starts_with("Nothing to reduce")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "Reduce recordings"), "{texts:?}");

    let mut relocate = crate::dialogs::Dialog::relocate(&app.sessions[0]);
    relocate.reply(crate::worker::Reply::Relocation(Default::default()));
    app.dialog = Some(relocate);
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Every recording of this session is already")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "Move recordings"), "{texts:?}");
}

#[test]
fn moving_to_the_import_folder_is_offered_only_for_recordings_elsewhere() {
    let menu = |elsewhere: Option<&str>| {
        // A context each: a menu left open in one would be closed by the next click.
        let ctx = egui::Context::default();
        let (mut app, _jobs, _events) = app_with_worker(&ctx);
        let mut session = session_of(vec![flight()]);
        session.files = vec![fpv_library::FileView {
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
            elsewhere: elsewhere.map(Into::into),
            cloud: false,
            freeable: false,
        }];
        app.sessions = vec![session];
        let draw: fn(&mut App, &egui::Context) = |app, ctx| app.sidebar(ctx);
        // Settled layout first: the row's place on the very first frame moves.
        let _ = shown_texts(&ctx, &mut app, draw);
        let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
        let row = text_positions(&output.shapes)
            .into_iter()
            .find(|(text, _)| text == "Test session")
            .unwrap()
            .1;
        right_click(&ctx, &mut app, row + egui::vec2(5.0, 5.0), draw);
        shown_texts(&ctx, &mut app, draw)
    };
    let texts = menu(None);
    assert!(
        texts.iter().any(|t| t == "Edit session…"),
        "the menu is open: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "Move to import folder…"),
        "{texts:?}"
    );
    let texts = menu(Some("/Volumes/Old"));
    assert!(
        texts.iter().any(|t| t == "Move to import folder…"),
        "{texts:?}"
    );
}

#[test]
fn the_detection_profile_saves_alone_and_a_re_segment_says_it_is_running() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    let mut letz = session_of(vec![FlightView {
        action_cam: false,
        ..flight()
    }]);
    letz.segmentable = true;
    app.sessions = vec![letz];
    app.dialog = Some(crate::dialogs::Dialog::profile(&app.sessions[0]));
    let dialog: fn(&mut App, &egui::Context) = |app, ctx| app.dialog_window(ctx);
    let texts = shown_texts(&ctx, &mut app, dialog);
    assert!(
        texts
            .iter()
            .any(|t| t == "A change applies at the next segmentation."),
        "{texts:?}"
    );
    click_text(&ctx, &mut app, "Save", dialog);
    let saved: Vec<_> = jobs.try_iter().collect();
    assert!(
        saved.iter().any(|job| app
            .tasks
            .iter()
            .any(|t| t.id == job.id && t.label == "Save detection profile")),
        "saved without segmenting"
    );

    app.dialog = None;
    app.browse.session = Some("session".into());
    let browser: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    assert!(
        shown_texts(&ctx, &mut app, browser)
            .iter()
            .any(|t| t == "Re-segment")
    );
    app.submit_in(
        crate::Scope::Session("session".into()),
        "Segment session",
        false,
        |_, _| unreachable!(),
    );
    let texts = shown_texts(&ctx, &mut app, browser);
    assert!(
        texts
            .iter()
            .any(|t| t == "Re-segmenting — the flights below are replaced when it ends"),
        "{texts:?}"
    );
}

#[test]
fn a_session_of_takes_offers_no_detection_profile() {
    for segmentable in [false, true] {
        let ctx = egui::Context::default();
        let (mut app, _jobs, _events) = app_with_worker(&ctx);
        let mut session = session_of(vec![flight()]);
        session.segmentable = segmentable;
        app.sessions = vec![session];
        let draw: fn(&mut App, &egui::Context) = |app, ctx| app.sidebar(ctx);
        let _ = shown_texts(&ctx, &mut app, draw);
        let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
        let row = text_positions(&output.shapes)
            .into_iter()
            .find(|(text, _)| text == "Test session")
            .unwrap()
            .1;
        right_click(&ctx, &mut app, row + egui::vec2(5.0, 5.0), draw);
        let texts = shown_texts(&ctx, &mut app, draw);
        assert!(texts.iter().any(|t| t == "Edit session…"), "{texts:?}");
        assert_eq!(
            texts.iter().any(|t| t == "Detection profile…"),
            segmentable,
            "{texts:?}"
        );
        assert_eq!(
            texts.iter().any(|t| t == "Export labels…"),
            segmentable,
            "{texts:?}"
        );
    }
}

#[test]
fn the_filters_count_what_each_name_lists_and_what_matches() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let mut alpes = session_of(vec![FlightView {
        labels: vec!["gates".into()],
        ..flight()
    }]);
    alpes.date = Some("2026-10-17T00:00:00+02:00".into());
    app.sessions = vec![alpes];
    app.browse.view = crate::browse::View::Flights;
    let texts = shown_texts(&ctx, &mut app, |app, ctx| app.sidebar(ctx));
    for expected in ["gates (1)", "1 flight match"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    let mut dialog = crate::dialogs::Dialog::bulk(vec!["test-flight".into()]);
    dialog.reply(crate::worker::Reply::Picked(Some("/out".into())));
    app.dialog = Some(dialog);
    let texts = shown_texts(&ctx, &mut app, |app, ctx| app.dialog_window(ctx));
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("1 flight: each becomes")),
        "{texts:?}"
    );
}

/// A text drawn: its words, where, and the colours of its parts.
type PaintedText = (String, egui::Rect, Vec<Color32>);
/// A filled rectangle drawn.
type PaintedRect = (egui::Rect, Color32);

/// Each text drawn, with the colours of its parts, and every filled rectangle.
fn painted(shapes: &[egui::epaint::ClippedShape]) -> (Vec<PaintedText>, Vec<PaintedRect>) {
    fn walk(shape: &egui::Shape, texts: &mut Vec<PaintedText>, rects: &mut Vec<PaintedRect>) {
        match shape {
            egui::Shape::Text(t) => texts.push((
                t.galley.job.text.clone(),
                t.galley.rect.translate(t.pos.to_vec2()),
                t.galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect(),
            )),
            egui::Shape::Rect(r) => rects.push((r.rect, r.fill)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, texts, rects)),
            _ => {}
        }
    }
    let (mut texts, mut rects) = (vec![], vec![]);
    for s in shapes {
        walk(&s.shape, &mut texts, &mut rects);
    }
    (texts, rects)
}

/// Red at any opacity: a dialog fading in dims its colours, not their hue.
fn reddish(color: Color32) -> bool {
    let [r, g, b, _] = color.to_array();
    r > 60 && u16::from(r) > 2 * u16::from(g) && u16::from(r) > 2 * u16::from(b)
}

#[test]
fn dialogs_have_no_close_button_by_their_title() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.dialog = Some(crate::dialogs::Dialog::settings(&Default::default()));
    let texts = shown_texts(&ctx, &mut app, |app, ctx| app.dialog_window(ctx));
    assert!(texts.iter().any(|t| t == "Settings"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "×"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Close"),
        "still closable: {texts:?}"
    );
}

#[test]
fn what_deletes_is_red_and_a_checkbox_keeps_its_distance_from_its_label() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let mut dialog = crate::dialogs::Dialog::settings(&Default::default());
    dialog.reply(crate::worker::Reply::Cache(fpv_library::CacheUsage {
        proxies: 1_000_000,
        strips: 1_000_000,
        posters: 1_000_000,
        previews: 0,
    }));
    app.dialog = Some(dialog);
    let mut shapes = vec![];
    for _ in 0..20 {
        shapes = ctx
            .run(input(egui::vec2(1280.0, 1400.0)), |ctx| {
                app.dialog_window(ctx)
            })
            .shapes;
    }
    let (texts, rects) = painted(&shapes);
    let empty_all = texts
        .iter()
        .find(|(t, _, _)| t.starts_with("Empty All"))
        .unwrap();
    assert!(
        rects
            .iter()
            .any(|(rect, fill)| reddish(*fill) && rect.contains(empty_all.1.center())),
        "Empty All sits on red"
    );
    let save = texts.iter().find(|(t, _, _)| t == "Save settings").unwrap();
    assert!(
        !rects
            .iter()
            .any(|(rect, fill)| reddish(*fill) && rect.contains(save.1.center())),
        "Save is not red"
    );
    // "Mute videos": the box, then 8 px of air before the words.
    let label = texts.iter().find(|(t, _, _)| t == "Mute videos").unwrap().1;
    let gap = rects
        .iter()
        .filter(|(rect, _)| {
            rect.right() <= label.left() && (rect.center().y - label.center().y).abs() < 6.0
        })
        .map(|(rect, _)| label.left() - rect.right())
        .fold(f32::MAX, f32::min);
    assert!(gap >= 8.0, "checkbox to label: {gap} px");

    // The card menu's Delete flight… reads red.
    let mut app = app_with_worker(&ctx).0;
    app.sessions = vec![session_of(vec![flight()])];
    app.browse.session = Some("session".into());
    let draw: fn(&mut App, &egui::Context) = |app, ctx| app.browser(ctx);
    let output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx));
    let title = text_positions(&output.shapes)
        .into_iter()
        .find(|(text, _)| text == "Layout test flight")
        .unwrap()
        .1;
    right_click(&ctx, &mut app, title - egui::vec2(-20.0, 60.0), draw);
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = ctx
            .run(input(egui::vec2(1280.0, 820.0)), |ctx| draw(&mut app, ctx))
            .shapes;
    }
    let (texts, _) = painted(&shapes);
    let delete = texts
        .iter()
        .find(|(t, _, _)| t == "Delete flight…")
        .unwrap();
    assert_eq!(delete.2, [crate::DANGER_TEXT]);
    let export = texts.iter().find(|(t, _, _)| t == "Export…").unwrap();
    assert_ne!(export.2, [crate::DANGER_TEXT]);
}

#[test]
fn the_view_tabs_are_blocks_of_one_height_with_their_text_centred() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = ctx
            .run(input(egui::vec2(1280.0, 820.0)), |ctx| app.sidebar(ctx))
            .shapes;
    }
    let (texts, rects) = painted(&shapes);
    let at = |word: &str, below: f32| {
        texts
            .iter()
            .filter(|(t, r, _)| t == word && r.top() > below)
            .map(|(_, r, _)| *r)
            .next()
            .unwrap_or_else(|| panic!("{word} drawn"))
    };
    let mut heights = vec![];
    for (name, count) in [("Sessions", "1"), ("All flights", "1"), ("Figures", "0")] {
        let name_at = at(name, 0.0);
        let block = rects
            .iter()
            .filter(|(rect, _)| rect.contains(name_at.center()) && rect.height() < 80.0)
            .map(|(rect, _)| *rect)
            .min_by(|a, b| a.area().total_cmp(&b.area()))
            .unwrap();
        let count_at = texts
            .iter()
            .filter(|(t, r, _)| t == count && block.contains(r.center()))
            .map(|(_, r, _)| *r)
            .next()
            .unwrap_or_else(|| panic!("{name}'s count in its block"));
        for (what, text) in [(name, name_at), (count, count_at)] {
            assert!(
                (text.center().x - block.center().x).abs() < 1.5,
                "{what} centred in {name}'s block: {text:?} in {block:?}"
            );
        }
        assert!(
            count_at.top() >= name_at.bottom() - 1.0,
            "{name}: the count below the name"
        );
        heights.push(block.height());
    }
    assert!(
        heights.windows(2).all(|h| (h[0] - h[1]).abs() < 0.5),
        "one height: {heights:?}"
    );
}

#[test]
fn every_character_the_interface_writes_exists_in_its_fonts() {
    // A glyph the bundled fonts lack draws as an empty box ("□ Marked at").
    let ctx = egui::Context::default();
    crate::configure_style(&ctx);
    let _ = ctx.run(input(egui::vec2(100.0, 100.0)), |_| {});
    let sources = [
        include_str!("../ui.rs"),
        include_str!("../dialogs.rs"),
        include_str!("../boundary.rs"),
        include_str!("../browse.rs"),
        include_str!("../cloudwait.rs"),
        include_str!("../taskbar.rs"),
        include_str!("../main.rs"),
    ];
    let mut wanted: Vec<char> = vec![];
    for source in sources {
        // Only what sits inside string literals, comments aside.
        for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
            let mut inside = false;
            for c in line.chars() {
                if c == '"' {
                    inside = !inside;
                } else if inside && !c.is_ascii() && !wanted.contains(&c) {
                    wanted.push(c);
                }
            }
        }
    }
    let font = egui::FontId::proportional(14.0);
    let missing: Vec<char> = wanted
        .into_iter()
        .filter(|&c| !ctx.fonts_mut(|f| f.has_glyph(&font, c)))
        .collect();
    assert!(missing.is_empty(), "no glyph for {missing:?}");
}

#[test]
fn a_dialog_taller_than_the_window_scrolls_and_keeps_close_in_reach() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    let mut dialog = crate::dialogs::Dialog::settings(&Default::default());
    dialog.reply(crate::worker::Reply::Cache(fpv_library::CacheUsage {
        proxies: 1,
        strips: 1,
        posters: 1,
        previews: 1,
    }));
    app.dialog = Some(dialog);
    // A small screen: Settings is taller than it.
    let size = egui::vec2(1280.0, 600.0);
    let mut texts = vec![];
    for _ in 0..20 {
        let output = ctx.run(input(size), |ctx| app.dialog_window(ctx));
        texts = text_positions(&output.shapes);
    }
    let close = texts
        .iter()
        .find(|(text, _)| text == "Close")
        .expect("Close is drawn")
        .1;
    let title = texts.iter().find(|(text, _)| text == "Settings").unwrap().1;
    assert!(
        close.y < size.y - 10.0,
        "Close within the window: {close:?}"
    );
    assert!(title.y > 0.0, "the title too: {title:?}");
}

#[test]
fn only_the_chosen_view_tab_is_coloured() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let mut shapes = vec![];
    for _ in 0..3 {
        shapes = ctx
            .run(input(egui::vec2(1280.0, 820.0)), |ctx| app.sidebar(ctx))
            .shapes;
    }
    let (texts, _) = painted(&shapes);
    let colour = |word: &str| texts.iter().find(|(t, _, _)| t == word).unwrap().2[0];
    assert_eq!(colour("Sessions"), Color32::WHITE, "the chosen tab");
    assert_eq!(
        colour("All flights"),
        ctx.style().visuals.text_color(),
        "the others plain"
    );
    assert_eq!(colour("Figures"), ctx.style().visuals.text_color());
}

#[test]
fn submitting_export_closes_the_panel_and_reports_background_failure() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.dialog_window(ctx);
    };
    app.sessions = vec![session_of(vec![flight()])];
    let mut dialog = crate::dialogs::Dialog::export("Test", &flight());
    dialog.reply(Reply::Picked(Some("/destination/out.avi".into())));
    app.dialog = Some(dialog);
    click_text(&ctx, &mut app, "Export", draw);
    let job = jobs.try_recv().unwrap();
    assert!(
        app.dialog.is_none(),
        "export continues in Tasks without its panel"
    );
    assert!(jobs.try_recv().is_err(), "only one export submitted");
    assert!(app.tasks.iter().any(|t| t.id == job.id && t.running));
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Err("Cannot write destination".into()),
        })
        .unwrap();
    shown_texts(&ctx, &mut app, draw);
    let task = app.tasks.iter().find(|t| t.id == job.id).unwrap();
    assert!(!task.running && task.failed);
    assert!(task.line.contains("Cannot write"));
    assert!(app.dialog.is_none());
}

#[test]
fn export_completion_keeps_other_dialogs_and_mismatch_recovery_waits_its_turn() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.dialog_window(ctx);
    };
    app.sessions = vec![session_of(vec![flight()])];
    app.dialog = Some(crate::dialogs::Dialog::export("Test", &flight()));
    click_text(&ctx, &mut app, "Export", draw);
    assert!(app.dialog.is_some(), "empty destination keeps the panel");
    assert!(jobs.try_recv().is_err());
    app.dialog
        .as_mut()
        .unwrap()
        .reply(Reply::Picked(Some("/destination/out.avi".into())));
    click_text(&ctx, &mut app, "Export", draw);
    let job = jobs.try_recv().unwrap();
    app.dialog = Some(crate::dialogs::Dialog::import(None, String::new()));
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(Reply::Mismatch("different rates".into())),
        })
        .unwrap();
    shown_texts(&ctx, &mut app, draw);
    assert!(app.dialog.is_some());
    assert_eq!(
        app.export_dialogs.len(),
        1,
        "recovery is retained behind the import panel"
    );
    app.dialog = None;
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Different frame rates"));
    while jobs.try_recv().is_ok() {} // Discard unrelated strip preparation.
    click_text(&ctx, &mut app, "Export 1 parts separately", draw);
    let job = jobs.try_recv().unwrap();
    assert!(app.dialog.is_none());
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(Reply::Done("Exported out.avi".into())),
        })
        .unwrap();
    shown_texts(&ctx, &mut app, draw);
    assert!(app.export_dialogs.is_empty());
    assert!(
        app.tasks
            .iter()
            .any(|t| t.id == job.id && !t.running && !t.failed)
    );
}

fn info_part() -> fpv_library::RecordingInfo {
    let source = fpv_core::SourceFile::new(
        "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        "/card/Original video.AVI",
        "/library/Imported video.AVI",
        None,
        fpv_core::MediaInfo {
            audio_codec: Some("pcm_s16le".into()),
            ..fpv_core::MediaInfo::new(
                640,
                480,
                fpv_core::FrameRate::new(1_000_000, 40839),
                120.0,
                "mjpeg",
            )
        },
    );
    let clip = fpv_core::Clip::new(source.session_id, source.id, 100, 250);
    fpv_library::RecordingInfo {
        clip,
        source,
        available: false,
        cloud: false,
        size_bytes: None,
    }
}

#[test]
fn thumbnail_info_requests_metadata_and_shows_copyable_source_paths() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    app.sessions = vec![session_of(vec![flight()])];
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.menu_button("…", |ui| app.flight_menu(ui, "test-session", &flight()));
        });
        app.dialog_window(ctx);
    };
    click_text(&ctx, &mut app, "…", draw);
    click_text(&ctx, &mut app, "Info", draw);
    let job = jobs
        .try_recv()
        .expect("metadata is requested on the worker");
    assert!(app.dialog.is_some());
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(Reply::FlightInfo(vec![info_part()])),
        })
        .unwrap();
    let texts = shown_texts(&ctx, &mut app, draw);
    for expected in [
        "Original video.AVI",
        "/card/Original video.AVI",
        "/library/Imported video.AVI",
        "Source unavailable locally",
    ] {
        assert!(
            texts.iter().any(|t| t == expected),
            "missing {expected}: {texts:?}"
        );
    }
    click_text(&ctx, &mut app, "Copy Original path", draw);
    // The click emits a clipboard command rather than opening or changing the file.
    assert!(app.dialog.is_some());
    assert!(
        jobs.try_recv().is_err(),
        "Info actions do not submit writes"
    );
    click_text(&ctx, &mut app, "Close", draw);
    assert!(app.dialog.is_none());
}

#[test]
fn info_displays_each_sources_exact_rate_and_exclusive_bounds_and_fits_small_windows() {
    let ctx = egui::Context::default();
    let mut app = app(&ctx);
    let mut second = info_part();
    second.source.original_path = "/second/card/second.mp4".into();
    second.source.media.frame_rate = fpv_core::FrameRate::new(60000, 1001);
    second.clip.start_frame = 0;
    second.clip.end_frame = 60;
    let mut dialog = crate::dialogs::Dialog::info(&flight());
    dialog.reply(Reply::FlightInfo(vec![info_part(), second]));
    app.dialog = Some(dialog);
    let mut output = ctx.run(input(egui::vec2(1000.0, 3000.0)), |ctx| {
        app.dialog_window(ctx)
    });
    for _ in 0..8 {
        output = ctx.run(input(egui::vec2(1000.0, 3000.0)), |ctx| {
            app.dialog_window(ctx)
        });
    }
    let mut texts = text_positions(&output.shapes);
    for _ in 0..36 {
        let mut raw = input(egui::vec2(1000.0, 3000.0));
        raw.events = vec![
            egui::Event::PointerMoved(egui::pos2(500.0, 1500.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -80.0),
                modifiers: Default::default(),
            },
        ];
        output = ctx.run(raw, |ctx| app.dialog_window(ctx));
        texts.extend(text_positions(&output.shapes));
    }
    for expected in [
        "Recording 1 of 2",
        "Recording 2 of 2",
        "24.486398 fps (1000000/40839)",
        "59.940060 fps (60000/1001)",
        "00:00:04.084 · frame 100",
        "00:00:10.210 · frame 250",
        "00:00:01.001 · frame 60",
        "pcm_s16le",
        "mjpeg · 640 × 480",
    ] {
        assert!(
            texts.iter().any(|(t, _)| t == expected),
            "missing {expected}: {texts:?}"
        );
    }
    for _ in 0..8 {
        output = ctx.run(input(egui::vec2(360.0, 480.0)), |ctx| {
            app.dialog_window(ctx)
        });
    }
    let close = text_positions(&output.shapes)
        .into_iter()
        .find(|(t, _)| t == "Close")
        .unwrap()
        .1;
    assert!(
        close.x >= 0.0 && close.x < 360.0 && close.y >= 0.0 && close.y < 480.0,
        "Close remains visible: {close:?}"
    );
}

#[test]
fn reopening_clips_restores_their_own_playback_mute_and_keeps_no_sound_authoritative() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, events) = app_with_worker(&ctx);
    let a = flight();
    let b = FlightView {
        id: "other-flight".into(),
        ..flight()
    };
    app.sessions = vec![session_of(vec![a.clone(), b.clone()])];
    for (id, saved, has_audio, default, expected) in [
        (&a.id, Some(true), true, false, true),
        (&b.id, Some(false), true, false, false),
        (&a.id, Some(true), true, false, true),
        (&a.id, Some(false), true, true, false),
        (&b.id, None, true, true, true),
        (&a.id, Some(false), false, false, true),
    ] {
        app.config.mute_videos = default;
        events
            .send(crate::worker::Event::Finished {
                id: 999,
                result: Ok(Reply::Playback {
                    flight: id.clone(),
                    at: 0.0,
                    playback_mute: saved,
                    parts: vec![fpv_library::NativeSegment {
                        segment: fpv_library::Segment {
                            path: "not-decoded".into(),
                            start: 0.0,
                            end: 1.0,
                            offset: 0.0,
                        },
                        width: 64,
                        height: 48,
                        has_audio,
                        all_intra: true,
                    }],
                }),
            })
            .unwrap();
        app.receive(&ctx);
        let viewer = app.viewer.as_ref().unwrap();
        assert_eq!(viewer.player.volume() == 0.0, expected);
        assert_eq!(viewer.muted, !has_audio);
        app.viewer = None;
    }
}

#[test]
fn speaker_changes_submit_a_playback_preference_without_waiting_for_close() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _events) = app_with_worker(&ctx);
    app.viewer = Some(test_viewer(&ctx));
    for expected in [true, false] {
        let mut output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
            app.player_window(ctx)
        });
        for _ in 0..8 {
            output = ctx.run(input(egui::vec2(1280.0, 820.0)), |ctx| {
                app.player_window(ctx)
            });
        }
        let (pos, width) = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t)
                    if t.galley.job.text == "Play" || t.galley.job.text == "Pause" =>
                {
                    Some((t.pos, t.galley.size().x))
                }
                _ => None,
            })
            .unwrap();
        let spacing = ctx.style().spacing.clone();
        let point = pos
            + egui::vec2(
                width + spacing.button_padding.x + spacing.item_spacing.x + 13.0,
                8.0,
            );
        for pressed in [true, false] {
            let mut raw = input(egui::vec2(1280.0, 820.0));
            raw.events = vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ];
            let _ = ctx.run(raw, |ctx| app.player_window(ctx));
        }
        assert_eq!(app.viewer.as_ref().unwrap().playback_muted, expected);
        let job = jobs.try_recv().expect("mute preference queued immediately");
        assert!(
            app.tasks
                .iter()
                .any(|t| t.id == job.id && t.label == "Remember playback mute")
        );
        assert!(jobs.try_recv().is_err(), "one write per change");
    }
}

#[test]
fn a_precise_long_gop_cut_offers_explicit_reencoding_instead_of_impossible_lossless_parts() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let draw: fn(&mut App, &egui::Context) = |app, ctx| {
        app.receive(ctx);
        app.dialog_window(ctx);
    };
    app.sessions = vec![session_of(vec![flight()])];
    let mut dialog = crate::dialogs::Dialog::export("Indoor", &flight());
    dialog.reply(Reply::Picked(Some("/export/clip.mp4".into())));
    app.dialog = Some(dialog);
    click_text(&ctx, &mut app, "Export", draw);
    let job = jobs.try_recv().unwrap();
    events
        .send(crate::worker::Event::Finished {
            id: job.id,
            result: Ok(Reply::NeedsReencode(
                "H.264 cut starts between keyframes".into(),
            )),
        })
        .unwrap();
    let texts = shown_texts(&ctx, &mut app, draw);
    assert!(texts.iter().any(|t| t == "Precise cut needs re-encoding"));
    assert!(!texts.iter().any(|t| t.contains("parts separately")));
    assert!(texts.iter().any(|t| t.contains("MPEG-4")));
    while jobs.try_recv().is_ok() {}
    click_text(&ctx, &mut app, "Re-encode into one file", draw);
    assert!(jobs.try_recv().is_ok());
    assert!(app.dialog.is_none(), "export continues in Tasks");
}

#[test]
fn dragging_player_corner_grows_video_and_restore_preserves_manual_size() {
    for source in [[1600, 900], [400, 300], [450, 800]] {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let mut viewer = test_viewer(&ctx);
        let texture = ctx.load_texture(
            "manual-resize",
            egui::ColorImage::filled(source, Color32::GRAY),
            Default::default(),
        );
        let id = texture.id();
        viewer.texture = Some(texture);
        app.viewer = Some(viewer);
        let size = egui::vec2(1800.0, 1200.0);
        let mut render = |events: Vec<egui::Event>| {
            let mut raw = input(size);
            raw.events = events;
            ctx.run(raw, |ctx| app.player_window(ctx))
        };
        for _ in 0..5 {
            render(vec![]);
        }
        let before = render(vec![]);
        let initial_video = texture_rect(&before.shapes, id).unwrap();
        let initial = ctx
            .memory(|m| m.area_rect(egui::Id::new("player")))
            .unwrap();
        let corner = initial.max - egui::vec2(2.0, 2.0);
        let target = corner + egui::vec2(240.0, 180.0);
        render(vec![egui::Event::PointerMoved(corner)]);
        render(vec![egui::Event::PointerButton {
            pos: corner,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        render(vec![egui::Event::PointerMoved(target)]);
        render(vec![egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        for _ in 0..5 {
            render(vec![]);
        }
        let enlarged = render(vec![]);
        let video = texture_rect(&enlarged.shapes, id).unwrap();
        let window = ctx
            .memory(|m| m.area_rect(egui::Id::new("player")))
            .unwrap();
        assert!(
            window.width() > initial.width() + 150.0 && window.height() > initial.height() + 100.0,
            "actual corner gesture must resize: {initial:?} -> {window:?}"
        );
        assert!(
            video.height() > initial_video.height() + 50.0,
            "manual resize must grow {source:?} video: {initial_video:?} -> {video:?}; window {window:?}"
        );
        assert!(
            (video.width() / video.height() - source[0] as f32 / source[1] as f32).abs() < 0.002
        );
        assert!(window.contains_rect(video));
        for label in ["Maximize", "Restore"] {
            let out = render(vec![]);
            let pos = text_positions(&out.shapes)
                .into_iter()
                .find(|(text, _)| text == label)
                .unwrap()
                .1
                + egui::vec2(5.0, 5.0);
            for pressed in [true, false] {
                render(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            for _ in 0..5 {
                render(vec![]);
            }
        }
        let restored = texture_rect(&render(vec![]).shapes, id).unwrap();
        assert!(
            (restored.size() - video.size()).length() < 3.0,
            "restore must recover manually resized video: {video:?} -> {restored:?}"
        );
    }
}

#[test]
fn selected_flights_offer_bulk_delete_without_ffmpeg() {
    let ctx = egui::Context::default();
    let (mut app, jobs, _) = app_with_worker(&ctx);
    let first = flight();
    app.sessions = vec![session_of(vec![first.clone()])];
    app.browse.view = View::Flights;
    app.selecting = true;
    app.selected.insert(first.id.clone());
    app.ffmpeg_ok = false;
    let texts = shown_texts(&ctx, &mut app, App::browser);
    assert!(texts.iter().any(|s| s == "Delete 1…"), "{texts:?}");
    click_text(&ctx, &mut app, "Delete 1…", App::browser);
    assert!(app.dialog.is_some());
    let _ = jobs.try_recv().expect("deletion preview is queued");
    click_text(&ctx, &mut app, "Close", App::dialog_window);
    assert!(app.dialog.is_none());
    assert!(app.selected.contains(&first.id));
    assert!(jobs.try_recv().is_err(), "cancelling must not delete");
}

#[test]
fn batch_deletion_holds_each_affected_session_until_finished() {
    let ctx = egui::Context::default();
    let (mut app, _jobs, _events) = app_with_worker(&ctx);
    app.submit_in(
        crate::Scope::Sessions(vec!["one".into(), "two".into()]),
        "Delete flights",
        false,
        |_, _| unreachable!(),
    );
    assert!(app.session_busy("one") && app.session_busy("two"));
    assert!(!app.session_busy("other"));
    assert!(app.busy_with(&crate::Scope::Sessions(vec!["two".into(), "three".into()])));
    assert!(!app.busy_with(&crate::Scope::Sessions(vec!["other".into()])));
}

#[test]
fn import_counter_remains_visible_when_tasks_are_collapsed_and_another_job_runs() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    let id = app.submit("Import recordings", false, |_, _| unreachable!());
    let _ = jobs.try_recv();
    app.submit("Export flights", false, |_, _| unreachable!());
    let _ = jobs.try_recv();
    events
        .send(crate::worker::Event::Progress {
            id,
            step: fpv_library::Step {
                done: 2,
                total: 4,
                line: "1/4 videos imported · 1 skipped · 0 failed — Copying movie.mp4…".into(),
            },
        })
        .unwrap();
    app.receive(&ctx);
    assert!(!app.show_tasks);
    let texts = shown_texts(&ctx, &mut app, App::task_bar);
    assert!(
        texts
            .iter()
            .any(|s| s.contains("1/4 videos imported") && s.contains("movie.mp4")),
        "{texts:?}"
    );
    events
        .send(crate::worker::Event::Finished {
            id,
            result: Err("Imported 1, skipped 1, failed 2".into()),
        })
        .unwrap();
    app.receive(&ctx);
    let texts = shown_texts(&ctx, &mut app, App::task_bar);
    assert!(
        texts
            .iter()
            .any(|s| s.contains("Imported 1, skipped 1, failed 2")),
        "{texts:?}"
    );
}

#[test]
fn import_dialog_shows_its_progress_then_keeps_it_after_close() {
    let ctx = egui::Context::default();
    let (mut app, jobs, events) = app_with_worker(&ctx);
    app.dialog = Some(crate::dialogs::Dialog::import(
        Some(PathBuf::from("/card")),
        "Training".into(),
    ));
    click_text(&ctx, &mut app, "Import", App::dialog_window);
    let job = jobs.try_recv().unwrap();
    let texts = shown_texts(&ctx, &mut app, App::dialog_window);
    assert!(texts.iter().any(|s| s == "Scanning videos…"));
    events
        .send(crate::worker::Event::Progress {
            id: job.id,
            step: fpv_library::Step {
                done: 1,
                total: 3,
                line: "1/3 videos imported · 0 skipped · 0 failed — Copying next.mp4…".into(),
            },
        })
        .unwrap();
    app.receive(&ctx);
    let texts = shown_texts(&ctx, &mut app, App::dialog_window);
    assert!(
        texts
            .iter()
            .any(|s| s.contains("1/3 videos imported") && s.contains("next.mp4")),
        "{texts:?}"
    );
    assert!(
        !texts
            .iter()
            .any(|s| s.contains("progress is shown in Tasks"))
    );
    click_text(&ctx, &mut app, "Close", App::dialog_window);
    assert!(app.dialog.is_none());
    assert!(app.tasks.iter().any(|t| t.id == job.id && t.running));
    let texts = shown_texts(&ctx, &mut app, App::task_bar);
    assert!(texts.iter().any(|s| s.contains("1/3 videos imported")));
}
