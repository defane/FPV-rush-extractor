//! Snow Split semantics are independent of black/menu and whole-clip filters.
use fpv_core::{ClipExit, DetectionProfile};
use fpv_detect::{ClipDetector, DetectorSettings, FrameMetrics};

#[test]
fn snow_split_keeps_black_menu_short_and_incoherent_footage_without_snow() {
    let profile = DetectionProfile::from_raw("snowSplit").expect("Snow Split must be selectable");
    let detector = ClipDetector::new(DetectorSettings::for_profile(profile));
    let times: Vec<_> = (0..9).map(|i| i as f64 / 4.0).collect();
    let mut frames: Vec<_> = times
        .iter()
        .enumerate()
        .map(|(i, _)| match i % 3 {
            0 => FrameMetrics::new(0.0, 0.0, 0.0, 0.0, 1.0),
            1 => FrameMetrics::new(86.6, 1286.8, 1.52, 0.84, 0.125),
            _ => FrameMetrics::new(156.9, 222.2, 1.64, 0.97, 0.0),
        })
        .collect();
    for frame in &mut frames {
        frame.coherence = -0.5;
        frame.scene_correlation = -0.5;
    }
    let clips = detector.detect(&times, &frames);
    assert_eq!(clips.len(), 1);
    assert_eq!(
        (clips[0].start, clips[0].end, clips[0].exit),
        (0.0, 2.25, ClipExit::Eof)
    );
}
