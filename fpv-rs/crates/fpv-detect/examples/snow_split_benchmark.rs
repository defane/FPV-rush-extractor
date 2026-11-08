//! Evaluate clip retention against explicitly annotated intervals, not predictions.
//! Usage: snow_split_benchmark PROFILE VIDEO LABELS.tsv
use fpv_core::{ClipExit, DetectionProfile};
use fpv_detect::{ClipDetector, DetectorSettings, FileDetector};
use fpv_media::{FfmpegRunner, FrameReader, MediaProbe};
use std::{error::Error, path::Path, time::Instant};
struct Label {
    start: f64,
    end: f64,
    kind: String,
    split: String,
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: snow_split_benchmark PROFILE VIDEO LABELS.tsv".into());
    }
    let profile = DetectionProfile::from_raw(&args[1]).ok_or("unknown profile")?;
    let path = Path::new(&args[2]);
    let name = path
        .file_name()
        .ok_or("missing file name")?
        .to_string_lossy();
    let mut labels = Vec::new();
    for (index, line) in std::fs::read_to_string(&args[3])?.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cells: Vec<_> = line.split('\t').collect();
        if cells.len() != 5 {
            return Err(format!("invalid annotation row {}", index + 1).into());
        }
        if cells[0] != name {
            continue;
        }
        if !["usable", "snow", "loss", "recovery"].contains(&cells[3]) {
            return Err("unknown annotation kind".into());
        }
        let start: f64 = cells[1].parse()?;
        let end: f64 = cells[2].parse()?;
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
            return Err("invalid annotation range".into());
        }
        labels.push(Label {
            start,
            end,
            kind: cells[3].into(),
            split: cells[4].into(),
        });
    }
    if labels.is_empty() {
        return Err("no annotations for this recording".into());
    }
    for (i, a) in labels.iter().enumerate().filter(|(_, a)| a.end > a.start) {
        if labels
            .iter()
            .skip(i + 1)
            .any(|b| b.end > b.start && a.start < b.end && b.start < a.end)
        {
            return Err("overlapping annotations".into());
        }
    }
    let duration = MediaProbe::new(&FfmpegRunner::default())
        .probe(path)?
        .duration;
    let settings = DetectorSettings::for_profile(profile);
    let step = 1.0 / settings.sample_rate;
    let start = Instant::now();
    let result = FileDetector::new(settings.clone(), FrameReader::default()).detect(path)?;
    let elapsed = start.elapsed().as_secs_f64();
    let classifier = ClipDetector::new(settings);
    let mut tp = 0.0;
    let mut fp = 0.0;
    let mut tn = 0.0;
    let mut fn_ = 0.0;
    let mut raw_tp = 0.0;
    let mut raw_fp = 0.0;
    let mut raw_fn = 0.0;
    for (t, metrics) in result.times.iter().zip(&result.metrics) {
        let end = (*t + step).min(duration);
        let Some(truth) = labels.iter().find(|a| {
            (a.kind == "usable" || a.kind == "snow") && a.start <= *t && end <= a.end + 1e-9
        }) else {
            continue;
        };
        let weight = end - t;
        let snow = truth.kind == "snow";
        let excluded = !result
            .clips
            .iter()
            .any(|c| c.start <= *t && *t < if c.at_file_end { duration } else { c.end });
        match (snow, excluded) {
            (true, true) => tp += weight,
            (true, false) => fn_ += weight,
            (false, true) => fp += weight,
            (false, false) => tn += weight,
        }
        let raw = classifier.classify(metrics) == fpv_detect::FrameState::NoSignal;
        match (snow, raw) {
            (true, true) => raw_tp += weight,
            (true, false) => raw_fn += weight,
            (false, true) => raw_fp += weight,
            _ => {}
        }
    }
    let false_cuts = result
        .clips
        .iter()
        .filter(|c| {
            c.exit == ClipExit::SignalLoss
                && labels
                    .iter()
                    .any(|a| a.kind == "usable" && a.start < c.end && c.end < a.end)
        })
        .count();
    let mut errors = Vec::new();
    let mut missing = 0;
    for a in labels
        .iter()
        .filter(|a| a.kind == "loss" || a.kind == "recovery")
    {
        let error = result
            .clips
            .iter()
            .filter_map(|c| {
                if a.kind == "loss" && c.exit == ClipExit::SignalLoss {
                    Some((c.end - a.start).abs())
                } else if a.kind == "recovery" && !c.at_file_start {
                    Some((c.start - a.start).abs())
                } else {
                    None
                }
            })
            .min_by(f64::total_cmp);
        if let Some(error) = error {
            errors.push(error)
        } else {
            missing += 1;
        }
    }
    let ratio = |a: f64, b: f64| {
        if b > 0.0 {
            format!("{:.6}", a / b)
        } else {
            "n/a".into()
        }
    };
    println!(
        "recording\tprofile\tsplit\tlabeled_s\tunknown_s\tusable_kept_s\tusable_lost_s\tsnow_kept_s\tprecision\trecall\traw_noise_precision\traw_noise_recall\tfalse_cuts\tboundary_mean_abs_s\tmissing_boundaries\tdecode_analyze_s"
    );
    println!(
        "{}\t{}\t{}\t{:.3}\t{:.3}\t{tn:.3}\t{fp:.3}\t{fn_:.3}\t{}\t{}\t{}\t{}\t{false_cuts}\t{}\t{missing}\t{elapsed:.3}",
        name,
        args[1],
        labels[0].split,
        tp + fp + tn + fn_,
        duration - tp - fp - tn - fn_,
        ratio(tp, tp + fp),
        ratio(tp, tp + fn_),
        ratio(raw_tp, raw_tp + raw_fp),
        ratio(raw_tp, raw_tp + raw_fn),
        ratio(errors.iter().sum(), errors.len() as f64)
    );
    for c in result.clips {
        eprintln!(
            "clip\t{:.3}\t{:.3}\t{:?}",
            c.start,
            if c.at_file_end { duration } else { c.end },
            c.exit
        );
    }
    Ok(())
}
