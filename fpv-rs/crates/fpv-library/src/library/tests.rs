//! Through the real ffmpeg, on tiny generated takes: this layer is glue, and
//! glue is only proven by running the pieces it glues.

use std::process::Command;

use super::*;

#[test]
fn native_playback_exposes_dvr_parts_without_resampling_or_proxy_generation() {
    use fpv_core::{FrameRate, Clip, Flight, DetectionRun};
    let root = tempfile::tempdir().unwrap();
    let library = open(root.path());
    let session = Session::new("Native DVR", None);
    library.store.save_session(&session).unwrap();
    let mut offset = 0.0;
    let flight = Flight::new(session.id, 1);
    for (index, rate, start, end) in [(1, FrameRate::new(1_000_000, 33459), 4194, 4217),
                                    (2, FrameRate::new(1_000_000, 40839), 0, 1475)] {
        let mut media = MediaInfo::new(640, 480, rate, 180.0, "mjpeg");
        media.audio_codec = Some("pcm_s16le".into());
        let file = SourceFile::new(session.id, "card", format!("/not-decoded/PICT{index:04}.AVI"), Some(index), media);
        library.store.save_source_files(std::slice::from_ref(&file)).unwrap();
        let clip = Clip::new(flight.id, file.id, start, end);
        library.store.replace_clips(file.id, &[clip], if index == 1 { std::slice::from_ref(&flight) } else { &[] },
            &DetectionRun::new(file.id, "test", Default::default())).unwrap();
        let parts = library.native_playback(&uuid_string(flight.id)).unwrap();
        let part = parts.last().unwrap();
        assert!((part.segment.start - rate.seconds(start)).abs() < 1e-9);
        assert!((part.segment.end - rate.seconds(end)).abs() < 1e-9);
        assert!((part.segment.offset - offset).abs() < 1e-9);
        assert!(part.has_audio);
        assert_eq!((part.width, part.height), (640, 480));
        offset += rate.seconds(end - start);
    }
    assert!(!root.path().join("cache/proxies").exists());
}

#[test]
fn refuses_to_overwrite_an_export_and_checks_all_parts_before_writing() {
    let (fixture, session) = imported();
    let destination = fixture._root.path().join("keep.mp4");
    std::fs::write(&destination, b"keep this file").unwrap();
    assert!(fixture.library.export(&session.flights[0].id, &destination, ExportMode::Lossless).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep this file");
    let part = destination.with_file_name("keep - part2.mp4");
    std::fs::write(&part, b"keep part two").unwrap();
    assert!(fixture.library.export(&session.flights[0].id, &destination, ExportMode::Parts).is_err());
    assert!(!destination.with_file_name("keep - part1.mp4").exists());
    assert_eq!(std::fs::read(part).unwrap(), b"keep part two");
}

/// A library with its configuration and media folders side by side, as the
/// app lays them out under the user's config and videos directories.
fn open(root: &Path) -> Library {
    let config = Config { media_root: root.join("library"), ..Config::default() };
    Library::open_with(&root.join("config"), &root.join("cache"), config, None).unwrap()
}

struct Fixture {
    _root: tempfile::TempDir,
    card: PathBuf,
    library: Library,
}

/// A GoPro card: take 42 in two chapters (2 s + 1 s), take 43 (1 s), at
/// 30 fps so frame arithmetic is easy to check by eye. Each names its camera
/// as a HERO11 does, by its firmware code.
fn gopro_card() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let card = root.path().join("card/DCIM/100GOPRO");
    std::fs::create_dir_all(&card).unwrap();
    for (name, seconds, taken) in [("GH010042.MP4", 2, "2026-08-23T10:00:00Z"),
                                   ("GH020042.MP4", 1, "2026-08-23T10:00:00Z"),
                                   ("GH010043.MP4", 1, "2026-08-23T10:05:00Z")] {
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i",
                   &format!("testsrc2=size=320x240:rate=30:duration={seconds}"),
                   "-f", "lavfi", "-i", &format!("sine=duration={seconds}"),
                   "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest",
                   "-metadata", &format!("creation_time={taken}"),
                   "-metadata", "firmware=H22.01.01.10.00", "-movflags", "use_metadata_tags",
                   "-y"])
            .arg(card.join(name))
            .status().expect("ffmpeg on PATH");
        assert!(status.success());
    }
    let library = open(root.path());
    Fixture { card: root.path().join("card"), library, _root: root }
}

fn imported() -> (Fixture, SessionView) {
    let fixture = gopro_card();
    let summary = fixture.library.import(&fixture.card, "Letz Kwad", Some("2026-08-23"), false,
                                         |_| {}).unwrap();
    assert_eq!((summary.imported, summary.flights, summary.failed), (3, 2, 0));
    let session = fixture.library.snapshot().unwrap().remove(0);
    (fixture, session)
}

#[test]
fn shows_action_cam_takes_as_flights_ready_to_play() {
    let (_fixture, session) = imported();
    assert!(!session.segmentable);
    assert_eq!(session.flights.len(), 2);
    let take = &session.flights[0];
    assert!(take.action_cam);
    assert_eq!(take.recordings, 2);
    assert!((take.duration - 3.0).abs() < 0.05, "{}", take.duration);
    assert!(take.clips.iter().all(|c| c.thumbnail.as_deref().is_some_and(|p| Path::new(p).exists())));
    assert!(session.files.iter().all(|f| f.action_cam && f.codec == "h264"));
}

#[test]
fn shows_what_recorded_each_flight_and_file() {
    let (_fixture, session) = imported();
    assert_eq!(session.flights[0].source, "GoPro HERO11");
    assert!(session.files.iter().all(|f| f.source == "GoPro HERO11"));
    // Both chapters are 320x240 H.264 at 30 fps: said once.
    assert_eq!(session.flights[0].format, "240p · H.264 · 30 fps");
    assert_eq!(session.files[0].format, "240p · H.264 · 30 fps");
}

#[test]
fn reads_the_camera_of_takes_imported_before_it_was_kept_once() {
    let (fixture, session) = imported();
    let library = &fixture.library;
    // As a build before the camera column left them.
    for file in &session.files {
        let mut stored = library.store.source_file(parse(&file.id).unwrap()).unwrap().unwrap();
        stored.media.camera = None;
        library.store.save_source_files(&[stored]).unwrap();
    }
    assert_eq!(library.snapshot().unwrap()[0].flights[0].source, "GoPro");

    assert_eq!(library.identify_cameras().unwrap(), 3);
    assert_eq!(library.snapshot().unwrap()[0].flights[0].source, "GoPro HERO11");
    assert_eq!(library.identify_cameras().unwrap(), 0, "read once, not at every open");
}

#[test]
fn tags_a_take_and_reads_it_back_on_the_flight_clock() {
    let (fixture, session) = imported();
    let take = &session.flights[0].id;
    // 2.5 s is in the second chapter, half a second in.
    let tag = fixture.library.add_tag(take, " split-s ", 2.5, 2.8).unwrap();
    assert_eq!(tag.label, "split-s");
    assert!((tag.start - 2.5).abs() < 0.02 && (tag.end - 2.8).abs() < 0.02);
    fixture.library.add_tag(take, "flip", 0.5, 0.5).unwrap();
    let tags = fixture.library.flight_tags(take).unwrap();
    assert_eq!(tags.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["flip", "split-s"]);
    assert!(tags[0].marker);
    // Not in the other take's sidebar.
    assert!(fixture.library.flight_tags(&session.flights[1].id).unwrap().is_empty());
    assert_eq!(fixture.library.tag_labels().unwrap().len(), 2);
    fixture.library.delete_tag(&tags[0].id).unwrap();
    assert_eq!(fixture.library.flight_tags(take).unwrap().len(), 1);
    assert!(fixture.library.add_tag(take, "  ", 1.0, 1.0).is_err());
}

#[test]
fn shows_each_flights_figures_on_its_card_in_time_order() {
    // The Figures view and a card's "N figures" read the snapshot, not one
    // flight at a time.
    let (fixture, session) = imported();
    let take = &session.flights[0].id;
    fixture.library.add_tag(take, "split-s", 2.5, 2.8).unwrap();
    fixture.library.add_tag(take, "flip", 0.5, 0.5).unwrap();
    let flights = &fixture.library.snapshot().unwrap()[0].flights;
    let labels: Vec<_> = flights[0].figures.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["flip", "split-s"]);
    assert!((flights[0].figures[1].start - 2.5).abs() < 0.02);
    assert!(flights[1].figures.is_empty());
}

#[test]
fn labels_a_whole_flight_and_suggests_labels_already_used() {
    let (fixture, session) = imported();
    let (take, other) = (&session.flights[0].id, &session.flights[1].id);
    let library = &fixture.library;
    library.add_flight_label(take, " freestyle ").unwrap();
    library.add_flight_label(take, "gates").unwrap();
    library.add_flight_label(other, "Gates").unwrap();
    let flights = &library.snapshot().unwrap()[0].flights;
    assert_eq!(flights[0].labels, ["freestyle", "gates"]);
    assert_eq!(flights[1].labels, ["Gates"]);
    // "gates" twice, whatever the case: first.
    assert_eq!(library.flight_label_names().unwrap(), ["gates", "freestyle"]);

    library.remove_flight_label(take, "freestyle").unwrap();
    assert_eq!(library.snapshot().unwrap()[0].flights[0].labels, ["gates"]);
    assert!(library.add_flight_label(take, "   ").is_err());
}

#[test]
fn edits_titles_likes_names_and_boundaries() {
    let (fixture, session) = imported();
    let take = &session.flights[1];
    fixture.library.set_flight_title(&take.id, "  Bando  ").unwrap();
    assert!(fixture.library.toggle_like(&take.id).unwrap());
    fixture.library.rename_session(&session.id, "Letz Kwad — GoPro").unwrap();
    // The end is exclusive, so the final source frame must remain selectable.
    fixture.library.update_clip(&take.clips[0].id, 6, 10_000).unwrap();

    let session = fixture.library.snapshot().unwrap().remove(0);
    let take = &session.flights[1];
    assert_eq!(session.name, "Letz Kwad — GoPro");
    assert_eq!(take.title.as_deref(), Some("Bando"));
    assert!(take.liked);
    assert_eq!((take.clips[0].start_frame, take.clips[0].end_frame), (6, take.clips[0].last_frame + 1));
    // A boundary edit re-keys the flight, so its strips and proxy go stale.
    assert!(!take.cache_key.ends_with("-0-30"));
    fixture.library.set_flight_title(&take.id, "").unwrap();
    assert_eq!(fixture.library.snapshot().unwrap()[0].flights[1].title, None);
}

#[test]
fn exports_plays_and_scrubs_a_take() {
    let (fixture, session) = imported();
    let take = &session.flights[0];
    let out = fixture.library.root.join("out/take.mp4");
    let written = fixture.library.export(&take.id, &out, ExportMode::Lossless).unwrap();
    assert_eq!(written, std::slice::from_ref(&out));
    let parts = fixture.library.export(&take.id, &out, ExportMode::Parts).unwrap();
    assert_eq!(parts.iter().map(|p| file_name(&p.to_string_lossy())).collect::<Vec<_>>(),
               ["take - part1.mp4", "take - part2.mp4"]);

    let proxy = fixture.library.proxy(&take.id).unwrap();
    assert!(proxy.exists());
    assert_eq!(fixture.library.proxy(&take.id).unwrap(), proxy); // a cache hit

    let frame = fixture.library.filmstrip_frame(&take.id, 23, 24).unwrap();
    assert!(frame.made && frame.path.exists() && frame.path.to_string_lossy().contains(&take.cache_key));
    // Asked again, it comes from the cache: the task bar records no work.
    assert!(!fixture.library.filmstrip_frame(&take.id, 23, 24).unwrap().made);
    let preview = fixture.library.preview_frame(&take.clips[0].file_id, 15).unwrap();
    assert!(preview.made && preview.path.exists());
    assert!(!fixture.library.preview_frame(&take.clips[0].file_id, 15).unwrap().made);
}

#[test]
fn sweeps_stale_strips_and_previews_at_open() {
    let (fixture, session) = imported();
    let take = &session.flights[0];
    let kept = fixture.library.filmstrip_frame(&take.id, 0, 24).unwrap().path;
    let preview = fixture.library.preview_frame(&take.clips[0].file_id, 3).unwrap().path;
    let stale = kept.with_file_name("00000000-0000-0000-0000-000000000000-0-30-0.jpg");
    std::fs::write(&stale, b"x").unwrap();

    let library = open(fixture.library.root.parent().unwrap());
    assert!(kept.exists() && !stale.exists() && !preview.exists());
    assert_eq!(library.snapshot().unwrap()[0].flights.len(), 2);
}

#[test]
fn deletes_a_session_and_its_media_folder() {
    let (fixture, session) = imported();
    let folder = fixture.library.root.join("2026-08-23 Letz Kwad GOPRO");
    assert!(folder.is_dir());
    fixture.library.delete_session(&session.id, false).unwrap();
    assert!(fixture.library.snapshot().unwrap().is_empty());
    assert!(!folder.exists());
    // The card was never touched.
    assert!(fixture.card.join("DCIM/100GOPRO/GH010042.MP4").exists());
}

/// An H.264 take of `seconds` named `name`, written into `folder`; takes of
/// different lengths differ by content, as two real takes do.
fn take_in(folder: &Path, name: &str, seconds: u32) {
    std::fs::create_dir_all(folder).unwrap();
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", &format!("testsrc2=size=320x240:rate=30:duration={seconds}"),
               "-c:v", "libx264", "-pix_fmt", "yuv420p", "-metadata", "creation_time=2026-08-23T11:00:00Z", "-y"])
        .arg(folder.join(name)).status().expect("ffmpeg on PATH");
    assert!(status.success());
}

/// Every folder of the import folder, as a viewer ticking them all would.
fn everything(library: &Library) -> Vec<String> {
    library.import_folder_contents().unwrap().into_iter().map(|f| f.name).collect()
}

#[test]
fn lists_the_import_folders_and_what_each_holds_that_is_new() {
    let (fixture, session) = imported();
    let library = &fixture.library;
    take_in(&library.root.join("2026-09-01 Bench"), "DJI_0002.MP4", 2);
    let contents = library.import_folder_contents().unwrap();
    assert_eq!(contents, [
        FolderView { name: "2026-08-23 Letz Kwad GOPRO".into(), session: "Letz Kwad".into(),
                     date: Some("2026-08-23".into()), recordings: 3, new: 0, deleted: vec![] },
        FolderView { name: "2026-09-01 Bench".into(), session: "Bench".into(),
                     date: Some("2026-09-01".into()), recordings: 1, new: 1, deleted: vec![] },
    ]);
    // Removed with its videos kept: its folder is new again.
    library.delete_session(&session.id, true).unwrap();
    assert_eq!(library.import_folder_contents().unwrap()[0].new, 3);
}

#[test]
fn loads_only_the_folders_chosen_and_none_outside_the_import_folder() {
    let (fixture, _) = imported();
    let library = &fixture.library;
    take_in(&library.root.join("2026-09-01 Bench"), "DJI_0002.MP4", 2);
    take_in(&library.root.join("2026-09-02 Roof"), "DJI_0003.MP4", 3);
    let summary = library.load_import_folders(&["2026-09-02 Roof".into()], &[], |_| {}).unwrap();
    assert_eq!(summary.imported, 1);
    let names: Vec<_> = library.snapshot().unwrap().into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"Roof".to_string()) && !names.contains(&"Bench".to_string()), "{names:?}");
    for outside in ["..", "../library", "/tmp", "2026-09-03 Nowhere"] {
        assert!(matches!(library.load_import_folders(&[outside.into()], &[], |_| {}), Err(LibraryError::NotFound(_))),
                "{outside}");
    }
}

#[test]
fn loads_recordings_left_in_the_import_folder_where_they_are() {
    // A session removed with its videos kept: loading the import folder brings
    // it back, read from its folder's name, without copying anything.
    let (fixture, session) = imported();
    let library = &fixture.library;
    let take = library.root.join("2026-08-23 Letz Kwad GOPRO/GH010042.MP4");
    let written = std::fs::metadata(&take).unwrap().modified().unwrap();
    library.delete_session(&session.id, true).unwrap();
    let mut copied = 0;
    let summary = library.load_import_folders(&everything(library), &[], |event| {
        if matches!(event, ImportEvent::Copying(_)) { copied += 1; }
    }).unwrap();
    assert_eq!((summary.imported, summary.skipped, summary.failed, summary.flights), (3, 0, 0, 2));
    assert_eq!(copied, 0);
    assert_eq!(std::fs::metadata(&take).unwrap().modified().unwrap(), written);
    let sessions = library.snapshot().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "Letz Kwad");
    assert!(sessions[0].date.as_deref().is_some_and(|d| d.starts_with("2026-08-2")), "{:?}", sessions[0].date);
    assert_eq!(sessions[0].flights.len(), 2);
    let paths: Vec<_> = sessions[0].files.iter()
        .map(|f| library.recording_path(&f.id).unwrap()).collect();
    assert!(paths.contains(&take), "registered where it is: {paths:?}");

    // Nothing new: nothing happens.
    let summary = library.load_import_folders(&everything(library), &[], |_| {}).unwrap();
    assert_eq!((summary.imported, summary.skipped), (0, 3));
    assert_eq!(library.snapshot().unwrap().len(), 1);
}

#[test]
fn leaves_a_recording_deleted_with_its_flight_unless_asked_to_load_it_again() {
    // The trash refused the take, or the pilot put it back: it is in the
    // import folder again, and loading the folder must not undo the deletion.
    let (fixture, session) = imported();
    let library = &fixture.library;
    let take = &session.flights[0];
    let chapters: Vec<_> = take.clips.iter().map(|c| library.recording_path(&c.file_id).unwrap()).collect();
    let kept: Vec<_> = chapters.iter().map(|path| std::fs::read(path).unwrap()).collect();
    library.delete_flight(&take.id).unwrap();
    for (path, bytes) in chapters.iter().zip(&kept) {
        std::fs::write(path, bytes).unwrap();
    }
    let folder = library.import_folder_contents().unwrap().remove(0);
    assert_eq!((folder.new, folder.deleted.as_slice()), (0, ["GH010042.MP4".to_owned(), "GH020042.MP4".to_owned()].as_slice()));

    let summary = library.load_import_folders(&everything(library), &[], |_| {}).unwrap();
    assert_eq!((summary.imported, summary.skipped), (0, 3));
    assert_eq!(library.snapshot().unwrap()[0].flights.len(), 1);

    // Asked for: back into the session beside the other take, and forgotten.
    let summary = library.load_import_folders(&[], std::slice::from_ref(&folder.name), |_| {}).unwrap();
    assert_eq!((summary.imported, summary.flights), (2, 1));
    let sessions = library.snapshot().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!((sessions[0].id.as_str(), sessions[0].flights.len()), (session.id.as_str(), 2));
    // Forgotten as deleted: a session later removed with its videos kept
    // loads back whole.
    library.delete_session(&session.id, true).unwrap();
    let folder = library.import_folder_contents().unwrap().remove(0);
    assert_eq!((folder.new, folder.deleted.len()), (3, 0));
}

#[test]
fn files_a_loaded_folder_into_its_session_and_day_or_a_new_one() {
    let (fixture, _) = imported();
    let library = &fixture.library;
    // The same session and day from another camera: joined.
    take_in(&library.root.join("2026-08-23 Letz Kwad DJI"), "DJI_0001.MP4", 1);
    // A folder dropped in by hand: a session of its own, dated from its name.
    take_in(&library.root.join("2026-09-01 Bench"), "DJI_0002.MP4", 2);
    let summary = library.load_import_folders(&everything(library), &[], |_| {}).unwrap();
    assert_eq!((summary.imported, summary.skipped, summary.flights), (2, 3, 2));

    let sessions = library.snapshot().unwrap();
    let names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    let letz = sessions.iter().find(|s| s.name == "Letz Kwad").unwrap();
    assert_eq!((letz.files.len(), letz.flights.len()), (4, 3));
    let bench = sessions.iter().find(|s| s.name == "Bench").unwrap();
    assert!(bench.date.as_deref().is_some_and(|d| d.starts_with("2026-08-31") || d.starts_with("2026-09-01")));
    assert_eq!(bench.flights[0].source, "DJI");
}

#[test]
fn removes_a_session_but_keeps_its_recordings_for_a_later_import() {
    let (fixture, session) = imported();
    let take = fixture.library.root.join("2026-08-23 Letz Kwad GOPRO/GH010042.MP4");
    let written = std::fs::metadata(&take).unwrap().modified().unwrap();
    fixture.library.delete_session(&session.id, true).unwrap();
    assert!(fixture.library.snapshot().unwrap().is_empty());
    assert!(take.is_file());

    // The same card again: the copies on disk are taken back — not skipped as
    // "already in the library", which would leave no session, and not copied twice.
    let mut copied = 0;
    let summary = fixture.library.import(&fixture.card, "Letz Kwad", Some("2026-08-23"), false, |event| {
        if matches!(event, ImportEvent::Copying(_)) { copied += 1; }
    }).unwrap();
    assert_eq!((summary.imported, summary.skipped, summary.failed, summary.flights), (3, 0, 0, 2));
    assert_eq!(copied, 0);
    assert_eq!(std::fs::metadata(&take).unwrap().modified().unwrap(), written);
    assert_eq!(fixture.library.snapshot().unwrap()[0].flights.len(), 2);

    // A third time, they are the library's again: skipped.
    let summary = fixture.library.import(&fixture.card, "Letz Kwad", Some("2026-08-23"), false, |_| {}).unwrap();
    assert_eq!((summary.imported, summary.skipped), (0, 3));
}

#[test]
fn never_overwrites_a_different_file_left_in_the_import_folder() {
    let (fixture, session) = imported();
    let take = fixture.library.root.join("2026-08-23 Letz Kwad GOPRO/GH010042.MP4");
    fixture.library.delete_session(&session.id, true).unwrap();
    std::fs::write(&take, b"something else").unwrap();
    let summary = fixture.library.import(&fixture.card, "Letz Kwad", Some("2026-08-23"), false, |_| {}).unwrap();
    assert_eq!(summary.failed, 1);
    assert_eq!(std::fs::read(&take).unwrap(), b"something else");
}

#[test]
fn frees_the_card_only_for_verified_copies() {
    let fixture = gopro_card();
    let summary = fixture.library.import(&fixture.card, "Letz Kwad", None, true, |_| {}).unwrap();
    assert_eq!(summary.deleted_from_card, 3);
    assert!(summary.kept.is_empty());
    assert!(!fixture.card.join("DCIM/100GOPRO/GH010042.MP4").exists());
    // The session took the first take's time, read as local.
    assert!(fixture.library.snapshot().unwrap()[0].date.is_some());
}

#[test]
fn writes_no_label_rows_for_takes_that_were_never_detected() {
    let (fixture, session) = imported();
    let labels = fixture.library.root.join("labels.csv");
    assert_eq!(fixture.library.export_labels(&session.id, &labels).unwrap(), 0);
    assert!(std::fs::read_to_string(labels).unwrap().contains("# reviewed: no"));
}

#[test]
fn refuses_bad_input_politely() {
    let (fixture, session) = imported();
    assert!(matches!(fixture.library.rename_session(&session.id, " "), Err(LibraryError::Invalid(_))));
    assert!(matches!(fixture.library.toggle_like("not-an-id"), Err(LibraryError::Invalid(_))));
    assert!(matches!(fixture.library.toggle_like(&uuid_string(Uuid::new_v4())),
                     Err(LibraryError::NotFound(_))));
    assert!(fixture.library.import(&fixture.card, "x", Some("23/08/2026"), false, |_| {}).is_err());
}

#[test]
fn keeps_the_database_in_the_config_folder_and_the_footage_in_the_media_folder() {
    let (fixture, _) = imported();
    let base = fixture.library.root.parent().unwrap();
    assert!(base.join("config/library.sqlite").is_file());
    assert!(!base.join("library/library.sqlite").exists());
    assert!(base.join("library/2026-08-23 Letz Kwad GOPRO/GH010042.MP4").is_file());
}

#[test]
fn moves_a_database_left_in_the_media_folder_by_an_earlier_version() {
    let (fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    drop(fixture.library);
    // Where the Swift app, and the first Rust builds, kept it.
    std::fs::rename(base.join("config/library.sqlite"), base.join("library/library.sqlite")).unwrap();

    let library = open(&base);
    assert!(base.join("config/library.sqlite").is_file());
    assert!(!base.join("library/library.sqlite").exists());
    assert_eq!(library.snapshot().unwrap()[0].id, session.id);
}

#[test]
fn opens_with_the_config_file_it_writes_on_first_run() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    Config { media_root: root.path().join("media"), proxy_budget_gb: 2.0, ..Config::default() }
        .save(&config).unwrap();
    let library = Library::open(&config, &root.path().join("cache"), None).unwrap();
    assert_eq!(library.root, root.path().join("media"));
    // A first run: neither folder existed, and both do now.
    assert!(library.root.is_dir());
    assert_eq!(library.config_directory, config);
}

#[test]
fn moves_future_imports_to_a_chosen_folder_and_keeps_the_old_footage_working() {
    let (mut fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    let old_take = fixture.library.root.join("2026-08-23 Letz Kwad GOPRO/GH010042.MP4");
    let take = &session.flights[0];
    let proxy = fixture.library.proxy(&take.id).unwrap();
    fixture.library.filmstrip_frame(&take.id, 0, 24).unwrap();

    let chosen = base.join("other disk/FPV");
    fixture.library.set_media_root(&chosen).unwrap();

    // Saved, so the next launch imports there too.
    assert_eq!(Config::load_or_create(&base.join("config")).unwrap().media_root, chosen);
    assert_eq!(fixture.library.root, chosen);
    // The caches are not the import folder's business: they stay put.
    assert!(proxy.starts_with(base.join("cache/proxies")) && proxy.exists());
    assert!(old_take.exists());
    assert!(session.flights[0].clips[0].thumbnail.as_deref().is_some_and(|p| Path::new(p).exists()));
    // Pictures all come from the cache, whatever the import folder.
    assert_eq!(fixture.library.image_directories().unwrap(), [base.join("cache")]);

    // Old flights still play, from the same cached proxy.
    assert_eq!(fixture.library.proxy(&take.id).unwrap(), proxy);

    // And a new card lands in the chosen folder.
    std::fs::remove_dir_all(fixture.card.join("DCIM/100GOPRO")).unwrap();
    std::fs::create_dir_all(fixture.card.join("DCIM/100GOPRO")).unwrap();
    // Different bytes from the take already imported, or the checksum skips it.
    std::fs::write(fixture.card.join("DCIM/100GOPRO/GH010099.MP4"),
                   [std::fs::read(&old_take).unwrap(), b"x".to_vec()].concat()).unwrap();
    fixture.library.import(&fixture.card, "Second outing", None, false, |_| {}).unwrap();
    assert!(chosen.join("2026-08-23 Second outing GOPRO/GH010099.MP4").exists());
}

#[test]
fn refuses_a_relative_or_unwritable_import_folder() {
    let (mut fixture, _) = imported();
    let before = fixture.library.root.clone();
    assert!(matches!(fixture.library.set_media_root(Path::new("videos")), Err(LibraryError::Invalid(_))));
    assert!(fixture.library.set_media_root(Path::new("/proc/fpv-rush")).is_err());
    assert_eq!(fixture.library.root, before);
}

#[test]
fn keeps_proxies_strips_and_previews_in_the_cache_folder() {
    let (fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    let take = &session.flights[0];
    assert!(fixture.library.proxy(&take.id).unwrap().starts_with(base.join("cache/proxies")));
    assert!(fixture.library.filmstrip_frame(&take.id, 0, 24).unwrap().path.starts_with(base.join("cache/strips")));
    assert!(fixture.library.preview_frame(&take.clips[0].file_id, 3).unwrap().path
        .starts_with(base.join("cache/preview")));
    assert!(!base.join("library/proxies").exists());
}

#[test]
fn drops_the_proxies_an_earlier_version_kept_beside_the_footage() {
    let (fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    let take = &session.flights[0].id;
    // Build one, then put it where the earlier layout had it.
    let proxy = fixture.library.proxy(take).unwrap();
    let legacy = base.join("library/proxies");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::rename(&proxy, legacy.join(proxy.file_name().unwrap())).unwrap();
    std::fs::create_dir_all(base.join("library/thumbnails/strips")).unwrap();
    drop(fixture.library);

    let library = open(&base);
    assert!(!legacy.exists() && !base.join("library/thumbnails/strips").exists());
    assert!(library.store.proxies().unwrap().is_empty());
    assert!(library.proxy(take).unwrap().starts_with(base.join("cache/proxies")));
}

#[test]
fn keeps_posters_in_the_cache_and_remakes_one_the_cache_lost() {
    let (fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    let clip = &session.flights[0].clips[0];
    let poster = PathBuf::from(clip.thumbnail.as_deref().unwrap());
    assert!(poster.starts_with(base.join("cache/posters")));
    assert!(!base.join("library/thumbnails").exists());

    // The user empties ~/.cache.
    std::fs::remove_dir_all(base.join("cache")).unwrap();
    assert_eq!(fixture.library.snapshot().unwrap()[0].flights[0].clips[0].thumbnail, None);
    let remade = fixture.library.poster(&clip.id).unwrap();
    assert!(remade.made);
    let remade = remade.path;
    assert!(remade.is_file() && remade.starts_with(base.join("cache/posters")));
    assert!(!fixture.library.poster(&clip.id).unwrap().made);
    assert_eq!(fixture.library.snapshot().unwrap()[0].flights[0].clips[0].thumbnail.as_deref(),
               Some(remade.to_string_lossy().as_ref()));
}

#[test]
fn moves_posters_an_earlier_version_kept_beside_the_footage_into_the_cache() {
    let (fixture, session) = imported();
    let base = fixture.library.root.parent().unwrap().to_path_buf();
    // Put every poster where the earlier layout had it, and point the clips there.
    let legacy = base.join("library/thumbnails");
    std::fs::create_dir_all(&legacy).unwrap();
    for flight in &session.flights {
        for view in &flight.clips {
            let old = PathBuf::from(view.thumbnail.as_deref().unwrap());
            let moved = legacy.join(old.file_name().unwrap());
            std::fs::rename(&old, &moved).unwrap();
            let clip = fixture.library.store.clip(Uuid::parse_str(&view.id).unwrap()).unwrap().unwrap();
            fixture.library.store.update_clip(&fpv_core::Clip {
                thumbnail_path: Some(moved.to_string_lossy().into_owned()), ..clip
            }).unwrap();
        }
    }
    drop(fixture.library);

    let library = open(&base);
    assert!(!legacy.exists(), "the emptied thumbnails folder goes");
    for flight in &library.snapshot().unwrap()[0].flights {
        for clip in &flight.clips {
            let poster = PathBuf::from(clip.thumbnail.as_deref().unwrap());
            assert!(poster.is_file() && poster.starts_with(base.join("cache/posters")));
        }
    }
}

#[test]
fn names_folders_from_the_configured_pattern_and_saves_a_new_one() {
    let fixture = gopro_card();
    let mut library = fixture.library;
    // The default pattern: date, session, source.
    library.import(&fixture.card, "Alpes", Some("2026-10-10"), false, |_| {}).unwrap();
    assert!(library.root.join("2026-10-10 Alpes GOPRO/GH010042.MP4").is_file());

    assert!(matches!(library.set_folder_pattern("{date} {place}"), Err(LibraryError::Invalid(_))));
    library.set_folder_pattern("{source} - {session}").unwrap();
    let saved = Config::load_or_create(&library.config_directory).unwrap();
    assert_eq!(saved.folder_pattern, "{source} - {session}");
    assert_eq!(Library::folder_preview("{date} {session} {source}", "Alpes").unwrap(),
               "2026-10-10 Alpes FATSHARK");
}

/// A take in two GoPro chapters recorded at different rates — the batch's
/// mismatch case, which a lossless one-file export refuses.
fn add_mismatched_take(card: &Path) {
    for (name, rate, taken) in [("GH010050.MP4", 30, "2026-08-23T11:00:00Z"),
                                ("GH020050.MP4", 25, "2026-08-23T11:00:00Z")] {
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i",
                   &format!("testsrc2=size=320x240:rate={rate}:duration=1"),
                   "-c:v", "libx264", "-pix_fmt", "yuv420p",
                   "-metadata", &format!("creation_time={taken}"),
                   "-metadata", "firmware=H22.01.01.10.00", "-movflags", "use_metadata_tags",
                   "-y"])
            .arg(card.join("DCIM/100GOPRO").join(name))
            .status().unwrap();
        assert!(status.success());
    }
}

#[test]
fn exports_a_batch_of_flights_into_one_folder_without_overwriting() {
    let fixture = gopro_card();
    add_mismatched_take(&fixture.card);
    fixture.library.import(&fixture.card, "Alpes", Some("2026-08-23"), false, |_| {}).unwrap();
    let session = fixture.library.snapshot().unwrap().remove(0);
    let ids: Vec<String> = session.flights.iter().map(|f| f.id.clone()).collect();
    assert_eq!(ids.len(), 3);
    fixture.library.set_flight_title(&ids[1], "Bando").unwrap();

    let out = fixture.library.root.parent().unwrap().join("export");
    let mut seen = vec![];
    let batch = fixture.library.export_flights(&ids, &out, ExportMode::Parts,
                                               |done, total, name| seen.push((done, total, name.to_owned())))
        .unwrap();
    assert!(batch.failed.is_empty(), "{:?}", batch.failed);
    let names = |b: &BulkExport| b.written.iter()
        .map(|p| file_name(&p.to_string_lossy())).collect::<Vec<_>>();
    // Titled or not, and the mismatched take split into its lossless parts.
    assert_eq!(names(&batch), ["Alpes — Flight 1.mp4", "Alpes — Bando.mp4",
                               "Alpes — Flight 3 - part1.mp4", "Alpes — Flight 3 - part2.mp4"]);
    assert_eq!(seen.first().map(|s| (s.0, s.1)), Some((0, 3)));
    assert_eq!(seen.last().map(|s| (s.0, s.1)), Some((3, 3)));

    // The same batch again: nothing is overwritten.
    let again = fixture.library.export_flights(&ids[..1], &out, ExportMode::Parts, |_, _, _| {}).unwrap();
    assert_eq!(names(&again), ["Alpes — Flight 1 (2).mp4"]);
}

#[test]
fn keeps_an_action_cam_take_lossless_in_a_re_encoding_batch_and_carries_on_past_failures() {
    let fixture = gopro_card();
    add_mismatched_take(&fixture.card);
    fixture.library.import(&fixture.card, "Alpes", None, false, |_| {}).unwrap();
    let session = fixture.library.snapshot().unwrap().remove(0);
    let mismatched = session.flights.iter()
        .find(|f| f.clips.len() == 2 && f.clips[0].fps != f.clips[1].fps)
        .unwrap().id.clone();
    let out = fixture.library.root.parent().unwrap().join("export");
    let batch = fixture.library.export_flights(
        &["not-a-flight".to_owned(), mismatched], &out, ExportMode::Reencode, |_, _, _| {}).unwrap();
    // The unknown flight is reported; the batch goes on.
    assert_eq!(batch.failed, [("not-a-flight".to_owned(), "not in the library".to_owned())]);
    // Re-encoding is for MJPEG: the GoPro take gets its two lossless parts.
    let names: Vec<_> = batch.written.iter().map(|p| file_name(&p.to_string_lossy())).collect();
    assert_eq!(names.len(), 2);
    assert!(names[0].ends_with(" - part1.mp4") && names[1].ends_with(" - part2.mp4"), "{names:?}");
}

#[test]
fn keeps_a_sessions_detection_profile() {
    let (fixture, session) = imported();
    assert_eq!(session.detection_profile, "standard");
    fixture.library.set_detection_profile(&session.id, "textured").unwrap();
    assert_eq!(fixture.library.snapshot().unwrap()[0].detection_profile, "textured");
    assert!(matches!(fixture.library.set_detection_profile(&session.id, "fuzzy"),
                     Err(LibraryError::Invalid(_))));
}

#[test]
fn opens_each_folder_of_a_session_once() {
    // The day's GoPro card, then a DJI card under the same session: two
    // folders by source, each shown by one of its recordings.
    let (fixture, session) = imported();
    let dji = fixture._root.path().join("dji");
    std::fs::create_dir_all(&dji).unwrap();
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=1",
               "-c:v", "libx264", "-pix_fmt", "yuv420p", "-metadata", "creation_time=2026-08-23T10:10:00Z", "-y"])
        .arg(dji.join("DJI_0001.MP4")).status().expect("ffmpeg on PATH");
    assert!(status.success());
    fixture.library.import(&dji, "Letz Kwad", Some("2026-08-23"), false, |_| {}).unwrap();

    let shown = fixture.library.session_folders(&session.id).unwrap();
    assert_eq!(shown.len(), 2, "{shown:?}");
    let folders: Vec<_> = shown.iter().map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert!(folders.iter().any(|f| f.ends_with("GOPRO")) && folders.iter().any(|f| f.ends_with("DJI")), "{folders:?}");
    assert!(shown.iter().all(|p| p.is_file()));
}

#[test]
fn finds_where_a_recording_was_copied_to() {
    let (fixture, session) = imported();
    let file = &session.files[0];
    let path = fixture.library.recording_path(&file.id).unwrap();
    assert!(path.is_absolute() && path.is_file());
    assert_eq!(path.file_name().unwrap().to_str(), Some(file.name.as_str()));
    assert!(matches!(fixture.library.recording_path(&uuid::Uuid::new_v4().to_string()),
                     Err(LibraryError::NotFound(_))));
}

#[test]
fn makes_hover_strips_in_batches_other_jobs_can_pass_between() {
    let (fixture, _) = imported();
    let batch = fixture.library.prepare_strips_up_to(8, None, 10, |_, _, _| {}).unwrap();
    assert_eq!((batch.made, batch.more), (10, true));
    assert_eq!(fixture.library.missing_strip_frames(8, None).unwrap().len(), 6);
    let batch = fixture.library.prepare_strips_up_to(8, None, 10, |_, _, _| {}).unwrap();
    assert_eq!((batch.made, batch.more), (6, false));
    let batch = fixture.library.prepare_strips_up_to(8, None, 10, |_, _, _| {}).unwrap();
    assert_eq!((batch.made, batch.more), (0, false));
}

#[test]
fn makes_every_hover_strip_ahead_of_the_pointer_once() {
    let (fixture, session) = imported();
    // Two takes, nothing made yet.
    assert_eq!(fixture.library.missing_strip_frames(8, None).unwrap().len(), 16);

    let mut steps = vec![];
    let prepared = fixture.library.prepare_strips(8, None, |done, total, label| {
        steps.push((done, total, label.to_owned()));
    }).unwrap();
    assert_eq!((prepared.made, prepared.failed.len()), (16, 0));
    assert_eq!(steps.first(), Some(&(0, 16, "Letz Kwad — Flight 1".to_owned())));
    assert_eq!(steps.last(), Some(&(16, 16, String::new())));
    assert!(fixture.library.missing_strip_frames(8, None).unwrap().is_empty());
    // The pointer now finds them on disk.
    assert!(!fixture.library.filmstrip_frame(&session.flights[0].id, 3, 8).unwrap().made);

    // Nothing left to make: a second run costs nothing.
    assert_eq!(fixture.library.prepare_strips(8, None, |_, _, _| {}).unwrap().made, 0);

    // A boundary edit re-keys one flight: only its strip is missing again.
    fixture.library.update_clip(&session.flights[1].clips[0].id, 6, 10_000).unwrap();
    let missing = fixture.library.missing_strip_frames(8, None).unwrap();
    assert_eq!(missing.len(), 8);
    assert!(missing.iter().all(|frame| frame.flight == session.flights[1].id));
}

/// Stands in for Proton Drive's Online Only, which Linux has no File
/// Provider for: a `.cloud` file beside a recording says it is in the cloud.
fn cloud_marker(path: &Path) -> bool {
    path.with_extension("cloud").exists()
}

fn send_to_cloud(library: &Library, session: &SessionView, name: &str) {
    let file = session.files.iter().find(|f| f.name == name).unwrap();
    let path = library.recording_path(&file.id).unwrap();
    std::fs::write(path.with_extension("cloud"), b"").unwrap();
}

#[test]
fn shows_what_is_only_in_the_cloud_and_never_downloads_it_for_a_picture() {
    let (mut fixture, session) = imported();
    fixture.library.cloud.only_in_cloud = cloud_marker;
    send_to_cloud(&fixture.library, &session, "GH010043.MP4");
    let library = &fixture.library;

    let shown = library.snapshot().unwrap().remove(0);
    let cloud: Vec<_> = shown.files.iter().filter(|f| f.cloud).map(|f| f.name.as_str()).collect();
    assert_eq!(cloud, ["GH010043.MP4"]);
    assert_eq!(shown.flights.iter().map(|f| f.cloud).collect::<Vec<_>>(), [false, true]);

    // The strips job makes the downloaded take's strip and leaves the other.
    assert_eq!(library.missing_strip_frames(8, None).unwrap().len(), 8);
    let prepared = library.prepare_strips(8, None, |_, _, _| {}).unwrap();
    assert_eq!((prepared.made, prepared.failed.len()), (8, 0));
    assert!(matches!(library.filmstrip_frame(&shown.flights[1].id, 0, 8),
                     Err(LibraryError::InCloud(name)) if name == "GH010043.MP4"));

    // A poster already made still shows; a lost one is not remade from the cloud.
    let clip = &shown.flights[1].clips[0];
    assert!(library.poster(&clip.id).is_ok());
    std::fs::remove_file(clip.thumbnail.as_deref().unwrap()).unwrap();
    assert!(matches!(library.poster(&clip.id), Err(LibraryError::InCloud(_))));

    // The camera is read from the downloaded takes only, the other one later.
    for file in &shown.files {
        let mut stored = library.store.source_file(parse(&file.id).unwrap()).unwrap().unwrap();
        stored.media.camera = None;
        library.store.save_source_files(&[stored]).unwrap();
    }
    assert_eq!(library.identify_cameras().unwrap(), 2);
    let path = library.recording_path(&shown.files.iter().find(|f| f.cloud).unwrap().id).unwrap();
    std::fs::remove_file(path.with_extension("cloud")).unwrap();
    assert_eq!(library.identify_cameras().unwrap(), 1);
    assert!(!library.snapshot().unwrap()[0].flights[1].cloud);
    assert_eq!(library.missing_strip_frames(8, None).unwrap().len(), 8, "its strip, now");
}

#[test]
fn downloads_what_is_only_in_the_cloud_by_reading_it_through() {
    let (mut fixture, session) = imported();
    // Nothing in the cloud: nothing to do.
    let summary = fixture.library.download_session(&session.id, |_| {}).unwrap();
    assert_eq!((summary.downloaded, summary.bytes, summary.failed.len()), (0, 0, 0));

    // In the cloud until read, as a File Provider placeholder is.
    fn until_read(path: &Path) -> bool {
        static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        cloud_marker(path) && ASKED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
    }
    fixture.library.cloud.only_in_cloud = until_read;
    send_to_cloud(&fixture.library, &session, "GH010043.MP4");
    let take = fixture.library.recording_path(&session.files.iter().find(|f| f.name == "GH010043.MP4")
                                              .unwrap().id).unwrap();
    let size = std::fs::metadata(&take).unwrap().len() as usize;
    let mut steps = vec![];
    let summary = fixture.library.download_flight(&session.flights[1].id, |step| {
        steps.push((step.done, step.total, step.line, (step.file, step.files)));
    }).unwrap();
    assert_eq!((summary.downloaded, summary.bytes as usize, summary.failed.len()), (1, size, 0));
    assert_eq!(steps.first().map(|s| (s.0, s.1, s.2.as_str(), s.3)),
               Some((0, size, "Downloading GH010043.MP4 (1/1)…", (1, 1))));
    assert_eq!(steps.last().map(|s| (s.0, s.1, s.3)), Some((size, size, (1, 1))));

    // Through to the end, a chunk at a time, however large the recording.
    fixture.library.cloud.only_in_cloud = cloud_marker;
    let file = fixture.library.store.source_file(parse(&session.files.iter()
        .find(|f| f.name == "GH010043.MP4").unwrap().id).unwrap()).unwrap().unwrap();
    let mut reads = 0;
    let summary = fixture.library.download(vec![file], 4096, |_| reads += 1).unwrap();
    assert_eq!(summary.failed.len(), 1, "the marker stays: not brought down");
    assert!(reads > size / 4096, "{reads} progress steps for {size} bytes");
}

#[test]
fn says_which_recording_a_download_is_on_when_the_bytes_cannot() {
    // A File Provider may hold each read until the whole file is down: the
    // window counts recordings then, so every step names the one it is on.
    let (mut fixture, session) = imported();
    fixture.library.cloud.only_in_cloud = cloud_marker;
    send_to_cloud(&fixture.library, &session, "GH010042.MP4");
    send_to_cloud(&fixture.library, &session, "GH020042.MP4");
    let mut parts: Vec<(String, (usize, usize))> = vec![];
    fixture.library.download_flight(&session.flights[0].id, |step| {
        if parts.last().is_none_or(|(line, _)| *line != step.line) {
            parts.push((step.line, (step.file, step.files)));
        }
    }).unwrap();
    assert_eq!(parts, [("Downloading GH010042.MP4 (1/2)…".to_owned(), (1, 2)),
                       ("Downloading GH020042.MP4 (2/2)…".to_owned(), (2, 2)),
                       ("0 recording(s) downloaded".to_owned(), (2, 2))]);
}

#[test]
fn says_when_the_cloud_drive_did_not_bring_a_recording_down() {
    let (mut fixture, session) = imported();
    fixture.library.cloud.only_in_cloud = cloud_marker;
    send_to_cloud(&fixture.library, &session, "GH020042.MP4");
    let summary = fixture.library.download_session(&session.id, |_| {}).unwrap();
    assert_eq!(summary.downloaded, 0);
    assert_eq!(summary.failed, [("GH020042.MP4".to_owned(), "the cloud drive did not bring it down".to_owned())]);
}

#[test]
fn frees_a_cloud_drives_space_and_says_what_it_kept() {
    let (mut fixture, session) = imported();
    fixture.library.cloud.only_in_cloud = cloud_marker;
    // Outside a cloud drive's folder there is nothing to free.
    let shown = fixture.library.snapshot().unwrap().remove(0);
    assert!(shown.files.iter().all(|f| !f.freeable) && shown.flights.iter().all(|f| !f.freeable));
    assert_eq!(fixture.library.free_up_session(&session.id, |_, _, _| {}).unwrap().freed, 0);

    // Inside one, with a drive that frees as Online Only does.
    fixture.library.cloud.on_drive = |_| true;
    fixture.library.cloud.free_up_space = |path| std::fs::write(path.with_extension("cloud"), b"");
    let shown = fixture.library.snapshot().unwrap().remove(0);
    assert!(shown.files.iter().all(|f| f.freeable) && shown.flights.iter().all(|f| f.freeable));
    let take = fixture.library.recording_path(&shown.files.iter().find(|f| f.name == "GH010043.MP4")
                                              .unwrap().id).unwrap();
    let size = std::fs::metadata(&take).unwrap().len();
    let mut lines = vec![];
    let summary = fixture.library.free_up_flight(&shown.flights[1].id, |_, _, line| lines.push(line)).unwrap();
    assert_eq!((summary.freed, summary.bytes, summary.failed.len()), (1, size, 0));
    assert_eq!(lines.first().map(String::as_str), Some("Freeing GH010043.MP4 (1/1)…"));
    let shown = fixture.library.snapshot().unwrap().remove(0);
    assert_eq!(shown.flights.iter().map(|f| (f.cloud, f.freeable)).collect::<Vec<_>>(),
               [(false, true), (true, false)]);

    // A drive that refuses says why; one that says yes and keeps the file is
    // not believed. The take already in the cloud is not asked again.
    fixture.library.cloud.free_up_space = |_| Err(std::io::Error::other("not uploaded yet"));
    let summary = fixture.library.free_up_session(&session.id, |_, _, _| {}).unwrap();
    assert_eq!(summary.freed, 0);
    assert_eq!(summary.failed, [("GH010042.MP4".to_owned(), "not uploaded yet".to_owned()),
                                ("GH020042.MP4".to_owned(), "not uploaded yet".to_owned())]);
    fixture.library.cloud.free_up_space = |_| Ok(());
    let summary = fixture.library.free_up_session(&session.id, |_, _, _| {}).unwrap();
    assert_eq!(summary.freed, 0);
    assert_eq!(summary.failed.iter().map(|(_, why)| why.as_str()).collect::<Vec<_>>(),
               ["the cloud drive kept it on this disk"; 2]);
    assert!(take.is_file() && fixture.library.recording_path(&shown.files[0].id).unwrap().is_file());
}

#[test]
fn makes_the_session_on_screen_first() {
    let (fixture, _) = imported();
    // A second card with footage of its own — the same bytes would be
    // recognised as duplicates and skipped.
    let other = tempfile::tempdir().unwrap();
    for (name, pattern) in [("GH010050.MP4", "testsrc"), ("GH010051.MP4", "smptebars")] {
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", &format!("{pattern}=size=320x240:rate=30:duration=1"),
                   "-c:v", "libx264", "-pix_fmt", "yuv420p",
                   "-metadata", "creation_time=2026-08-24T10:00:00Z", "-y"])
            .arg(other.path().join(name))
            .status().unwrap();
        assert!(status.success());
    }
    let summary = fixture.library.import(other.path(), "Bando", Some("2026-08-24"), false, |_| {}).unwrap();
    assert_eq!(summary.flights, 2);
    // Whichever session comes last on its own is the one asked for first.
    let natural = fixture.library.missing_strip_frames(2, None).unwrap();
    let last = natural.last().unwrap().label.split(" — ").next().unwrap().to_owned();
    assert!(!natural[0].label.starts_with(&format!("{last} — ")));
    let wanted = fixture.library.snapshot().unwrap().into_iter().find(|s| s.name == last).unwrap();
    let missing = fixture.library.missing_strip_frames(2, Some(&wanted.id)).unwrap();
    assert_eq!(missing.len(), 8);
    assert!(missing[..4].iter().all(|frame| frame.label.starts_with(&format!("{last} — "))));
}

#[test]
fn reports_an_unreadable_recording_once_and_makes_the_other_strips() {
    let (fixture, session) = imported();
    let broken = fixture.library.recording_path(&session.flights[1].clips[0].file_id).unwrap();
    std::fs::write(&broken, b"not a video any more").unwrap();
    let prepared = fixture.library.prepare_strips(8, None, |_, _, _| {}).unwrap();
    assert_eq!(prepared.made, 8);
    assert_eq!(prepared.failed.len(), 1);
    assert_eq!(prepared.failed[0].0, "Letz Kwad — Flight 2");
}

#[test]
fn takes_rough_strip_frames_only_on_long_gop_footage_spread_out_enough() {
    use fpv_core::FrameRate;
    let osmo = MediaInfo::new(3840, 2160, FrameRate::new(60_000, 1001), 441.0, "hevc");
    let dvr = MediaInfo::new(640, 480, FrameRate::new(1_000_000, 40_061), 372.7, "mjpeg");
    // A 7:21 Osmo take, 24 frames 18 s apart: the nearest keyframe will do.
    assert!(rough_strip(&osmo, 441.0 / 24.0));
    // A 3 s chapter: frames 0.125 s apart would land on the same keyframe.
    assert!(!rough_strip(&osmo, 3.0 / 24.0));
    // The DVR is all keyframes: always exact, at no extra cost.
    assert!(!rough_strip(&dvr, 372.7 / 24.0));
}

#[test]
fn measures_and_empties_the_cache_and_everything_comes_back() {
    let (fixture, session) = imported();
    let take = &session.flights[0];
    fixture.library.proxy(&take.id).unwrap();
    fixture.library.prepare_strips(4, None, |_, _, _| {}).unwrap();
    fixture.library.preview_frame(&take.clips[0].file_id, 3).unwrap();

    let usage = fixture.library.cache_usage().unwrap();
    assert!(usage.proxies > 0 && usage.strips > 0 && usage.posters > 0 && usage.previews > 0, "{usage:?}");
    assert_eq!(usage.total(), usage.proxies + usage.strips + usage.posters + usage.previews);

    let freed = fixture.library.clear_cache(&CacheKind::ALL).unwrap();
    assert_eq!(freed.total(), usage.total());
    let after = fixture.library.cache_usage().unwrap();
    assert_eq!(after.total(), 0, "{after:?}");
    // The proxy index forgot them too: the next play makes a new one.
    assert!(fixture.library.proxy_cache().entries().unwrap().is_empty());

    // Nothing is lost: each comes back on demand.
    assert!(fixture.library.snapshot().unwrap()[0].flights[0].clips[0].thumbnail.is_none());
    assert!(fixture.library.poster(&take.clips[0].id).unwrap().made);
    assert!(fixture.library.proxy(&take.id).unwrap().is_file());
    assert_eq!(fixture.library.missing_strip_frames(4, None).unwrap().len(), 8);
}

#[test]
fn empties_one_kind_of_cache_and_leaves_the_others() {
    let (fixture, session) = imported();
    let take = &session.flights[0];
    fixture.library.proxy(&take.id).unwrap();
    fixture.library.prepare_strips(4, None, |_, _, _| {}).unwrap();
    let before = fixture.library.cache_usage().unwrap();

    let freed = fixture.library.clear_cache(&[CacheKind::Strips]).unwrap();
    assert_eq!((freed.strips, freed.proxies, freed.posters), (before.strips, 0, 0));
    let after = fixture.library.cache_usage().unwrap();
    assert_eq!((after.strips, after.proxies, after.posters), (0, before.proxies, before.posters));

    fixture.library.clear_cache(&[CacheKind::Proxies]).unwrap();
    assert!(fixture.library.proxy_cache().entries().unwrap().is_empty());
    assert_eq!(fixture.library.cache_usage().unwrap().posters, before.posters);
}

/// Dates the pictures of one cache folder an hour back, a second apart in
/// name order, so the trimming order is known; returns the oldest.
fn age_pictures(library: &Library, folder: &str) -> PathBuf {
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut frames: Vec<PathBuf> = std::fs::read_dir(library.cache_directory.join(folder)).unwrap()
        .flatten().map(|e| e.path()).collect();
    frames.sort();
    for (age, frame) in frames.iter().enumerate() {
        std::fs::File::options().write(true).open(frame).unwrap()
            .set_modified(hour_ago + std::time::Duration::from_secs(age as u64)).unwrap();
    }
    frames.remove(0)
}

fn gigabytes(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

#[test]
fn keeps_the_pictures_under_their_budget_strips_before_posters_and_the_last_used_longest() {
    let (mut fixture, session) = imported();
    fixture.library.prepare_strips(8, None, |_, _, _| {}).unwrap();
    let usage = fixture.library.cache_usage().unwrap();
    assert!(usage.strips > 0 && usage.posters > 0);
    let oldest = age_pictures(&fixture.library, "strips");
    // The pointer comes back over the oldest frame: it is now the one kept longest.
    let (flight, index) = session.flights.iter().flat_map(|f| (0..8).map(move |i| (f, i)))
        .find(|(f, i)| fixture.library.filmstrip_frame(&f.id, *i, 8).unwrap().path == oldest).unwrap();
    let used = fixture.library.filmstrip_frame(&flight.id, index, 8).unwrap().path;

    // Room for the posters and about half the strips.
    fixture.library.config.picture_budget_gb = gigabytes(usage.posters + usage.strips / 2);
    let freed = fixture.library.trim_pictures().unwrap();
    let after = fixture.library.cache_usage().unwrap();
    assert!(freed > 0 && after.strips + after.posters <= usage.posters + usage.strips / 2, "{after:?}");
    assert_eq!(after.posters, usage.posters, "posters go last");
    assert!(used.is_file(), "the frame used last stays");
    // The strips job fills what room is left without going over, and the
    // next trim — the next open — finds nothing to do.
    fixture.library.prepare_strips(8, None, |_, _, _| {}).unwrap();
    let filled = fixture.library.cache_usage().unwrap();
    assert!(filled.strips + filled.posters <= usage.posters + usage.strips / 2, "{filled:?}");
    assert_eq!(fixture.library.trim_pictures().unwrap(), 0, "no frame made and trimmed at every launch");
    // Less room than a frame: nothing to make, so no job at the next launch.
    fixture.library.config.picture_budget_gb = gigabytes(filled.strips + filled.posters + 100);
    assert!(fixture.library.missing_strip_frames(8, None).unwrap().is_empty());

    // Smaller than the posters alone: every strip goes, then the oldest
    // posters — not the oldest one a card just showed again.
    let oldest = age_pictures(&fixture.library, "posters");
    let shown = session.flights.iter().flat_map(|f| &f.clips)
        .find(|c| Path::new(c.thumbnail.as_deref().unwrap()) == oldest).unwrap();
    fixture.library.poster(&shown.id).unwrap();
    fixture.library.config.picture_budget_gb = gigabytes(usage.posters / 2);
    fixture.library.trim_pictures().unwrap();
    let after = fixture.library.cache_usage().unwrap();
    assert_eq!(after.strips, 0);
    assert!(after.posters > 0 && after.posters <= usage.posters / 2, "{after:?}");
    assert!(oldest.is_file(), "the poster shown last stays");
    // A poster trimmed away is made again when its card shows.
    let lost = session.flights.iter().flat_map(|f| &f.clips)
        .find(|c| !Path::new(c.thumbnail.as_deref().unwrap()).is_file()).unwrap();
    assert!(fixture.library.poster(&lost.id).unwrap().made);
}

#[test]
fn stops_making_strips_at_the_pictures_budget() {
    let (mut fixture, _) = imported();
    let posters = fixture.library.cache_usage().unwrap().posters;
    // Room for the posters and a few strip frames (~6 KB each here), not all sixteen.
    fixture.library.config.picture_budget_gb = gigabytes(posters + 20_000);
    let prepared = fixture.library.prepare_strips(8, None, |_, _, _| {}).unwrap();
    assert!(prepared.at_budget);
    assert!(prepared.made >= 1 && prepared.made < 16, "{}", prepared.made);
    assert!(fixture.library.cache_usage().unwrap().total() <= posters + 20_000, "never a frame over");
}

#[test]
fn saves_both_cache_budgets_and_refuses_one_too_small() {
    let (mut fixture, _) = imported();
    fixture.library.set_cache_budgets(4.0, 0.5).unwrap();
    let saved = Config::load_or_create(&fixture.library.config_directory).unwrap();
    assert_eq!((saved.proxy_budget_gb, saved.picture_budget_gb), (4.0, 0.5));
    assert!(matches!(fixture.library.set_cache_budgets(0.0, 1.0), Err(LibraryError::Invalid(_))));
    assert!(matches!(fixture.library.set_cache_budgets(1.0, f64::NAN), Err(LibraryError::Invalid(_))));
    assert_eq!(fixture.library.config().proxy_budget_gb, 4.0, "a refused budget changes nothing");
}

#[test]
fn empties_only_the_folders_it_made() {
    let (fixture, _) = imported();
    let mine = fixture.library.cache_directory.join("notes.txt");
    std::fs::write(&mine, b"keep me").unwrap();
    fixture.library.clear_cache(&CacheKind::ALL).unwrap();
    assert!(mine.is_file());
}

#[test]
fn plays_an_action_cam_take_from_its_own_files_chapter_by_chapter() {
    let (fixture, session) = imported();
    // Take 42: two chapters, 2 s then 1 s, at 30 fps.
    let take = &session.flights[0];
    assert_eq!(take.recordings, 2);
    let Playback::Direct { segments } = fixture.library.playback(&take.id).unwrap() else {
        panic!("an action-cam take plays from its own files")
    };
    let spans: Vec<_> = segments.iter().map(|s| (s.start, s.end, s.offset)).collect();
    assert_eq!(spans, [(0.0, 2.0, 0.0), (0.0, 1.0, 2.0)]);
    assert!(segments[0].path.ends_with("GH010042.MP4") && segments[1].path.ends_with("GH020042.MP4"));
    // No proxy was made for it.
    assert!(fixture.library.proxy_cache().entries().unwrap().is_empty());

    // Trimmed in the editor, it plays between its new bounds.
    fixture.library.update_clip(&take.clips[0].id, 15, 45).unwrap();
    let Playback::Direct { segments } = fixture.library.playback(&take.id).unwrap() else { panic!() };
    assert_eq!((segments[0].start, segments[0].end, segments[1].offset), (0.5, 1.5, 1.0));
}

#[test]
fn plays_the_dvr_through_a_proxy_and_action_cams_directly() {
    use fpv_core::FrameRate;
    let dvr = MediaInfo::new(640, 480, FrameRate::new(1_000_000, 33_459), 141.0, "mjpeg");
    let gopro = MediaInfo::new(3840, 2160, FrameRate::new(60_000, 1001), 30.0, "h264");
    let osmo = MediaInfo::new(3840, 2160, FrameRate::new(60_000, 1001), 441.0, "hevc");
    assert!(!plays_directly([&dvr]));
    assert!(plays_directly([&gopro, &gopro]));
    assert!(plays_directly([&osmo]));
    // Never seen, but a flight mixing the two would need the proxy's join.
    assert!(!plays_directly([&gopro, &dvr]));
}

/// A DVR card: one 40 s MJPEG recording at 25 fps, imported, never segmented.
fn dvr_card() -> (Fixture, SessionView) {
    let root = tempfile::tempdir().unwrap();
    let card = root.path().join("card");
    std::fs::create_dir_all(&card).unwrap();
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=40",
               "-f", "lavfi", "-i", "sine=duration=40", "-c:v", "mjpeg", "-q:v", "5",
               "-c:a", "pcm_s16le", "-shortest", "-y"])
        .arg(card.join("PICT0032.AVI")).status().expect("ffmpeg on PATH");
    assert!(status.success());
    let library = open(root.path());
    library.import(&card, "Letz Kwad", Some("2026-08-23"), false, |_| {}).unwrap();
    let session = library.snapshot().unwrap().remove(0);
    (Fixture { card, library, _root: root }, session)
}

/// Flights as a segmentation would leave them, at 25 fps: A 10-20 s and B
/// 21-24 s, close enough for their margins to merge; C 35-36 s on its own.
fn flights_in(fixture: &Fixture, session: &SessionView) -> Uuid {
    let file = parse(&session.files[0].id).unwrap();
    let flights = [(250, 500), (525, 600), (875, 900)].map(|_| Flight::new(parse(&session.id).unwrap(), 0));
    let flights: Vec<Flight> = flights.into_iter().enumerate()
        .map(|(i, f)| Flight { order_in_session: i as i64 + 1, ..f }).collect();
    let clips: Vec<Clip> = [(250, 500), (525, 600), (875, 900)].iter().zip(&flights)
        .map(|(&(start, end), flight)| Clip::new(flight.id, file, start, end)).collect();
    fixture.library.store.replace_clips(file, &clips, &flights,
        &fpv_core::DetectionRun::new(file, "test-1", Default::default())).unwrap();
    file
}

#[test]
fn reduces_a_dvr_recording_to_its_flights_and_keeps_everything_on_them() {
    let (fixture, session) = dvr_card();
    let library = &fixture.library;
    let recording = library.recording_path(&session.files[0].id).unwrap();

    // Never segmented: nothing says where its flights are, so nothing is cut.
    assert_eq!(library.reduce_preview(&session.id).unwrap(),
               ReducePreview { unsegmented: 1, ..Default::default() });
    assert_eq!(library.reduce_session(&session.id, |_, _, _| {}).unwrap().recordings, 0);
    assert!(recording.is_file());
    assert!(!session.reducible);

    let mut policy = library.store.source_file(parse(&session.files[0].id).unwrap()).unwrap().unwrap();
    policy.muted = true;
    library.store.save_source_files(&[policy]).unwrap();
    flights_in(&fixture, &session);
    let before = library.snapshot().unwrap().remove(0);
    assert!(before.reducible);
    let (a, c) = (&before.flights[0].id, &before.flights[2].id);
    library.set_flight_title(a, "Rock gap").unwrap();
    library.add_flight_label(c, "gates").unwrap();
    library.add_tag(a, "split-S", 2.0, 2.4).unwrap();
    // A figure tag in the snow between the two pieces (725-750), beyond both margins.
    let snow = FigureTag::new(parse(&before.files[0].id).unwrap(), 730, 740, "nothing");
    library.store.save_tag(&snow).unwrap();

    let preview = library.reduce_preview(&session.id).unwrap();
    assert_eq!((preview.recordings, preview.emptied, preview.unsegmented, preview.figures_lost), (1, 0, 0, 1));
    // 850 frames kept of 1000.
    assert_eq!(preview.bytes_after, (preview.bytes_now as f64 * 0.85) as u64, "{preview:?}");

    let summary = library.reduce_session(&session.id, |_, _, _| {}).unwrap();
    assert_eq!((summary.recordings, summary.pieces, summary.failed.len()), (1, 2, 0));
    assert!(!recording.exists(), "the recording went to the trash");

    let after = library.snapshot().unwrap().remove(0);
    assert!(library.store.source_files(parse(&session.id).unwrap()).unwrap().iter().all(|f| f.muted && f.media.audio_codec.is_some()));
    let names: Vec<_> = after.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["PICT0032-1.AVI", "PICT0032-2.AVI"]);
    // 5 s margins at 25 fps: frames 125-725 (A and B merged) and 750-1000.
    let frames = |name: &str| {
        let out = Command::new("ffprobe").args(["-v", "error", "-count_frames", "-select_streams", "v",
            "-show_entries", "stream=nb_read_frames,r_frame_rate", "-of", "csv=p=0"])
            .arg(recording.with_file_name(name)).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert_eq!(frames("PICT0032-1.AVI"), "25/1,600");
    assert_eq!(frames("PICT0032-2.AVI"), "25/1,250");

    // The same flights, as long as before, with what the pilot put on them.
    assert_eq!(after.flights.iter().map(|f| f.duration).collect::<Vec<_>>(),
               before.flights.iter().map(|f| f.duration).collect::<Vec<_>>());
    assert_eq!(after.flights[0].title.as_deref(), Some("Rock gap"));
    assert_eq!(after.flights[2].labels, ["gates"]);
    let figures: Vec<_> = after.flights.iter().flat_map(|f| &f.figures).collect();
    assert_eq!(figures.len(), 1);
    assert!((figures[0].start - 2.0).abs() < 0.05, "{:?}", figures[0]);
    // Cut losslessly, A still exports, frame for frame.
    let out = fixture._root.path().join("a.avi");
    library.export(&after.flights[0].id, &out, ExportMode::Lossless).unwrap();
    let probe = MediaProbe::new(&library.ffmpeg()).probe(&out).unwrap();
    assert_eq!(probe.frame_count, Some(250));
    assert!(probe.audio_codec.is_none(), "reduced pieces keep the import audio policy");

    // Reduced once: never cut again, and segmentation leaves it be.
    assert_eq!(library.reduce_preview(&session.id).unwrap(), ReducePreview::default());
    assert!(!after.segmentable && !after.reducible);
    // And it says so, on its recordings and its flights.
    assert!(!before.files.iter().any(|f| f.reduced) && !before.flights.iter().any(|f| f.reduced));
    assert!(after.files.iter().all(|f| f.reduced) && after.flights.iter().all(|f| f.reduced));
}

#[test]
fn deletes_a_dvr_flight_but_keeps_its_whole_recording() {
    let (fixture, session) = dvr_card();
    let library = &fixture.library;
    flights_in(&fixture, &session);
    let before = library.snapshot().unwrap().remove(0);
    let recording = library.recording_path(&before.files[0].id).unwrap();
    let expected = FlightDeletion { recordings: vec!["PICT0032.AVI".into()], ..Default::default() };
    assert_eq!(library.flight_deletion(&before.flights[0].id).unwrap(), expected);
    assert_eq!(library.delete_flight(&before.flights[0].id).unwrap(), expected);

    let after = library.snapshot().unwrap().remove(0);
    assert_eq!(after.flights.len(), 2);
    assert_eq!(after.files.len(), 1);
    assert!(recording.is_file(), "only the pointer went");
}

#[test]
fn deletes_a_reduced_flight_with_its_own_piece_and_keeps_a_shared_one() {
    let (fixture, session) = dvr_card();
    let library = &fixture.library;
    flights_in(&fixture, &session);
    library.reduce_session(&session.id, |_, _, _| {}).unwrap();
    let reduced = library.snapshot().unwrap().remove(0);
    let folder = library.recording_path(&reduced.files[0].id).unwrap().parent().unwrap().to_path_buf();
    let (a, b, c) = (&reduced.flights[0], &reduced.flights[1], &reduced.flights[2]);

    // C alone in PICT0032-2: the piece goes with it.
    let deleted = library.delete_flight(&c.id).unwrap();
    assert_eq!(deleted, FlightDeletion { trashed: vec!["PICT0032-2.AVI".into()], ..Default::default() });
    assert!(!folder.join("PICT0032-2.AVI").exists());
    // A shares PICT0032-1 with B: the piece stays for B.
    let deleted = library.delete_flight(&a.id).unwrap();
    assert_eq!(deleted, FlightDeletion { shared: vec!["PICT0032-1.AVI".into()], ..Default::default() });
    assert!(folder.join("PICT0032-1.AVI").is_file());

    let after = library.snapshot().unwrap().remove(0);
    assert_eq!(after.flights.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), [b.id.as_str()]);
    assert_eq!(after.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["PICT0032-1.AVI"]);
    // B, now alone in its piece, would take it along.
    assert_eq!(library.flight_deletion(&b.id).unwrap().trashed, ["PICT0032-1.AVI"]);
}

#[test]
fn deletes_an_action_cam_take_with_its_chapters() {
    let (fixture, session) = imported();
    let library = &fixture.library;
    let take = &session.flights[0];
    let chapter = library.recording_path(&take.clips[1].file_id).unwrap();
    assert_eq!(library.delete_flight(&take.id).unwrap(),
               FlightDeletion { trashed: vec!["GH010042.MP4".into(), "GH020042.MP4".into()], ..Default::default() });
    assert!(!chapter.exists());
    let after = library.snapshot().unwrap().remove(0);
    assert_eq!(after.flights.len(), 1);
    assert_eq!(after.files.len(), 1, "the other take stays");
}

#[test]
fn edits_a_sessions_details_and_tags_from_one_dialog() {
    let (fixture, session) = imported();
    let library = &fixture.library;
    library.update_session(&session.id, &SessionEdit {
        name: "  Alpes ".into(), date: Some("2026-10-17".into()), location: " Col du Lautaret ".into(),
        notes: "".into(), labels: vec!["mountain".into(), " race ".into(), "Mountain".into(), "".into()],
    }).unwrap();
    let edited = library.snapshot().unwrap().remove(0);
    assert_eq!(edited.name, "Alpes");
    assert!(edited.date.as_deref().is_some_and(|d| d.starts_with("2026-10-1")), "{:?}", edited.date);
    assert_eq!(edited.location.as_deref(), Some("Col du Lautaret"));
    assert_eq!(edited.notes, None);
    assert_eq!(edited.labels, ["mountain", "race"]);
    // One vocabulary with the flights' tags, for the suggestions.
    assert_eq!(library.flight_label_names().unwrap(), ["mountain", "race"]);

    let undated = SessionEdit { name: "Alpes".into(), ..Default::default() };
    library.update_session(&session.id, &undated).unwrap();
    let cleared = library.snapshot().unwrap().remove(0);
    assert!(cleared.date.is_none() && cleared.labels.is_empty() && cleared.location.is_none());
    assert!(library.update_session(&session.id, &SessionEdit { name: " ".into(), ..Default::default() }).is_err());
    assert!(library.update_session(&session.id, &SessionEdit { name: "A".into(), date: Some("23/10".into()),
                                                              ..Default::default() }).is_err());
}

#[test]
fn moves_a_sessions_recordings_from_an_earlier_import_folder_into_the_current_one() {
    let (mut fixture, session) = imported();
    let old = fixture.library.root.join("2026-08-23 Letz Kwad GOPRO");
    let take = old.join("GH010042.MP4");
    let written = std::fs::metadata(&take).unwrap().modified().unwrap();
    let new_root = fixture._root.path().join("new import folder");
    std::fs::create_dir_all(&new_root).unwrap();
    fixture.library.set_media_root(&new_root).unwrap();
    let library = &fixture.library;

    // Left behind by the change: said, with where.
    let before = library.snapshot().unwrap().remove(0);
    assert!(before.files.iter().all(|f| f.elsewhere.as_deref() == Some(old.to_string_lossy().as_ref())));
    let preview = library.relocation_preview(&session.id).unwrap();
    let new_folder = new_root.join("2026-08-23 Letz Kwad GOPRO");
    assert_eq!((preview.recordings, preview.from.clone(), preview.to.clone()),
               (3, vec![old.to_string_lossy().into_owned()], vec![new_folder.to_string_lossy().into_owned()]));

    let summary = library.move_session_into_import_folder(&session.id, |_, _, _| {}).unwrap();
    assert_eq!((summary.moved, summary.failed.len()), (3, 0));
    let moved = new_folder.join("GH010042.MP4");
    assert!(moved.is_file() && !take.exists());
    assert!(!old.exists(), "the emptied folder went");
    assert_eq!(std::fs::metadata(&moved).unwrap().modified().unwrap(), written);
    let after = library.snapshot().unwrap().remove(0);
    assert!(after.files.iter().all(|f| f.elsewhere.is_none()));
    assert_eq!(library.recording_path(&after.files[0].id).unwrap().parent().unwrap(), new_folder);
    // The library follows: the take still exports from where it is now.
    let out = fixture._root.path().join("take.mp4");
    library.export(&after.flights[0].id, &out, ExportMode::Lossless).unwrap();
    assert!(out.is_file());
    // Nothing left to move.
    assert_eq!(library.relocation_preview(&session.id).unwrap(), RelocationPreview::default());
}

#[test]
fn no_sound_import_persists_and_silences_only_new_recordings_in_playback_and_exports() {
    let fixture = gopro_card();
    let normal_card = fixture._root.path().join("normal-card");
    std::fs::create_dir_all(&normal_card).unwrap();
    std::fs::copy(fixture.card.join("DCIM/100GOPRO/GH010043.MP4"), normal_card.join("GH010043.MP4")).unwrap();
    fixture.library.import(&normal_card, "Audio choices", Some("2026-08-23"), false, |_| {}).unwrap();
    let report = fixture.library.import_with_audio(&fixture.card, "Audio choices", Some("2026-08-23"), false, true, |_| {}).unwrap();
    assert_eq!((report.imported, report.skipped), (2, 1));
    assert!(!fixture.library.config.mute_videos);
    let session = fixture.library.store.sessions().unwrap().remove(0);
    let files = fixture.library.store.source_files(session.id).unwrap();
    for file in &files {
        assert_eq!(file.muted, !file.imported_path.ends_with("GH010043.MP4"));
        assert!(file.media.audio_codec.is_some(), "retain the source audio metadata");
        assert_eq!(std::fs::read(&file.original_path).unwrap(), std::fs::read(&file.imported_path).unwrap());
    }
    let library = open(fixture._root.path());
    let session = library.snapshot().unwrap().remove(0);
    let ffmpeg = library.ffmpeg();
    let probe = MediaProbe::new(&ffmpeg);
    for (index, flight) in session.flights.iter().enumerate() {
        let muted = flight.recordings == 2;
        assert!(library.native_playback(&flight.id).unwrap().iter().all(|p| p.has_audio != muted));
        for mode in [ExportMode::Lossless, ExportMode::Parts] {
            let destination = fixture._root.path().join(format!("flight-{index}-{mode:?}.mp4"));
            for path in library.export(&flight.id, &destination, mode).unwrap() {
                assert_eq!(probe.probe(&path).unwrap().audio_codec.is_none(), muted);
            }
        }
    }
    let muted_flight = session.flights.iter().find(|f| f.recordings == 2).unwrap();
    let single = session.flights.iter().find(|f| f.recordings == 1).unwrap();
    let target = fixture._root.path().join("single-reencode.mp4");
    assert!(probe.probe(&library.export(&single.id, &target, ExportMode::Reencode).unwrap()[0]).unwrap().audio_codec.is_some());
    let batch = fixture._root.path().join("batch");
    let ids: Vec<_> = session.flights.iter().map(|f| f.id.clone()).collect();
    library.export_flights(&ids, &batch, ExportMode::Parts, |_,_,_| {}).unwrap();
    let exported: Vec<_> = std::fs::read_dir(&batch).unwrap().map(|f| probe.probe(&f.unwrap().path()).unwrap()).collect();
    assert_eq!(exported.len(), 2);
    assert_eq!(exported.iter().filter(|m| m.audio_codec.is_none()).count(), 1);
    let skipped = library.import_with_audio(&fixture.card, "Audio choices", Some("2026-08-23"), false, false, |_| {}).unwrap();
    assert_eq!((skipped.imported, skipped.skipped), (0, 3));
    assert!(library.native_playback(&muted_flight.id).unwrap().iter().all(|p| !p.has_audio));
}

#[test]
fn source_boundary_previews_and_multipart_edits_are_non_destructive() {
    let (fixture, session) = imported();
    let flight = &session.flights[0];
    assert_eq!(flight.clips.len(), 2);
    let mut originals = vec![];
    for clip in &flight.clips {
        let source = fixture.library.native_source_playback(&clip.file_id).unwrap();
        assert_eq!((source.segment.start, source.segment.end, source.segment.offset), (0.0, clip.file_duration, 0.0));
        assert!(source.has_audio);
        originals.push((source.segment.path.clone(), std::fs::read(&source.segment.path).unwrap()));
        fixture.library.update_clip(&clip.id, 1, clip.last_frame + 1).unwrap();
    }
    // Discarding an unsaved draft leaves the persisted selection untouched.
    let mut draft = fixture.library.snapshot().unwrap()[0].flights[0].clips[0].clone();
    draft.start_frame = 9;
    drop(draft);
    let reopened = open(fixture._root.path());
    let snapshot = reopened.snapshot().unwrap();
    for (edited, original) in snapshot[0].flights[0].clips.iter().zip(&flight.clips) {
        assert_eq!((edited.start_frame, edited.end_frame), (1, original.last_frame + 1));
    }
    for (path, bytes) in originals { assert_eq!(std::fs::read(path).unwrap(), bytes); }
    let file = fixture.library.store.source_file(parse(&flight.clips[0].file_id).unwrap()).unwrap().unwrap();
    fixture.library.store.save_source_files(&[SourceFile { muted: true, ..file }]).unwrap();
    assert!(!fixture.library.native_source_playback(&flight.clips[0].file_id).unwrap().has_audio);
}

#[test]
fn flight_info_preserves_source_paths_rates_and_each_parts_exclusive_bounds_without_reading_media() {
    use fpv_core::{FrameRate, Clip, Flight, DetectionRun};
    let root = tempfile::tempdir().unwrap();
    let library = open(root.path());
    let session = Session::new("Info", None);
    library.store.save_session(&session).unwrap();
    let flight = Flight::new(session.id, 1);
    let mut expected = Vec::new();
    for (index, rate, start, end) in [(1, FrameRate::new(1_000_000, 33459), 100, 250),
                                     (2, FrameRate::new(1_000_000, 40839), 0, 90)] {
        let path = root.path().join(format!("PICT{index:04}.AVI"));
        // Arbitrary bytes: Info must use stored metadata, not probe the file.
        if index == 1 { std::fs::write(&path, b"unchanged source").unwrap(); }
        let mut file = SourceFile::new(session.id, format!("/original/PICT{index:04}.AVI"),
            path.to_string_lossy(), Some(index), MediaInfo::new(640, 480, rate, 20.0, "mjpeg"));
        file.media.audio_codec = Some("pcm_s16le".into());
        file.muted = index == 2;
        library.store.save_source_files(std::slice::from_ref(&file)).unwrap();
        let clip = Clip::new(flight.id, file.id, start, end);
        library.store.replace_clips(file.id, std::slice::from_ref(&clip),
            if index == 1 { std::slice::from_ref(&flight) } else { &[] },
            &DetectionRun::new(file.id, "test", Default::default())).unwrap();
        expected.push((clip, file));
    }
    let info = library.flight_info(&uuid_string(flight.id)).unwrap();
    assert_eq!(info.len(), 2);
    for (part, (clip, source)) in info.iter().zip(&expected) {
        assert_eq!(&part.source, source);
        assert_eq!(part.clip.start_frame, clip.start_frame);
        assert_eq!(part.clip.end_frame, clip.end_frame);
        assert_eq!(part.available, source.sequence_number == Some(1));
        assert_eq!(part.size_bytes, if part.available { Some(16) } else { None });
        assert!(!part.cloud);
    }
    assert_eq!(std::fs::read(&expected[0].1.imported_path).unwrap(), b"unchanged source");
    assert!(!Path::new(&expected[1].1.imported_path).exists());
    let again = library.flight_info(&uuid_string(flight.id)).unwrap();
    assert_eq!(again[1].clip.end_frame, 90);
    assert!(library.flight_info(&uuid_string(Uuid::new_v4())).is_err());
}
