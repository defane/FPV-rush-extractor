use std::collections::BTreeMap;

use chrono::DateTime;

use super::*;

fn media(duration: f64) -> MediaInfo {
    MediaInfo {
        frame_count: Some(4217),
        audio_codec: Some("pcm_s16le".into()),
        ..MediaInfo::new(640, 480, FrameRate::new(1_000_000, 33459), duration, "mjpeg")
    }
}

fn file(session: &Session, name: &str, sequence: i64) -> SourceFile {
    SourceFile::new(session.id, format!("/card/{name}"), format!("/library/{name}"),
                    Some(sequence), media(141.09))
}

fn run(file: Uuid, version: &str) -> DetectionRun {
    DetectionRun::new(file, version, BTreeMap::new())
}

fn session_with_one_file(store: &LibraryStore, name: &str, sequence: i64) -> (Session, SourceFile) {
    let session = Session::new("Letz Kwad", None);
    store.save_session(&session).unwrap();
    let file = file(&session, name, sequence);
    store.save_source_files(std::slice::from_ref(&file)).unwrap();
    (session, file)
}

/// One flight of one clip, saved as a detection run would.
fn one_clip(store: &LibraryStore, file: &SourceFile, start: i64, end: i64) -> (Flight, Clip) {
    let flight = Flight::new(file.session_id, 1);
    let clip = Clip::new(flight.id, file.id, start, end);
    store.replace_clips(file.id, std::slice::from_ref(&clip), std::slice::from_ref(&flight),
                        &run(file.id, "v1")).unwrap();
    (flight, clip)
}

/// SQLite keeps dates to the millisecond, so a tag stamped with `Utc::now()`
/// does not compare equal after a round trip; a whole second does.
fn tag(file: Uuid, start: i64, end: i64, label: &str) -> FigureTag {
    FigureTag {
        created_at: DateTime::from_timestamp(1_787_443_200, 0).unwrap(),
        ..FigureTag::new(file, start, end, label)
    }
}

#[test]
fn applies_the_schema_to_a_fresh_library() {
    assert!(LibraryStore::in_memory().unwrap().sessions().unwrap().is_empty());
}

#[test]
fn keeps_the_frame_rate_exact_across_a_save_and_load() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, _) = session_with_one_file(&store, "PICT0030.AVI", 30);
    let loaded = &store.source_files(session.id).unwrap()[0];
    // Storing 29.887 as a float and rebuilding a ratio from it would not come
    // back as 1000000/33459.
    assert_eq!(loaded.media.frame_rate, FrameRate::new(1_000_000, 33459));
    assert_eq!(loaded.sequence_number, Some(30));
    assert_eq!(loaded.media.audio_codec.as_deref(), Some("pcm_s16le"));
}

#[test]
fn holds_a_flight_that_spans_two_files_and_plays_it_in_card_order() {
    let store = LibraryStore::in_memory().unwrap();
    let session = Session::new("Letz Kwad", None);
    store.save_session(&session).unwrap();
    let first = file(&session, "PICT0030.AVI", 30);
    let second = SourceFile { media: media(433.9), ..file(&session, "PICT0031.AVI", 31) };
    store.save_source_files(&[first.clone(), second.clone()]).unwrap();

    let flight = Flight::new(session.id, 1);
    let tail = Clip { entry: ClipEntry::Poweron, exit: ClipExit::Eof,
                      ..Clip::new(flight.id, first.id, 4194, 4217) };
    let head = Clip { entry: ClipEntry::Continuation, exit: ClipExit::Menu,
                      ..Clip::new(flight.id, second.id, 0, 1554) };
    // Saved out of order on purpose: playing order must come from the card.
    store.replace_clips(second.id, &[head.clone(), tail.clone()], &[flight],
                        &run(second.id, "spike-2026-09-05")).unwrap();

    let flights = store.flights(session.id).unwrap();
    assert_eq!(flights.len(), 1);
    assert_eq!(flights[0].1, [tail, head]);
}

#[test]
fn keeps_the_parameters_that_produced_a_clip_set() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let flight = Flight::new(file.session_id, 1);
    let parameters = BTreeMap::from([("snow_blkcv".to_owned(), 0.6),
                                     ("min_clip_coherence".to_owned(), 0.5)]);
    store.replace_clips(file.id, &[Clip::new(flight.id, file.id, 0, 100)], &[flight],
                        &DetectionRun::new(file.id, "spike-2026-09-05", parameters.clone()))
        .unwrap();
    let saved = &store.detection_runs(file.id).unwrap()[0];
    assert_eq!(saved.algorithm_version, "spike-2026-09-05");
    assert_eq!(saved.parameters, parameters);
}

#[test]
fn rerunning_the_detector_replaces_clips_and_drops_emptied_flights() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0036.AVI", 36);
    one_clip(&store, &file, 0, 200);
    assert_eq!(store.flights(session.id).unwrap().len(), 1);

    // A stricter detector now rejects this file outright — PICT0036 is another
    // pilot's interference. The flight it used to hold must not linger empty.
    store.replace_clips(file.id, &[], &[], &run(file.id, "v2")).unwrap();
    assert!(store.flights(session.id).unwrap().is_empty());
    assert_eq!(store.detection_runs(file.id).unwrap().len(), 2);
}

#[test]
fn persists_a_hand_corrected_boundary() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let (_, detected) = one_clip(&store, &file, 13538, 17080);
    let corrected = Clip { start_frame: 13583, ..detected };
    store.update_clip(&corrected).unwrap();
    assert_eq!(store.flights(session.id).unwrap()[0].1, [corrected]);
}

#[test]
fn refuses_to_update_a_clip_that_does_not_exist() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let stranger = Clip::new(Uuid::new_v4(), file.id, 0, 10);
    assert!(matches!(store.update_clip(&stranger), Err(StoreError::NotFound { .. })));
}

#[test]
fn finds_a_flight_a_file_and_a_clip_by_id() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let (flight, clip) = one_clip(&store, &file, 10, 20);
    assert_eq!(store.flight(flight.id).unwrap(), Some((flight, vec![clip.clone()])));
    assert_eq!(store.source_file(file.id).unwrap(), Some(file));
    assert_eq!(store.clip(clip.id).unwrap(), Some(clip));
    assert_eq!(store.flight(Uuid::new_v4()).unwrap(), None);
}

#[test]
fn persists_a_flight_rating() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0038.AVI", 38);
    let (flight, _) = one_clip(&store, &file, 0, 100);
    store.update_flight(&Flight { rating: Some(1), ..flight }).unwrap();
    assert_eq!(store.flights(session.id).unwrap()[0].0.rating, Some(1));
}

#[test]
fn saving_a_session_again_does_not_cascade_through_it() {
    // The trap an INSERT OR REPLACE would spring: delete the old row, and its
    // files, flights and clips with it.
    let store = LibraryStore::in_memory().unwrap();
    let (mut session, file) = session_with_one_file(&store, "PICT0030.AVI", 30);
    one_clip(&store, &file, 0, 100);
    session.name = "Letz Kwad, renamed".into();
    store.save_session(&session).unwrap();
    assert_eq!(store.sessions().unwrap()[0].name, "Letz Kwad, renamed");
    assert_eq!(store.flights(session.id).unwrap().len(), 1);
}

#[test]
fn deleting_a_session_cascades_to_everything_it_owns() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0030.AVI", 30);
    one_clip(&store, &file, 0, 100);
    store.save_tag(&tag(file.id, 0, 10, "split-s")).unwrap();

    store.delete_session(session.id).unwrap();

    assert!(store.sessions().unwrap().is_empty());
    assert!(store.source_files(session.id).unwrap().is_empty());
    assert!(store.flights(session.id).unwrap().is_empty());
    assert!(store.detection_runs(file.id).unwrap().is_empty());
    assert!(store.tags_of_source_file(file.id).unwrap().is_empty());
    assert!(store.tag_labels().unwrap().is_empty());
}

#[test]
fn a_figure_tag_survives_re_segmentation() {
    // The regression test for the cascade trap: re-segmenting deletes every
    // clip of the file, and a tag owned by a clip would go with them.
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    one_clip(&store, &file, 1000, 5000);
    let tag = FigureTag { note: Some("clean one".into()), ..tag(file.id, 1500, 1590, "split-s") };
    store.save_tag(&tag).unwrap();

    let reflight = Flight::new(file.session_id, 1);
    let reclip = Clip::new(reflight.id, file.id, 1045, 5000);
    store.replace_clips(file.id, std::slice::from_ref(&reclip), &[reflight],
                        &run(file.id, "v2")).unwrap();

    assert_eq!(store.tags_of_source_file(file.id).unwrap(), std::slice::from_ref(&tag));
    // And it resolves against the new clip, which has a new identity.
    assert_eq!(store.tags_of_clip(&reclip).unwrap(), [tag]);
}

#[test]
fn resolves_tags_to_the_clip_they_overlap() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let flight = Uuid::new_v4();
    let early = Clip::new(flight, file.id, 0, 999);
    let late = Clip::new(flight, file.id, 2000, 2999);
    let in_early = tag(file.id, 500, 500, "flip");
    let straddling = tag(file.id, 1900, 2100, "power loop");
    let in_gap = tag(file.id, 1200, 1300, "roll");
    for t in [&straddling, &in_gap, &in_early] {
        store.save_tag(t).unwrap();
    }
    assert_eq!(store.tags_of_clip(&early).unwrap(), std::slice::from_ref(&in_early));
    assert_eq!(store.tags_of_clip(&late).unwrap(), std::slice::from_ref(&straddling));
    assert_eq!(store.tags_of_source_file(file.id).unwrap(), [in_early, in_gap, straddling]);
}

#[test]
fn edits_and_deletes_a_tag() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let mut t = tag(file.id, 10, 20, "split-s");
    store.save_tag(&t).unwrap();
    t.label = "matty flip".into();
    t.end_frame = 40;
    store.save_tag(&t).unwrap();
    assert_eq!(store.tags_in_session(session.id).unwrap(), [t.clone()]);

    store.delete_tag(t.id).unwrap();
    assert!(store.tags_in_session(session.id).unwrap().is_empty());
}

#[test]
fn lists_a_sessions_tags_in_card_order() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, second) = session_with_one_file(&store, "PICT0031.AVI", 31);
    let first = file(&session, "PICT0030.AVI", 30);
    store.save_source_files(std::slice::from_ref(&first)).unwrap();
    let (_, other) = session_with_one_file(&store, "PICT0040.AVI", 40);

    let late = tag(second.id, 5, 5, "b");
    let early = tag(first.id, 4000, 4100, "a");
    store.save_tag(&late).unwrap();
    store.save_tag(&early).unwrap();
    store.save_tag(&tag(other.id, 0, 1, "c")).unwrap();

    assert_eq!(store.tags_in_session(session.id).unwrap(), [early, late]);
}

#[test]
fn offers_tag_labels_most_used_first() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    for (index, label) in ["split-s", "power loop", "split-s", "dive", "split-s", "dive"]
        .into_iter().enumerate() {
        store.save_tag(&tag(file.id, index as i64, index as i64, label)).unwrap();
    }
    assert_eq!(store.tag_labels().unwrap(), ["split-s", "dive", "power loop"]);
}

#[test]
fn keeps_the_proxy_index_in_eviction_order() {
    let store = LibraryStore::in_memory().unwrap();
    let at = |s| DateTime::from_timestamp(s, 0).unwrap();
    let entry = |key: &str, used| ProxyEntry {
        key: key.into(), bytes: 100, created_at: at(0), last_used_at: at(used),
    };
    store.save_proxy(&entry("recent", 20)).unwrap();
    store.save_proxy(&entry("stale", 10)).unwrap();
    assert_eq!(store.proxies().unwrap().iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
               ["stale", "recent"]);
    store.save_proxy(&entry("stale", 30)).unwrap();
    assert_eq!(store.proxies().unwrap()[0].key, "recent");
    assert_eq!(store.proxy("stale").unwrap(), Some(entry("stale", 30)));
    store.delete_proxy("stale").unwrap();
    store.delete_all_proxies().unwrap();
    assert!(store.proxies().unwrap().is_empty());
}

#[test]
fn upgrades_an_existing_v1_library_in_place() {
    // A real library already ran v1. Every later migration has to arrive on
    // top of it with the footage rows intact, not on a fresh database only.
    // The row is written as a v1 build wrote it: v1's columns only.
    let store = LibraryStore::in_memory_up_to("v1").unwrap();
    let session = Session { date: Some(DateTime::from_timestamp(1_787_443_200, 0).unwrap()),
                            ..Session::new("Letz Kwad", None) };
    store.db().execute("INSERT INTO session (id, name, date) VALUES (?1, ?2, ?3)",
                       params![Id(session.id), session.name, session.date.map(Date)]).unwrap();
    let connection = store.connection.into_inner().unwrap();

    let store = LibraryStore::with_connection(connection, None).unwrap();
    assert_eq!(store.sessions().unwrap(), [session]);
    assert!(store.tag_labels().unwrap().is_empty());
}

#[test]
fn writes_ids_and_dates_as_grdb_does() {
    let store = LibraryStore::in_memory().unwrap();
    let date = DateTime::parse_from_rfc3339("2026-10-18T21:06:32.099Z").unwrap().to_utc();
    let session = Session { date: Some(date), ..Session::new("Letz Kwad", None) };
    store.save_session(&session).unwrap();
    let (id, stored): (String, String) = store.db()
        .query_row("SELECT id, date FROM session", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(id, fpv_core::uuid_string(session.id));
    assert_eq!(id, id.to_uppercase());
    assert_eq!(stored, "2026-10-18 21:06:32.099");
}

#[test]
fn keeps_a_sessions_detection_profile_and_defaults_old_ones_to_standard() {
    // A library at v2 — every session before profiles existed.
    let store = LibraryStore::in_memory_up_to("v2").unwrap();
    store.db().execute("INSERT INTO session (id, name) VALUES ('6F1C2A3B-4D5E-4F60-8A7B-9C0D1E2F3A4B', 'Old')", [])
        .unwrap();
    let connection = store.connection.into_inner().unwrap();
    let store = LibraryStore::with_connection(connection, None).unwrap();
    assert_eq!(store.sessions().unwrap()[0].detection_profile, DetectionProfile::Standard);

    let alps = Session { detection_profile: DetectionProfile::Textured, ..Session::new("Alpes", None) };
    store.save_session(&alps).unwrap();
    let read = store.sessions().unwrap().into_iter().find(|s| s.id == alps.id).unwrap();
    assert_eq!(read.detection_profile, DetectionProfile::Textured);
}

#[test]
fn keeps_the_camera_a_file_names() {
    let store = LibraryStore::in_memory().unwrap();
    let session = Session::new("Alpes", None);
    store.save_session(&session).unwrap();
    let take = SourceFile::new(session.id, "/card/GH010042.MP4", "/library/GH010042.MP4", None,
        MediaInfo { camera: Some("GoPro HERO11".into()),
                    ..MediaInfo::new(1920, 1080, FrameRate::new(60, 1), 10.0, "h264") });
    store.save_source_files(std::slice::from_ref(&take)).unwrap();
    assert_eq!(store.source_file(take.id).unwrap().unwrap().media.camera.as_deref(),
               Some("GoPro HERO11"));
}

#[test]
fn lists_action_cam_files_never_read_for_their_camera_once() {
    // Files imported before the camera column: NULL. The DVR names no camera
    // and is never listed; a take that named none is not read again.
    let store = LibraryStore::in_memory().unwrap();
    let (session, dvr) = session_with_one_file(&store, "PICT0030.AVI", 30);
    let take = SourceFile::new(session.id, "/card/DJI_0001.MP4", "/library/DJI_0001.MP4", None,
                               MediaInfo::new(3840, 2160, FrameRate::new(60, 1), 10.0, "hevc"));
    store.save_source_files(std::slice::from_ref(&take)).unwrap();
    let listed: Vec<_> = store.unidentified_cameras().unwrap().iter().map(|f| f.id).collect();
    assert_eq!(listed, [take.id]);
    assert!(!listed.contains(&dvr.id));

    store.set_camera(take.id, None).unwrap();
    assert!(store.unidentified_cameras().unwrap().is_empty());
    assert_eq!(store.source_file(take.id).unwrap().unwrap().media.camera, None);
    store.set_camera(take.id, Some("DJI Osmo Action 4")).unwrap();
    assert_eq!(store.source_file(take.id).unwrap().unwrap().media.camera.as_deref(),
               Some("DJI Osmo Action 4"));
}

#[test]
fn labels_a_flight_once_whatever_the_case_and_forgets_a_removed_label() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let flight = one_clip(&store, &file, 0, 200).0.id;
    store.add_flight_label(flight, "freestyle").unwrap();
    store.add_flight_label(flight, "Freestyle").unwrap();
    store.add_flight_label(flight, "gates").unwrap();
    assert_eq!(store.flight_labels_in_session(session.id).unwrap()[&flight], ["freestyle", "gates"]);

    store.remove_flight_label(flight, "FREESTYLE").unwrap();
    assert_eq!(store.flight_labels_in_session(session.id).unwrap()[&flight], ["gates"]);
    assert!(store.add_flight_label(Uuid::new_v4(), "gates").is_err(), "no such flight");
}

#[test]
fn suggests_the_most_used_flight_labels_first() {
    let store = LibraryStore::in_memory().unwrap();
    let (_, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let (_, other) = session_with_one_file(&store, "PICT0033.AVI", 33);
    let first = one_clip(&store, &file, 0, 200).0.id;
    let second = one_clip(&store, &other, 300, 500).0.id;
    store.add_flight_label(first, "cinematic").unwrap();
    store.add_flight_label(first, "gates").unwrap();
    store.add_flight_label(second, "Gates").unwrap();
    // The spelling first used names it.
    assert_eq!(store.flight_label_names().unwrap(), ["gates", "cinematic"]);
}

#[test]
fn drops_a_flights_labels_with_the_flight() {
    // Re-segmenting replaces a file's flights: their labels go with them, as
    // their titles and likes do.
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0036.AVI", 36);
    let flight = one_clip(&store, &file, 0, 200).0.id;
    store.add_flight_label(flight, "crash").unwrap();
    one_clip(&store, &file, 0, 150);
    assert!(store.flight_labels_in_session(session.id).unwrap().is_empty());
    assert!(store.flight_label_names().unwrap().is_empty());
}

/// PICT0032 with a flight at frames 1000-2000 and the snow around it; the
/// piece kept is frames 900-2100, renumbered from 0.
fn reduction(store: &LibraryStore) -> (Session, SourceFile, Clip, [FigureTag; 2], SourceFile) {
    let (session, file) = session_with_one_file(store, "PICT0032.AVI", 32);
    let (_, clip) = one_clip(store, &file, 1000, 2000);
    let inside = tag(file.id, 1500, 1600, "split-S");
    let snow = tag(file.id, 50, 60, "nothing");
    store.save_tag(&inside).unwrap();
    store.save_tag(&snow).unwrap();
    let piece = SourceFile::new(session.id, file.original_path.clone(), "/library/PICT0032-1.AVI",
                                Some(32), media(48.0));
    (session, file, clip, [inside, snow], piece)
}

#[test]
fn swaps_a_recording_for_its_pieces_moving_clips_and_tags() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file, clip, [inside, _], piece) = reduction(&store);
    let moved_clip = Clip { source_file_id: piece.id, start_frame: 100, end_frame: 1100, ..clip.clone() };
    let moved_tag = FigureTag { source_file_id: piece.id, start_frame: 600, end_frame: 700, ..inside.clone() };
    let run = DetectionRun::new(piece.id, REDUCED_ALGORITHM, BTreeMap::new());
    store.apply_reduction(file.id, std::slice::from_ref(&piece), std::slice::from_ref(&moved_clip),
                          std::slice::from_ref(&moved_tag), &[run]).unwrap();

    assert_eq!(store.source_files(session.id).unwrap(), std::slice::from_ref(&piece));
    assert_eq!(store.flights(session.id).unwrap()[0].1, [moved_clip]);
    // The tag in the flight moved with it; the one in the snow went with the recording.
    assert_eq!(store.tags_in_session(session.id).unwrap(), [moved_tag]);
    assert_eq!(store.reduced_files(session.id).unwrap(), HashSet::from([piece.id]));
}

#[test]
fn leaves_everything_as_it_was_when_a_reduction_fails_halfway() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file, clip, tags, piece) = reduction(&store);
    // A clip the library does not hold: the transaction must not half-apply.
    let stray = Clip::new(clip.flight_id, piece.id, 0, 10);
    assert!(store.apply_reduction(file.id, std::slice::from_ref(&piece), &[stray], &[], &[]).is_err());
    assert_eq!(store.source_files(session.id).unwrap(), [file]);
    assert_eq!(store.flights(session.id).unwrap()[0].1, [clip]);
    assert_eq!(store.tags_in_session(session.id).unwrap().len(), tags.len());
    assert!(store.reduced_files(session.id).unwrap().is_empty());
}

#[test]
fn deletes_a_flight_with_the_recordings_that_held_only_it() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032-2.AVI", 32);
    let (flight, _) = one_clip(&store, &file, 0, 200);
    store.add_flight_label(flight.id, "crash").unwrap();
    store.save_tag(&tag(file.id, 10, 20, "dive")).unwrap();
    store.delete_flight(flight.id, &[file.id]).unwrap();
    assert!(store.flights(session.id).unwrap().is_empty());
    assert!(store.source_files(session.id).unwrap().is_empty());
    assert!(store.tags_in_session(session.id).unwrap().is_empty());
    assert!(store.flight_label_names().unwrap().is_empty());
    // Remembered, so loading the import folder does not bring it back unasked.
    assert_eq!(store.deleted_paths().unwrap(), HashSet::from([file.imported_path.clone()]));
    store.forget_deleted(std::slice::from_ref(&file.imported_path)).unwrap();
    assert!(store.deleted_paths().unwrap().is_empty());
    // A flight that is not there: nothing happens, and it says so.
    assert!(store.delete_flight(flight.id, &[]).is_err());
}

#[test]
fn deletes_only_the_flight_when_its_recording_stays() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let (flight, _) = one_clip(&store, &file, 0, 200);
    store.delete_flight(flight.id, &[]).unwrap();
    assert!(store.flights(session.id).unwrap().is_empty());
    assert_eq!(store.source_files(session.id).unwrap(), [file]);
    assert!(store.deleted_paths().unwrap().is_empty(), "still in the library");
}

#[test]
fn keeps_a_sessions_tags_apart_from_its_flights_and_through_a_re_segment() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    store.set_session_labels(session.id, &["Alpes".into(), "race".into(), "ALPES".into()]).unwrap();
    assert_eq!(store.session_labels().unwrap()[&session.id], ["Alpes", "race"]);
    // A re-segment remakes the flights; the session's tags stay.
    one_clip(&store, &file, 0, 200);
    one_clip(&store, &file, 0, 150);
    assert_eq!(store.session_labels().unwrap()[&session.id], ["Alpes", "race"]);
    store.set_session_labels(session.id, &["race".into()]).unwrap();
    assert_eq!(store.session_labels().unwrap()[&session.id], ["race"]);
}

#[test]
fn suggests_session_and_flight_tags_as_one_vocabulary() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let flight = one_clip(&store, &file, 0, 200).0.id;
    store.add_flight_label(flight, "gates").unwrap();
    store.set_session_labels(session.id, &["Gates".into(), "Alpes".into()]).unwrap();
    assert_eq!(store.flight_label_names().unwrap(), ["gates", "Alpes"]);
}

#[test]
fn records_where_a_moved_recording_now_lives() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    store.set_imported_path(file.id, "/new/import/2026-08-23 Letz Kwad FATSHARK/PICT0032.AVI").unwrap();
    assert_eq!(store.source_files(session.id).unwrap()[0].imported_path,
               "/new/import/2026-08-23 Letz Kwad FATSHARK/PICT0032.AVI");
    assert!(store.set_imported_path(Uuid::new_v4(), "/x").is_err());
}

#[test]
fn persists_recording_mute_without_changing_audio_metadata() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, mut recording) = session_with_one_file(&store, "PICT0030.AVI", 30);
    recording.muted = true;
    let id = recording.id;
    store.save_source_files(&[recording]).unwrap();
    assert!(store.source_files(session.id).unwrap()[0].muted);
    let connection = store.connection.into_inner().unwrap();
    let store = LibraryStore::with_connection(connection, None).unwrap();
    let recording = store.source_file(id).unwrap().unwrap();
    assert!(recording.muted);
    assert!(recording.media.audio_codec.is_some());
}

#[test]
fn upgrading_v7_keeps_existing_recordings_audible() {
    let store = LibraryStore::in_memory_up_to("v7").unwrap();
    let session = Session::new("Old library", None);
    store.save_session(&session).unwrap();
    let id = Uuid::new_v4();
    store.db().execute("INSERT INTO sourceFile (id, sessionId, originalPath, importedPath, width, height,
        frameRateNumerator, frameRateDenominator, duration, videoCodec, audioCodec)
        VALUES (?1, ?2, '/card/old.avi', '/library/old.avi', 640, 480, 25, 1, 1.0, 'mjpeg', 'pcm_s16le')",
        params![Id(id), Id(session.id)]).unwrap();
    let store = LibraryStore::with_connection(store.connection.into_inner().unwrap(), None).unwrap();
    let file = store.source_file(id).unwrap().unwrap();
    assert!(!file.muted);
    assert_eq!(file.media.audio_codec.as_deref(), Some("pcm_s16le"));
}

#[test]
fn playback_mute_migrates_from_v8_is_per_flight_and_survives_reopening() {
    let root = std::env::temp_dir().join(format!("fpv-mute-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("library.sqlite");
    let store = LibraryStore::with_connection(Connection::open(&path).unwrap(), Some("v8")).unwrap();
    let (session, source) = session_with_one_file(&store, "PICT0001.AVI", 1);
    let (first, _) = one_clip(&store, &source, 0, 100);
    let second_source = file(&session, "PICT0002.AVI", 2);
    store.save_source_files(std::slice::from_ref(&second_source)).unwrap();
    let (second, _) = one_clip(&store, &second_source, 0, 100);
    drop(store);
    let store = LibraryStore::open(&path).unwrap();
    assert_eq!(store.playback_mute(first.id).unwrap(), None);
    store.set_playback_mute(first.id, true).unwrap();
    store.set_playback_mute(second.id, false).unwrap();
    drop(store);
    let store = LibraryStore::open(&path).unwrap();
    assert_eq!(store.playback_mute(first.id).unwrap(), Some(true));
    assert_eq!(store.playback_mute(second.id).unwrap(), Some(false));
    store.set_playback_mute(first.id, false).unwrap();
    assert_eq!(store.playback_mute(first.id).unwrap(), Some(false));
    assert_eq!(store.source_file(source.id).unwrap().unwrap(), source, "export/import policy unchanged");
    store.delete_flight(first.id, &[]).unwrap();
    assert_eq!(store.playback_mute(first.id).unwrap(), None, "deleted flights leave no preference");
    assert!(store.set_playback_mute(Uuid::new_v4(), true).is_err());
    assert_eq!(store.playback_mute(second.id).unwrap(), Some(false));
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn batch_deletion_rolls_back_when_any_selected_flight_is_missing() {
    let store = LibraryStore::in_memory().unwrap();
    let (session, file) = session_with_one_file(&store, "PICT0032.AVI", 32);
    let (flight, _) = one_clip(&store, &file, 0, 200);
    assert!(store.delete_flights(&[flight.id, Uuid::new_v4()], &[file.id]).is_err());
    assert_eq!(store.flights(session.id).unwrap().len(), 1);
    assert!(store.source_file(file.id).unwrap().is_some());
    assert!(store.deleted_paths().unwrap().is_empty());
}
