use std::collections::{BTreeMap, HashMap};

use fpv_core::{ClipExit, FrameRate, MediaInfo, Session};
use fpv_detect::DetectError;
use fpv_media::{FfmpegError, FfmpegTool, ProcessResult};

use super::*;

struct StubDetector(HashMap<&'static str, Vec<DetectedClip>>);

impl ClipDetecting for StubDetector {
    fn clips(&self, path: &Path) -> Result<Vec<DetectedClip>, DetectError> {
        let name = path.file_name().unwrap().to_str().unwrap();
        if name == "BROKEN.AVI" {
            return Err(FfmpegError::UnreadableOutput { tool: "ffmpeg".into(),
                                                       detail: "broken".into() }.into());
        }
        self.0.get(name).cloned().ok_or_else(|| FfmpegError::UnreadableOutput {
            tool: "ffmpeg".into(), detail: format!("no fixture for {name}") }.into())
    }
}

/// ffmpeg for the posters: writes the requested file and succeeds.
struct SilentFfmpeg;

impl FfmpegInvoking for SilentFfmpeg {
    fn run(&self, _: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
        if let Some(index) = arguments.iter().rposition(|a| a == "-y") {
            std::fs::write(&arguments[index + 1], b"j").unwrap();
        }
        Ok(ProcessResult { status: 0, stdout: vec![], stderr: String::new() })
    }
}

fn clip(start: f64, end: f64, exit: ClipExit, at_start: bool, at_end: bool) -> DetectedClip {
    DetectedClip { start, end, exit, entry: ClipEntry::Unknown, confidence: 0.9, coherence: 1.0,
                   at_file_start: at_start, at_file_end: at_end }
}

fn source(session: &Session, name: &str, sequence: i64, rate: FrameRate, codec: &str) -> SourceFile {
    SourceFile::new(session.id, format!("/card/{name}"), format!("/library/{name}"),
                    Some(sequence), MediaInfo::new(640, 480, rate, 600.0, codec))
}

struct Fixture {
    store: LibraryStore,
    session: Session,
    thumbnails: tempfile::TempDir,
}

impl Fixture {
    fn new(files: &[SourceFile], session: Session) -> Self {
        let store = LibraryStore::in_memory().unwrap();
        store.save_session(&session).unwrap();
        store.save_source_files(files).unwrap();
        Self { store, session, thumbnails: tempfile::tempdir().unwrap() }
    }

    fn segment(&self, detected: &[(&'static str, Vec<DetectedClip>)]) -> SegmentationReport {
        let detector = StubDetector(detected.iter().cloned().collect());
        SegmentationService::new(&detector, &SilentFfmpeg, &self.store, DetectorSettings::default())
            .segment(self.session.id, self.thumbnails.path(), |_| {})
            .unwrap()
    }
}

#[test]
fn converts_seconds_to_frames_through_the_files_own_rate() {
    let session = Session::new("Letz Kwad", None);
    // 24.978 fps. At a nominal 25 the same second is a different frame.
    let rate = FrameRate::new(250_000, 10009);
    let fixture = Fixture::new(&[source(&session, "PICT0032.AVI", 32, rate, "mjpeg")], session);
    fixture.segment(&[("PICT0032.AVI", vec![clip(346.25, 443.5, ClipExit::SignalLoss, false, false)])]);
    let saved = &fixture.store.flights(fixture.session.id).unwrap()[0].1[0];
    assert_eq!(saved.start_frame, 8648);
    assert_eq!(saved.end_frame, rate.frame_index(443.5));
    assert!(saved.thumbnail_path.is_some());
}

#[test]
fn makes_one_flight_out_of_a_recording_the_dvr_cut_in_two() {
    let session = Session::new("Letz Kwad", None);
    let first = source(&session, "PICT0030.AVI", 30, FrameRate::new(1_000_000, 33459), "mjpeg");
    let second = source(&session, "PICT0031.AVI", 31, FrameRate::new(1_000_000, 40839), "mjpeg");
    let fixture = Fixture::new(&[first.clone(), second.clone()], session);
    let report = fixture.segment(&[
        ("PICT0030.AVI", vec![clip(140.3, 141.1, ClipExit::Eof, false, true)]),
        ("PICT0031.AVI", vec![clip(0.0, 60.25, ClipExit::Black, true, false)]),
    ]);
    assert_eq!(report.stitched.len(), 1);
    let flights = fixture.store.flights(fixture.session.id).unwrap();
    assert_eq!(flights.len(), 1);
    let clips = &flights[0].1;
    assert_eq!(clips.iter().map(|c| c.source_file_id).collect::<Vec<_>>(), [first.id, second.id]);
    assert_eq!(clips[1].entry, ClipEntry::Continuation);
}

#[test]
fn keeps_separate_flights_separate_within_one_file() {
    let session = Session::new("Letz Kwad", None);
    let fixture = Fixture::new(
        &[source(&session, "PICT0034.AVI", 34, FrameRate::new(1_000_000, 40323), "mjpeg")], session);
    let report = fixture.segment(&[("PICT0034.AVI", vec![
        clip(0.0, 146.5, ClipExit::SignalLoss, true, false),
        clip(179.0, 275.5, ClipExit::SignalLoss, false, false),
        clip(281.0, 303.75, ClipExit::SignalLoss, false, false),
    ])]);
    assert_eq!((report.flights, report.clips), (3, 3));
    let flights = fixture.store.flights(fixture.session.id).unwrap();
    assert_eq!(flights.iter().map(|(f, _)| f.order_in_session).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn records_the_settings_that_produced_the_clips() {
    let session = Session::new("Letz Kwad", None);
    let file = source(&session, "PICT0033.AVI", 33, FrameRate::new(15625, 627), "mjpeg");
    let fixture = Fixture::new(std::slice::from_ref(&file), session);
    fixture.segment(&[("PICT0033.AVI", vec![clip(0.0, 165.25, ClipExit::Eof, true, false)])]);
    let run = &fixture.store.detection_runs(file.id).unwrap()[0];
    assert_eq!(run.algorithm_version, ALGORITHM_VERSION);
    assert_eq!(run.parameters["snowBlockVariation"], 0.60);
    assert_eq!(run.parameters["minimumClipCoherence"], 0.50);
}

#[test]
fn carries_on_past_a_file_it_cannot_decode() {
    let session = Session::new("Letz Kwad", None);
    let rate = FrameRate::new(30, 1);
    let fixture = Fixture::new(&[source(&session, "PICT0030.AVI", 30, rate, "mjpeg"),
                                 source(&session, "BROKEN.AVI", 31, rate, "mjpeg"),
                                 source(&session, "PICT0032.AVI", 32, rate, "mjpeg")], session);
    let whole = vec![clip(0.0, 100.0, ClipExit::Eof, true, false)];
    let report = fixture.segment(&[("PICT0030.AVI", whole.clone()), ("PICT0032.AVI", whole)]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.clips, 2);
}

/// A session holding DVR files and one GoPro take imported as a whole-file
/// flight, numbered 2 after the DVR's single flight.
fn mixed_session() -> (Fixture, SourceFile) {
    let session = Session::new("Letz Kwad", None);
    let dvr = source(&session, "PICT0034.AVI", 34, FrameRate::new(1_000_000, 40323), "mjpeg");
    let cam = source(&session, "GH010042.MP4", 4201, FrameRate::new(60000, 1001), "h264");
    let fixture = Fixture::new(&[dvr, cam.clone()], session);
    let take = Flight::new(fixture.session.id, 2);
    fixture.store.replace_clips(cam.id, &[Clip::new(take.id, cam.id, 0, 3596)], &[take],
                                &DetectionRun::new(cam.id, "whole-file-1", BTreeMap::new()))
        .unwrap();
    (fixture, cam)
}

#[test]
fn segments_action_cam_codecs_instead_of_preserving_whole_file_placeholders() {
    let (fixture, cam) = mixed_session();
    let report = fixture.segment(&[
        ("PICT0034.AVI", vec![clip(0.0, 100.0, ClipExit::SignalLoss, true, false)]),
        ("GH010042.MP4", vec![clip(0.0, 20.0, ClipExit::SignalLoss, true, false),
                              clip(40.0, 59.0, ClipExit::Eof, false, true)]),
    ]);
    assert_eq!(report.whole_file, 0);
    assert!(report.failed.is_empty());
    let flights = fixture.store.flights(fixture.session.id).unwrap();
    let parts: Vec<_> = flights.iter().flat_map(|(_, clips)| clips)
        .filter(|clip| clip.source_file_id == cam.id).collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].end_frame, cam.media.frame_rate.frame_index(20.0));
    assert_eq!(parts[1].start_frame, cam.media.frame_rate.frame_index(40.0));
    assert!(fixture.store.detection_runs(cam.id).unwrap().iter()
        .any(|run| run.algorithm_version == ALGORITHM_VERSION));
}

#[test]
fn leaves_a_dvr_recording_reduced_to_its_flights_alone() {
    let session = Session::new("Letz Kwad", None);
    let piece = source(&session, "PICT0032-1.AVI", 32, FrameRate::new(250_000, 10009), "mjpeg");
    let fixture = Fixture::new(std::slice::from_ref(&piece), session);
    let flight = Flight { title: Some("Rock gap".into()), ..Flight::new(fixture.session.id, 1) };
    fixture.store.replace_clips(piece.id, &[Clip::new(flight.id, piece.id, 0, 1200)], std::slice::from_ref(&flight),
                                &DetectionRun::new(piece.id, fpv_core::REDUCED_ALGORITHM, BTreeMap::new()))
        .unwrap();
    // The detector would cut it in two; it is never asked.
    let report = fixture.segment(&[("PICT0032-1.AVI", vec![clip(0.0, 10.0, ClipExit::SignalLoss, true, false),
                                                          clip(20.0, 40.0, ClipExit::Eof, false, true)])]);
    assert_eq!(report.whole_file, 1);
    let flights = fixture.store.flights(fixture.session.id).unwrap();
    assert_eq!(flights.len(), 1);
    assert_eq!(flights[0].0.title.as_deref(), Some("Rock gap"));
}

#[test]
fn preserves_and_renumbers_a_recording_whose_detector_failed() {
    // The DVR file now yields three flights. Numbered from 1, they would
    // collide with the take at 2 unless it moves after them.
    let (fixture, cam) = mixed_session();
    fixture.segment(&[("PICT0034.AVI", vec![
        clip(0.0, 100.0, ClipExit::SignalLoss, true, false),
        clip(200.0, 300.0, ClipExit::SignalLoss, false, false),
        clip(400.0, 500.0, ClipExit::SignalLoss, false, false),
    ])]);
    let flights = fixture.store.flights(fixture.session.id).unwrap();
    assert_eq!(flights.iter().map(|(f, _)| f.order_in_session).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(flights[3].1[0].source_file_id, cam.id);
}

#[test]
fn clean_tail_keeps_the_final_source_frames_beyond_the_sampling_grid() {
    let session = Session::new("Indoor", None);
    let mut file = source(&session, "Indoor.mp4", 1, FrameRate::new(30, 1), "h264");
    file.media.duration = 600.067;
    file.media.frame_count = Some(18_002);
    let detected = clip(218.25, 599.75, ClipExit::Eof, false, true);
    let saved = super::clip(&detected, &file, Uuid::new_v4(), false);
    assert_eq!(saved.start_frame, 6548);
    assert_eq!(saved.end_frame, 18_002);
}
