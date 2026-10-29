//! What the front end displays, flattened and serialisable. Seconds are
//! computed here, through each file's own rate, so the UI never does frame
//! arithmetic — the one place it could quietly assume 30 fps (CLAUDE.md 2).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use fpv_core::{Clip, FigureTag, Flight, FlightTimeline, MediaInfo, Session, SourceFile,
               uuid_string};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub name: String,
    /// RFC 3339; the UI shows the local day.
    pub date: Option<String>,
    pub files: Vec<FileView>,
    pub flights: Vec<FlightView>,
    /// Whether segmentation has anything to do here: DVR recordings not yet
    /// reduced to their flights.
    pub segmentable: bool,
    /// Whether "Reduce…" has anything to offer: such recordings, and flights
    /// found in them.
    pub reducible: bool,
    /// `standard` or `textured`: the detector thresholds for its DVR footage.
    pub detection_profile: String,
    pub location: Option<String>,
    pub notes: Option<String>,
    /// Tags on the whole session: every flight of it carries them.
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    pub id: String,
    pub name: String,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
    pub duration: f64,
    pub codec: String,
    pub action_cam: bool,
    /// `DVR`, or the camera: "GoPro HERO11", "DJI"… (`fpv_import::source_name`).
    pub source: String,
    /// "4K · HEVC · 59.94 fps" (`format_label`).
    pub format: String,
    /// A piece of a DVR recording cut down to its flights ("Reduce…").
    pub reduced: bool,
    /// The folder it sits in when that is outside the current import folder —
    /// an earlier one, before the setting changed. `None` inside it.
    pub elsewhere: Option<String>,
    /// Only in the cloud drive the import folder sits in (Proton Drive made
    /// it Online Only): reading it would download it first.
    pub cloud: bool,
    /// On this disk in a cloud drive's folder: "Free Up Space" can make it
    /// Online Only.
    pub freeable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightView {
    pub id: String,
    pub order: i64,
    pub title: Option<String>,
    pub liked: bool,
    pub duration: f64,
    pub clips: Vec<ClipView>,
    /// How many recordings it spans: the DVR's cuts and a GoPro's chapters.
    pub recordings: usize,
    pub action_cam: bool,
    /// What recorded it, as on the Files tab: its first recording's.
    pub source: String,
    /// Its recordings' `format_label`, each once: two for a DVR flight whose
    /// parts were captured at different rates.
    pub format: String,
    /// Its DVR footage was cut down to its flights: the snow around it is gone.
    pub reduced: bool,
    /// A recording of it is only in the cloud: playing it downloads it first.
    pub cloud: bool,
    /// A recording of it could be made Online Only.
    pub freeable: bool,
    /// Changes whenever a boundary does: what cached strips are keyed on.
    pub cache_key: String,
    /// The pilot's free labels on the whole flight: "freestyle", "gates".
    pub labels: Vec<String>,
    /// Its figure tags on the flight's clock, in time order.
    pub figures: Vec<TagView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: String,
    pub file_id: String,
    pub file_name: String,
    pub start_frame: i64,
    pub end_frame: i64,
    pub fps: f64,
    /// The last frame index the recording has, the editor's upper bound.
    pub last_frame: i64,
    pub file_duration: f64,
    pub entry: String,
    pub exit: String,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagView {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
    /// Flight seconds, clipped to the flight.
    pub start: f64,
    pub end: f64,
    pub marker: bool,
}

pub(crate) fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub(crate) fn file_view(file: &SourceFile) -> FileView {
    FileView {
        id: uuid_string(file.id),
        name: file_name(&file.imported_path),
        width: file.media.width,
        height: file.media.height,
        fps: file.media.frame_rate.value(),
        duration: file.media.duration,
        codec: file.media.video_codec.clone(),
        action_cam: !file.media.is_analog_dvr(),
        source: source(file),
        format: format_label(&file.media),
        reduced: false,
        elsewhere: None,
        cloud: false,
        freeable: false,
    }
}

/// What a recording is, the way a pilot says it: "4K · HEVC · 59.94 fps",
/// "480p · MJPEG · 29.89 fps". The rate keeps two decimals because they
/// matter — 59.94 is not 60, and the DVR records its own odd rate (CLAUDE.md 2).
pub(crate) fn format_label(media: &MediaInfo) -> String {
    let (width, height) = (media.width, media.height);
    let size = if width >= 5000 {
        format!("{:.1}K", width as f64 / 1000.0)
    } else if width >= 3800 {
        "4K".into()
    } else if width >= 2600 {
        "2.7K".into()
    } else {
        format!("{height}p")
    };
    let codec = match media.video_codec.as_str() {
        "mjpeg" => "MJPEG".into(),
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "prores" => "ProRes".into(),
        other => other.to_uppercase(),
    };
    let rate = format!("{:.2}", media.frame_rate.value());
    let rate = rate.trim_end_matches('0').trim_end_matches('.');
    format!("{size} · {codec} · {rate} fps")
}

fn source(file: &SourceFile) -> String {
    fpv_import::source_name(&fpv_import::DiscoveredFile::new(&file.imported_path), &file.media)
}

/// The folder of a recording that is not under `root`, or `None`.
pub(crate) fn outside(root: &Path, path: &str) -> Option<String> {
    let path = Path::new(path);
    (!path.starts_with(root)).then(|| path.parent().unwrap_or(path).to_string_lossy().into_owned())
}

pub(crate) fn last_frame(file: &SourceFile) -> i64 {
    file.media.frame_count.filter(|&n| n > 0).map(|n| n - 1)
        .unwrap_or_else(|| file.media.frame_rate.frame_index(file.media.duration))
}

pub(crate) fn flight_view(flight: &Flight, clips: &[Clip], files: &HashMap<Uuid, SourceFile>,
                          labels: Vec<String>, tags: &[FigureTag]) -> FlightView {
    let parts: Vec<(Clip, SourceFile)> = clips.iter()
        .filter_map(|clip| files.get(&clip.source_file_id).map(|f| (clip.clone(), f.clone())))
        .collect();
    let timeline = FlightTimeline::new(parts.clone());
    let mut figures: Vec<TagView> = tags.iter().filter_map(|tag| tag_view(tag, &timeline)).collect();
    figures.sort_by(|a, b| a.start.total_cmp(&b.start));
    FlightView {
        reduced: false,
        cloud: false,
        freeable: false,
        labels,
        figures,
        id: uuid_string(flight.id),
        order: flight.order_in_session,
        title: flight.title.clone(),
        liked: flight.rating.is_some(),
        duration: timeline.duration(),
        recordings: parts.len(),
        action_cam: parts.iter().any(|(_, f)| !f.media.is_analog_dvr()),
        source: parts.first().map(|(_, f)| source(f)).unwrap_or_default(),
        format: {
            let mut labels: Vec<String> = Vec::new();
            for (_, file) in &parts {
                let label = format_label(&file.media);
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            labels.join(" / ")
        },
        cache_key: Flight::cache_key(flight.id, clips),
        clips: parts.iter().map(|(clip, file)| ClipView {
            id: uuid_string(clip.id),
            file_id: uuid_string(file.id),
            file_name: file_name(&file.imported_path),
            start_frame: clip.start_frame,
            end_frame: clip.end_frame,
            fps: file.media.frame_rate.value(),
            last_frame: last_frame(file),
            file_duration: file.media.duration,
            entry: clip.entry.as_str().into(),
            exit: clip.exit.as_str().into(),
            // Only a poster that is there: the cache may have been emptied, and
            // the UI then asks `poster` to remake it.
            thumbnail: clip.thumbnail_path.clone().filter(|p| Path::new(p).is_file()),
        }).collect(),
    }
}

/// What a session's view needs beside its own rows.
pub(crate) struct SessionContext<'a> {
    /// The current import folder: recordings outside it are "elsewhere".
    pub root: &'a Path,
    pub session_labels: Vec<String>,
    pub flight_labels: HashMap<Uuid, Vec<String>>,
    pub tags: &'a [FigureTag],
    pub reduced: &'a HashSet<Uuid>,
    /// Recordings only in the cloud (`Library::in_cloud`).
    pub cloud: &'a HashSet<Uuid>,
    /// Recordings whose space a cloud drive could free.
    pub freeable: &'a HashSet<Uuid>,
}

pub(crate) fn session_view(session: &Session, files: Vec<SourceFile>, flights: Vec<(Flight, Vec<Clip>)>,
                           context: SessionContext) -> SessionView {
    let SessionContext { root, session_labels, flight_labels: mut labels, tags, reduced, cloud, freeable } = context;
    let segmentable = files.iter().any(|f| f.media.needs_segmentation() && !reduced.contains(&f.id));
    let by_id: HashMap<Uuid, SourceFile> = files.iter().map(|f| (f.id, f.clone())).collect();
    SessionView {
        id: uuid_string(session.id),
        name: session.name.clone(),
        date: session.date.map(|d| d.to_rfc3339()),
        segmentable,
        reducible: segmentable && flights.iter().flat_map(|(_, clips)| clips)
            .any(|clip| by_id.get(&clip.source_file_id)
                .is_some_and(|f| f.media.is_all_intra() && !reduced.contains(&f.id))),
        detection_profile: session.detection_profile.as_str().into(),
        location: session.location.clone(),
        notes: session.notes.clone(),
        labels: session_labels,
        files: files.iter()
            .map(|f| FileView {
                reduced: reduced.contains(&f.id),
                elsewhere: outside(root, &f.imported_path),
                cloud: cloud.contains(&f.id),
                freeable: freeable.contains(&f.id),
                ..file_view(f)
            })
            .collect(),
        flights: flights.iter()
            .map(|(flight, clips)| FlightView {
                reduced: clips.iter().any(|c| reduced.contains(&c.source_file_id)),
                cloud: clips.iter().any(|c| cloud.contains(&c.source_file_id)),
                freeable: clips.iter().any(|c| freeable.contains(&c.source_file_id)),
                ..flight_view(flight, clips, &by_id, labels.remove(&flight.id).unwrap_or_default(), tags)
            })
            .collect(),
    }
}

pub(crate) fn tag_view(tag: &FigureTag, timeline: &FlightTimeline) -> Option<TagView> {
    let range = timeline.flight_time_range(tag)?;
    Some(TagView {
        id: uuid_string(tag.id),
        label: tag.label.clone(),
        note: tag.note.clone(),
        start: *range.start(),
        end: *range.end(),
        marker: tag.is_marker(),
    })
}

#[cfg(test)]
mod tests {
    use fpv_core::FrameRate;

    use super::*;

    fn label(width: i64, height: i64, rate: FrameRate, codec: &str) -> String {
        format_label(&MediaInfo::new(width, height, rate, 1.0, codec))
    }

    #[test]
    fn says_what_a_recording_is_the_way_a_pilot_does() {
        // The pilot's Osmo Nano take, and the DVR's own rate from PICT0030.
        assert_eq!(label(3840, 2160, FrameRate::new(60000, 1001), "hevc"), "4K · HEVC · 59.94 fps");
        assert_eq!(label(640, 480, FrameRate::new(1_000_000, 33459), "mjpeg"), "480p · MJPEG · 29.89 fps");
        assert_eq!(label(1920, 1080, FrameRate::new(60, 1), "h264"), "1080p · H.264 · 60 fps");
        assert_eq!(label(1280, 720, FrameRate::new(30000, 1001), "h264"), "720p · H.264 · 29.97 fps");
        assert_eq!(label(2704, 1520, FrameRate::new(25, 1), "h264"), "2.7K · H.264 · 25 fps");
        assert_eq!(label(5312, 2988, FrameRate::new(24000, 1001), "hevc"), "5.3K · HEVC · 23.98 fps");
        assert_eq!(label(1920, 1080, FrameRate::new(24, 1), "av1"), "1080p · AV1 · 24 fps");
    }
}
