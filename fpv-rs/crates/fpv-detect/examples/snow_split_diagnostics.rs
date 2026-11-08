//! Print sample-level noise evidence without exporting source pixels.
use fpv_core::DetectionProfile;
use fpv_detect::{DetectorSettings, FileDetector};
use fpv_media::FrameReader;
fn main() {
    let path = std::env::args_os().nth(1).expect("recording path");
    let result = FileDetector::new(
        DetectorSettings::for_profile(DetectionProfile::SnowSplit),
        FrameReader::default(),
    )
    .detect(std::path::Path::new(&path))
    .unwrap();
    println!("time_s,spatial_coverage,confirmed_coverage,frozen_coverage");
    for (time, frame) in result.times.iter().zip(result.metrics) {
        let e = frame.snow_split.unwrap();
        println!(
            "{time:.2},{:.6},{:.6},{:.6}",
            e.spatial_coverage(),
            e.confirmed_coverage(),
            e.frozen_coverage()
        );
    }
}
