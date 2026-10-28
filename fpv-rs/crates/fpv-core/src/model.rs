use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::MediaInfo;

/// A UUID as Foundation's `uuidString` writes it: upper-case. Every file name
/// derived from an ID goes through this, so a library the Swift app built keeps
/// its thumbnails, proxies and filmstrips.
pub fn uuid_string(id: Uuid) -> String {
    id.hyphenated().encode_upper(&mut Uuid::encode_buffer()).to_owned()
}

/// A flying session: one card import, or one outing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    /// Supplied by the user at import, or read off the card folder name. The
    /// analog DVR has no clock, so this cannot be derived from the files. An
    /// instant, as Foundation's `Date` is: the day it names is read in local time.
    pub date: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub notes: Option<String>,
    /// Which detector thresholds segment this session's DVR footage.
    pub detection_profile: DetectionProfile,
}

impl Session {
    pub fn new(name: impl Into<String>, date: Option<DateTime<Utc>>) -> Self {
        Self {
            id: Uuid::new_v4(), name: name.into(), date, location: None, notes: None,
            detection_profile: DetectionProfile::Standard,
        }
    }
}

/// The detector's thresholds were tuned on one goggle/VTX chain (the Letz Kwad
/// reference card, CLAUDE.md 10), and another chain can sit elsewhere: on the
/// pilot's Alps footage a clean picture of a rock face has a vertical
/// correlation around 0.87, under the standard snow threshold of 0.92, and the
/// flight shatters into clips. A session picks the profile its footage needs;
/// `Standard` is the tuned default and never changes behind its back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectionProfile {
    #[default]
    Standard,
    /// Footage whose clean picture has a low vertical correlation — fine,
    /// uniform terrain filling the frame, or a softer camera chain.
    Textured,
}

/// One recording as it came off the card, copied verbatim and never modified.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: Uuid,
    pub session_id: Uuid,
    pub original_path: String,
    pub imported_path: String,
    /// `PICT0030.AVI` -> 30. The only reliable ordering: the DVR has no RTC and
    /// its mtimes read 1970, 1980 or worse (CLAUDE.md 10).
    pub sequence_number: Option<i64>,
    pub media: MediaInfo,
    pub checksum: Option<String>,
    /// Silence this recording during playback/export while keeping its bytes intact.
    #[serde(default)]
    pub muted: bool,
}

impl SourceFile {
    pub fn new(session_id: Uuid, original_path: impl Into<String>,
               imported_path: impl Into<String>, sequence_number: Option<i64>,
               media: MediaInfo) -> Self {
        Self {
            id: Uuid::new_v4(), session_id, original_path: original_path.into(),
            imported_path: imported_path.into(), sequence_number, media, checksum: None, muted: false,
        }
    }
}

/// What the user browses and exports. A flight is one or more clips, because the
/// DVR cuts a recording mid-flight and the feed continues in the next file — it
/// does so in five of the twelve reference files (CLAUDE.md 3.3, 5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flight {
    pub id: Uuid,
    pub session_id: Uuid,
    pub order_in_session: i64,
    pub title: Option<String>,
    pub rating: Option<i64>,
}

impl Flight {
    pub fn new(session_id: Uuid, order_in_session: i64) -> Self {
        Self { id: Uuid::new_v4(), session_id, order_in_session, title: None, rating: None }
    }

    /// The flight's clip boundaries, `"4194-4217_0-1475"`: what every cached
    /// derivative of a flight is keyed on, so one boundary edit invalidates the
    /// hover filmstrip and the playback proxy together.
    pub fn bounds_key(clips: &[Clip]) -> String {
        clips.iter()
            .map(|clip| format!("{}-{}", clip.start_frame, clip.end_frame))
            .collect::<Vec<_>>()
            .join("_")
    }

    /// `"<flight ID>-<bounds>"`. The ID alone would not do: re-segmenting or a
    /// boundary edit changes what the flight shows without changing who it is.
    pub fn cache_key(id: Uuid, clips: &[Clip]) -> String {
        format!("{}-{}", uuid_string(id), Self::bounds_key(clips))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipKind {
    Flight,
    Unknown,
    Discarded,
}

/// How the clip opened. Set from the OSD where one is legible, `Unknown`
/// otherwise — it never moves a boundary and never lowers confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipEntry {
    Poweron,
    Midflight,
    FeedChange,
    Continuation,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipExit {
    SignalLoss,
    Black,
    Menu,
    FeedChange,
    Interference,
    Eof,
}

/// The raw values the Swift enums store, which the database keeps verbatim.
macro_rules! raw_values {
    ($type:ty { $($case:ident => $raw:literal),* $(,)? }) => {
        impl $type {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$case => $raw),* }
            }
            pub fn from_raw(raw: &str) -> Option<Self> {
                match raw { $($raw => Some(Self::$case),)* _ => None }
            }
        }
    };
}
raw_values!(ClipKind { Flight => "flight", Unknown => "unknown", Discarded => "discarded" });
raw_values!(DetectionProfile { Standard => "standard", Textured => "textured" });
raw_values!(ClipEntry {
    Poweron => "poweron", Midflight => "midflight", FeedChange => "feedChange",
    Continuation => "continuation", Unknown => "unknown",
});
raw_values!(ClipExit {
    SignalLoss => "signalLoss", Black => "black", Menu => "menu", FeedChange => "feedChange",
    Interference => "interference", Eof => "eof",
});

/// One file's share of a flight: a virtual cut, never a new file on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: Uuid,
    pub flight_id: Uuid,
    pub source_file_id: Uuid,
    pub start_frame: i64,
    pub end_frame: i64,
    pub kind: ClipKind,
    pub entry: ClipEntry,
    pub exit: ClipExit,
    pub confidence: f64,
    pub craft_name: Option<String>,
    pub thumbnail_path: Option<String>,
}

impl Clip {
    /// A flight clip with the Swift initialiser's defaults.
    pub fn new(flight_id: Uuid, source_file_id: Uuid, start_frame: i64, end_frame: i64) -> Self {
        Self {
            id: Uuid::new_v4(), flight_id, source_file_id, start_frame, end_frame,
            kind: ClipKind::Flight, entry: ClipEntry::Unknown, exit: ClipExit::SignalLoss,
            confidence: 0.9, craft_name: None, thumbnail_path: None,
        }
    }
}

/// A labelled range the pilot marks while watching a flight — "split-S here".
///
/// Owned by the recording, not by a clip, and addressed in that recording's own
/// frames. Both are structural: a frame index only means something through one
/// file's rate, and a flight spanning two files has no single rate (CLAUDE.md 2);
/// and `replace_clips` deletes a file's clips on every re-segmentation, which a
/// tag hanging off a clip would not survive. The clip a tag falls in is resolved
/// when it is read, never stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FigureTag {
    pub id: Uuid,
    pub source_file_id: Uuid,
    pub start_frame: i64,
    /// Equal to `start_frame` for a point marker.
    pub end_frame: i64,
    pub label: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl FigureTag {
    /// The two ends may come in either order.
    pub fn new(source_file_id: Uuid, start_frame: i64, end_frame: i64,
               label: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(), source_file_id,
            start_frame: start_frame.min(end_frame), end_frame: start_frame.max(end_frame),
            label: label.into(), note: None, created_at: Utc::now(),
        }
    }

    pub fn is_marker(&self) -> bool {
        self.start_frame == self.end_frame
    }
}

/// The parameters and algorithm version that produced a clip set, so results stay
/// reproducible and re-runnable after the detector changes.
/// The run recorded for a piece of a reduced DVR recording: cut down to its
/// flights, the snow between them gone. Segmentation leaves such a piece
/// alone — there is nothing left to find, and redoing it would drop the
/// pilot's titles, likes and tags for nothing.
pub const REDUCED_ALGORITHM: &str = "reduced-1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionRun {
    pub id: Uuid,
    pub source_file_id: Uuid,
    pub algorithm_version: String,
    pub parameters: BTreeMap<String, f64>,
    pub ran_at: DateTime<Utc>,
}

impl DetectionRun {
    pub fn new(source_file_id: Uuid, algorithm_version: impl Into<String>,
               parameters: BTreeMap<String, f64>) -> Self {
        Self {
            id: Uuid::new_v4(), source_file_id, algorithm_version: algorithm_version.into(),
            parameters, ran_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_a_flight_on_its_boundaries_in_foundation_case() {
        let id = Uuid::parse_str("6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b").unwrap();
        let clips = [Clip::new(id, Uuid::new_v4(), 4194, 4217), Clip::new(id, Uuid::new_v4(), 0, 1475)];
        assert_eq!(Flight::cache_key(id, &clips),
                   "6F1C2A3B-4D5E-4F60-8A7B-9C0D1E2F3A4B-4194-4217_0-1475");
    }

    #[test]
    fn keeps_the_swift_raw_values() {
        assert_eq!(ClipExit::SignalLoss.as_str(), "signalLoss");
        assert_eq!(ClipEntry::from_raw("feedChange"), Some(ClipEntry::FeedChange));
        assert_eq!(ClipEntry::Poweron.as_str(), "poweron");
    }

    #[test]
    fn a_new_session_uses_the_tuned_detector() {
        let session = Session::new("Alpes", None);
        assert_eq!(session.detection_profile, DetectionProfile::Standard);
        assert_eq!(DetectionProfile::default(), DetectionProfile::Standard);
        // The stored names, which the database and the UI both speak.
        for profile in [DetectionProfile::Standard, DetectionProfile::Textured] {
            assert_eq!(DetectionProfile::from_raw(profile.as_str()), Some(profile));
        }
        assert_eq!(DetectionProfile::Textured.as_str(), "textured");
        assert_eq!(DetectionProfile::from_raw("fuzzy"), None);
    }

    #[test]
    fn a_tag_with_one_frame_is_a_marker() {
        assert!(FigureTag::new(Uuid::new_v4(), 120, 120, "flip").is_marker());
    }

    #[test]
    fn orders_a_tags_ends() {
        let tag = FigureTag::new(Uuid::new_v4(), 367, 245, "split-s");
        assert_eq!((tag.start_frame, tag.end_frame), (245, 367));
        assert!(!tag.is_marker());
    }
}
