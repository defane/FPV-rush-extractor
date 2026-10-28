use std::collections::HashMap;

use chrono::{Local, TimeZone};

use fpv_core::FrameRate;
use fpv_media::{FfmpegError, FfmpegTool, ProcessResult};

use super::*;
use crate::FolderPattern;

/// ffprobe output by file name: the reference card's MJPEG for `PICT` files,
/// a GoPro's H.264 with its (local, mislabelled UTC) clock for the rest.
/// Stubbed so the tests need no media, only files that exist.
struct StubProbe {
    taken_at: HashMap<&'static str, &'static str>,
    fail: bool,
}

impl StubProbe {
    fn new(taken_at: &[(&'static str, &'static str)]) -> Self {
        Self { taken_at: taken_at.iter().copied().collect(), fail: false }
    }
}

impl FfmpegInvoking for StubProbe {
    fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
        if self.fail {
            return Ok(ProcessResult { status: 1, stdout: vec![], stderr: "Invalid data found".into() });
        }
        // ffmpeg itself (the poster frame) "succeeds" and writes nothing.
        let name = file_name(Path::new(arguments.last().unwrap()));
        let json = match (tool, self.taken_at.get(name.as_str())) {
            (FfmpegTool::Ffmpeg, _) => String::new(),
            (_, None) => r#"{ "streams": [ { "codec_name": "mjpeg", "codec_type": "video",
                "width": 640, "height": 480, "r_frame_rate": "1000000/33459", "nb_frames": "4217" } ],
                "format": { "duration": "141.096603" } }"#.into(),
            (_, Some(taken)) => format!(r#"{{ "streams": [ {{ "codec_name": "h264",
                "codec_type": "video", "width": 1920, "height": 1080,
                "r_frame_rate": "60000/1001", "nb_frames": "3596" }} ],
                "format": {{ "duration": "60.0", "tags": {{ "creation_time": "{taken}" }} }} }}"#),
        };
        Ok(ProcessResult { status: 0, stdout: json.into_bytes(), stderr: String::new() })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    card: PathBuf,
    library: PathBuf,
}

impl Fixture {
    fn new(files: &[&str]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let (card, library) = (root.path().join("card"), root.path().join("library"));
        for name in files {
            let path = card.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // Unique bytes per name: two real recordings are never identical,
            // and identical ones are exactly what the checksum skip must catch.
            std::fs::write(path, name.as_bytes()).unwrap();
        }
        Self { _root: root, card, library }
    }

    fn import(&self, ffmpeg: &dyn FfmpegInvoking, store: &LibraryStore, name: &str) -> ImportReport {
        LibraryImporter::new(ffmpeg, store)
            .import_card(&self.card, &self.library, name, None, |_| {})
            .unwrap()
    }
}

fn dvr() -> StubProbe {
    StubProbe::new(&[])
}

#[test]
fn imports_a_card_into_one_session_and_records_it() {
    let fixture = Fixture::new(&["PICT0030.AVI", "PICT0031.AVI", "PICT0032.AVI", "notes.txt"]);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&dvr(), &store, "Letz Kwad");

    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.imported.len(), 3); // notes.txt is not a recording
    assert!(report.failed.is_empty());
    let files = store.source_files(report.sessions[0].id).unwrap();
    assert_eq!(files.iter().map(|f| f.sequence_number).collect::<Vec<_>>(),
               [Some(30), Some(31), Some(32)]);
    assert!(files.iter().all(|f| f.media.frame_rate == FrameRate::new(1_000_000, 33459)));
    // The card is read-only: the originals are still there.
    assert!(fixture.card.join("PICT0030.AVI").exists());
    assert!(Path::new(&files[0].imported_path).exists());
    // A DVR card makes no flights: those come from segmentation.
    assert!(report.flights.is_empty());
}

#[test]
fn splits_a_card_whose_counter_jumps_into_numbered_sessions() {
    let fixture = Fixture::new(&["PICT0030.AVI", "PICT0031.AVI", "PICT0058.AVI", "PICT0059.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&dvr(), &store, "Letz Kwad");
    assert_eq!(report.sessions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
               ["Letz Kwad (1)", "Letz Kwad (2)"]);
    for session in &report.sessions {
        assert_eq!(store.source_files(session.id).unwrap().len(), 2);
    }
}

#[test]
fn skips_recordings_already_in_the_library() {
    let fixture = Fixture::new(&["PICT0030.AVI", "PICT0031.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    fixture.import(&dvr(), &store, "Letz Kwad");
    let second = fixture.import(&dvr(), &store, "Letz Kwad");
    assert!(second.imported.is_empty());
    assert_eq!(second.skipped.len(), 2);
    // An import that imports nothing must leave nothing behind.
    assert!(second.sessions.is_empty());
    assert_eq!(store.sessions().unwrap().len(), 1);
}

#[test]
fn carries_on_past_a_file_it_cannot_read() {
    let fixture = Fixture::new(&["PICT0030.AVI", "PICT0031.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&StubProbe { fail: true, ..dvr() }, &store, "Letz Kwad");
    // The card was walked to the end and both failures reported. Probing comes
    // before anything is created now, so no empty session is left either.
    assert_eq!(report.failed.len(), 2);
    assert!(report.imported.is_empty());
    assert!(store.sessions().unwrap().is_empty());
}

#[test]
fn skips_a_recording_whose_bytes_are_already_in_the_library() {
    let fixture = Fixture::new(&["PICT0030.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    fixture.import(&dvr(), &store, "First outing");
    // The same recording back under a new name: a reformatted card restarted
    // the counter. The name check cannot catch it; the content does.
    std::fs::write(fixture.card.join("PICT0001.AVI"), b"PICT0030.AVI").unwrap();
    let report = fixture.import(&dvr(), &store, "Second outing");
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped.len(), 2); // PICT0030 by name, PICT0001 by content
    assert!(report.sessions.is_empty());
    let session = &store.sessions().unwrap()[0];
    assert_eq!(store.source_files(session.id).unwrap()[0].checksum.as_ref().map(String::len),
               Some(64));
}

#[test]
fn keeps_the_mtime_of_the_card_file() {
    let fixture = Fixture::new(&["PICT0030.AVI"]);
    let stamp = FileTime::from_unix_time(315_532_800, 0); // 1980: the DVR's idea of a date
    filetime::set_file_mtime(fixture.card.join("PICT0030.AVI"), stamp).unwrap();
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&dvr(), &store, "Letz Kwad");
    let copied = std::fs::metadata(&report.imported[0].imported_path).unwrap();
    assert_eq!(FileTime::from_last_modification_time(&copied), stamp);
}

#[test]
fn names_the_library_folder_after_the_session_by_default() {
    // The CLI's layout, and every earlier build's: no pattern, the bare name.
    let fixture = Fixture::new(&["PICT0030.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&dvr(), &store, "Letz Kwad");
    assert!(Path::new(&report.imported[0].imported_path)
        .ends_with(Path::new("Letz Kwad").join("PICT0030.AVI")));
}

#[test]
fn names_folders_from_the_pattern_one_per_source() {
    // The pilot's layout: "2026-10-10 Alpes FATSHARK", and the day's GoPro
    // takes beside it in "2026-10-10 Alpes GOPRO" — one session in the app.
    let card = Fixture::new(&["PICT0030.AVI", "PICT0031.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let day = Local.with_ymd_and_hms(2026, 10, 10, 0, 0, 0).unwrap().to_utc();
    let probe = dvr();
    let importer = LibraryImporter::new(&probe, &store).folders_named(FolderPattern::default());
    importer.import_card(&card.card, &card.library, "Alpes", Some(day), |_| {}).unwrap();

    let cam_card = Fixture { library: card.library.clone(), ..Fixture::new(&GOPRO_CARD) };
    let cam = gopro();
    LibraryImporter::new(&cam, &store).folders_named(FolderPattern::default())
        .import_card(&cam_card.card, &cam_card.library, "Alpes", None, |_| {}).unwrap();

    assert!(card.library.join("2026-10-10 Alpes FATSHARK/PICT0030.AVI").is_file());
    assert!(card.library.join("2026-10-10 Alpes GOPRO/GH010042.MP4").is_file());
    assert_eq!(store.sessions().unwrap().len(), 1);
    // A second run finds them where the pattern put them: nothing is copied twice.
    let again = importer.import_card(&card.card, &card.library, "Alpes", Some(day), |_| {}).unwrap();
    assert!(again.imported.is_empty() && again.skipped.len() == 2);
}

#[test]
fn leaves_the_date_out_of_an_undated_dvr_session() {
    let card = Fixture::new(&["PICT0030.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let probe = dvr();
    LibraryImporter::new(&probe, &store).folders_named(FolderPattern::default())
        .import_card(&card.card, &card.library, "Alpes", None, |_| {}).unwrap();
    assert!(card.library.join("Alpes FATSHARK/PICT0030.AVI").is_file());
}

// Action cams

/// A GoPro card as the camera lays it out: two chapters of take 42, take 43,
/// and the preview and thumbnail files that must not be imported.
const GOPRO_CARD: [&str; 5] = ["DCIM/100GOPRO/GH010042.MP4", "DCIM/100GOPRO/GH020042.MP4",
                               "DCIM/100GOPRO/GH010043.MP4", "DCIM/100GOPRO/GL010042.LRV",
                               "DCIM/100GOPRO/GH010042.THM"];

fn gopro() -> StubProbe {
    StubProbe::new(&[("GH010042.MP4", "2026-08-23T10:00:00.000000Z"),
                     ("GH020042.MP4", "2026-08-23T10:00:00.000000Z"),
                     ("GH010043.MP4", "2026-08-23T10:20:00.000000Z")])
}

#[test]
fn turns_each_action_cam_take_into_one_flight_and_joins_its_chapters() {
    let fixture = Fixture::new(&GOPRO_CARD);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&gopro(), &store, "Letz Kwad");

    assert_eq!(report.imported.len(), 3); // no .LRV, no .THM
    assert!(report.failed.is_empty());
    let session = &report.sessions[0];
    let flights = store.flights(session.id).unwrap();
    assert_eq!(flights.len(), 2);

    // Take 42: both chapters, in order, the second a continuation — the same
    // shape as a flight the DVR cut across two files.
    let (first, clips) = &flights[0];
    assert_eq!(first.order_in_session, 1);
    let files: HashMap<_, _> = store.source_files(session.id).unwrap().into_iter()
        .map(|f| (f.id, file_name(Path::new(&f.imported_path)))).collect();
    assert_eq!(clips.iter().map(|c| files[&c.source_file_id].as_str()).collect::<Vec<_>>(),
               ["GH010042.MP4", "GH020042.MP4"]);
    assert_eq!(clips.iter().map(|c| c.entry).collect::<Vec<_>>(),
               [ClipEntry::Unknown, ClipEntry::Continuation]);
    // The whole file, every frame of it.
    assert!(clips.iter().all(|c| c.start_frame == 0 && c.end_frame == 3596));
    assert_eq!(flights[1].1.len(), 1);

    // And the run says no detector was involved.
    let runs = store.detection_runs(clips[0].source_file_id).unwrap();
    assert_eq!(runs[0].algorithm_version, WHOLE_FILE_ALGORITHM);
}

#[test]
fn dates_an_action_cam_session_from_its_first_take_in_local_time() {
    let fixture = Fixture::new(&GOPRO_CARD);
    let store = LibraryStore::in_memory().unwrap();
    let report = fixture.import(&gopro(), &store, "Letz Kwad");
    let date = report.sessions[0].date.unwrap().with_timezone(&Local);
    // GoPro wrote 10:00 local and called it UTC; read back, it is 10:00 here.
    assert_eq!(date.format("%Y-%m-%d %H:%M").to_string(), "2026-08-23 10:00");
}

#[test]
fn joins_the_dvr_session_of_the_same_name() {
    // The day's DVR card first, then the GoPro under the same name: one
    // session, the GoPro takes after the DVR's flights.
    let dvr_card = Fixture::new(&["PICT0030.AVI"]);
    let store = LibraryStore::in_memory().unwrap();
    let dvr_session = dvr_card.import(&dvr(), &store, "Letz Kwad").sessions[0].clone();
    let dvr_flight = Flight::new(dvr_session.id, 1);
    let file = &store.source_files(dvr_session.id).unwrap()[0];
    store.replace_clips(file.id, &[Clip::new(dvr_flight.id, file.id, 0, 100)], &[dvr_flight],
                        &DetectionRun::new(file.id, "test", BTreeMap::new())).unwrap();

    let cam = Fixture { library: dvr_card.library.clone(), ..Fixture::new(&GOPRO_CARD) };
    let report = cam.import(&gopro(), &store, "Letz Kwad");

    assert_eq!(store.sessions().unwrap().len(), 1);
    assert_eq!(report.sessions[0].id, dvr_session.id);
    let orders: Vec<_> = store.flights(dvr_session.id).unwrap().iter()
        .map(|(f, _)| f.order_in_session).collect();
    assert_eq!(orders, [1, 2, 3]);
    // The DVR session had no date; joining it does not invent one.
    assert_eq!(store.sessions().unwrap()[0].date, None);
}

#[test]
fn splits_action_cam_takes_hours_apart_into_sessions() {
    let fixture = Fixture::new(&["DCIM/100MEDIA/DJI_0001.MP4", "DCIM/100MEDIA/DJI_0002.MP4"]);
    let store = LibraryStore::in_memory().unwrap();
    let probe = StubProbe::new(&[("DJI_0001.MP4", "2026-08-23T09:00:00Z"),
                                 ("DJI_0002.MP4", "2026-08-23T17:00:00Z")]);
    let report = fixture.import(&probe, &store, "Letz Kwad");
    assert_eq!(report.sessions.len(), 2);
    assert_eq!(report.flights.len(), 2);
}

#[test]
fn reimporting_an_action_cam_card_adds_no_flights() {
    let fixture = Fixture::new(&GOPRO_CARD);
    let store = LibraryStore::in_memory().unwrap();
    fixture.import(&gopro(), &store, "Letz Kwad");
    let second = fixture.import(&gopro(), &store, "Letz Kwad");
    assert!(second.flights.is_empty());
    assert_eq!(store.flights(store.sessions().unwrap()[0].id).unwrap().len(), 2);
}
