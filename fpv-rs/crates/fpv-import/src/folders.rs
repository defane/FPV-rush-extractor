//! Where an import copies a recording: `<import folder>/<rendered pattern>/<file>`.

use chrono::{DateTime, Local, NaiveDate, Utc};
use fpv_core::MediaInfo;

use crate::DiscoveredFile;

/// The name of a session's folder on disk, from a pattern such as
/// `{date} {session} {source}` → `2026-10-10 Alpes FATSHARK`.
///
/// - `{date}`: the session's flying date, `YYYY-MM-DD` in local time — empty
///   for an undated session, the DVR having no clock.
/// - `{session}`: the session's name.
/// - `{source}`: what recorded the file (`source_label`). With it, one session
///   holding DVR and GoPro footage gets one folder per source.
///
/// Missing values leave no stray spaces, and characters no file system takes
/// in a folder name are replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderPattern(String);

impl FolderPattern {
    pub const DEFAULT: &'static str = "{date} {session} {source}";
    const TOKENS: [&'static str; 3] = ["{date}", "{session}", "{source}"];

    /// Refuses an empty pattern, an unknown `{token}`, and one without
    /// `{session}`, which would pour every session into the same folder.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if !text.contains("{session}") {
            return Err("the pattern needs {session}, or every session lands in one folder".into());
        }
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let close = rest[open..].find('}')
                .ok_or_else(|| format!("unclosed {{ in \"{text}\""))? + open;
            let token = &rest[open..=close];
            if !Self::TOKENS.contains(&token) {
                return Err(format!("unknown {token} — use {{date}}, {{session}} or {{source}}"));
            }
            rest = &rest[close + 1..];
        }
        Ok(Self(text.to_owned()))
    }

    /// Just the session's name: how the CLI and earlier builds laid folders out.
    pub fn session_only() -> Self {
        Self("{session}".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn render(&self, date: Option<DateTime<Utc>>, session: &str, source: &str) -> String {
        let date = date.map(|d| d.with_timezone(&Local).format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let filled = self.0.replace("{date}", &date).replace("{session}", session)
            .replace("{source}", source);
        let collapsed = filled.split_whitespace().collect::<Vec<_>>().join(" ");
        folder_name(&collapsed)
    }
}

impl Default for FolderPattern {
    fn default() -> Self {
        Self(Self::DEFAULT.into())
    }
}

/// What recorded a file, for `{source}`. MJPEG is the Fat Shark DVR's
/// signature (the Scout's `.mov` included); action cams are told apart by the
/// names they give their files.
pub fn source_label(file: &DiscoveredFile, media: &MediaInfo) -> &'static str {
    let name = file.name().to_ascii_uppercase();
    if media.is_analog_dvr() {
        "FATSHARK"
    } else if file.gopro.is_some() {
        "GOPRO"
    } else if name.starts_with("DJI_") {
        "DJI"
    } else if name.starts_with("VID_") && name.contains("_00_") {
        // Insta360: VID_20261010_101500_00_001.mp4
        "INSTA360"
    } else {
        "CAM"
    }
}

/// What recorded a file, as the library shows it: `DVR` for the goggles'
/// recordings; for an action cam, the model its tags name (`MediaInfo::camera`)
/// or else the brand its file names give away, and `Action cam` when neither
/// says.
pub fn source_name(file: &DiscoveredFile, media: &MediaInfo) -> String {
    if media.is_analog_dvr() {
        return "DVR".into();
    }
    if let Some(camera) = &media.camera {
        return camera.clone();
    }
    match source_label(file, media) {
        "GOPRO" => "GoPro",
        "DJI" => "DJI",
        "INSTA360" => "Insta360",
        _ => "Action cam",
    }.into()
}

/// The session a folder of the import folder holds, read back from its name:
/// a leading `YYYY-MM-DD` is its day, a trailing source label (`FATSHARK`,
/// `GOPRO`…) is dropped, the rest is its name. That undoes the default
/// pattern and the bare-session-name layout; a folder named any other way
/// keeps its whole name, which the pilot can rename.
pub fn session_of_folder(folder: &str) -> (Option<NaiveDate>, String) {
    let mut rest = folder.trim();
    let mut date = None;
    if let Some((head, tail)) = rest.split_once(' ')
        && let Ok(day) = NaiveDate::parse_from_str(head, "%Y-%m-%d")
    {
        date = Some(day);
        rest = tail.trim_start();
    }
    if let Some((head, last)) = rest.rsplit_once(' ')
        && SOURCE_LABELS.contains(&last)
    {
        rest = head.trim_end();
    }
    if rest.is_empty() { (date, folder.trim().to_owned()) } else { (date, rest.to_owned()) }
}

/// Every label `source_label` gives.
const SOURCE_LABELS: [&str; 5] = ["FATSHARK", "GOPRO", "DJI", "INSTA360", "CAM"];

/// A name a folder can carry on every desktop: no path separators, no colon,
/// never empty, and never `.` or `..` — a session named `..` under the
/// `{session}` pattern would have been imported beside the import folder.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name.chars()
        .map(|c| if c == '/' || c == ':' || c == '\\' { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().all(|c| c == '.') { "session".into() } else { trimmed.into() }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use fpv_core::FrameRate;

    use super::*;

    fn local(y: i32, m: u32, d: u32) -> Option<DateTime<Utc>> {
        let day = NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(0, 0, 0)?;
        Some(Local.from_local_datetime(&day).earliest()?.to_utc())
    }

    #[test]
    fn names_the_source_by_its_model_else_its_brand() {
        let take = |codec: &str, camera: Option<&str>| MediaInfo {
            camera: camera.map(String::from),
            ..MediaInfo::new(1920, 1080, FrameRate::new(60, 1), 1.0, codec)
        };
        let name = |file: &str, media: &MediaInfo| source_name(&DiscoveredFile::new(file), media);
        assert_eq!(name("PICT0030.AVI", &take("mjpeg", None)), "DVR");
        assert_eq!(name("GH010042.MP4", &take("h264", Some("GoPro HERO11"))), "GoPro HERO11");
        assert_eq!(name("GH010042.MP4", &take("h264", None)), "GoPro");
        assert_eq!(name("DJI_0001.MP4", &take("hevc", None)), "DJI");
        assert_eq!(name("VID_20261010_101500_00_001.mp4", &take("h264", None)), "Insta360");
        assert_eq!(name("clip.mp4", &take("h264", None)), "Action cam");
    }

    #[test]
    fn reads_a_session_back_from_its_folder_name() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 10);
        assert_eq!(session_of_folder("2026-10-10 Alpes FATSHARK"), (day, "Alpes".into()));
        assert_eq!(session_of_folder("2026-10-10 Entrainement Letz Kwad GOPRO"),
                   (day, "Entrainement Letz Kwad".into()));
        // The bare-session-name layout, and an undated session.
        assert_eq!(session_of_folder("Letz Kwad"), (None, "Letz Kwad".into()));
        assert_eq!(session_of_folder("Alpes DJI"), (None, "Alpes".into()));
        // A name is never emptied: a source word alone stays the name.
        assert_eq!(session_of_folder("GOPRO"), (None, "GOPRO".into()));
        assert_eq!(session_of_folder("2026-10-10 CAM"), (day, "CAM".into()));
        // Every label the folders are made with reads back.
        for label in ["FATSHARK", "GOPRO", "DJI", "INSTA360", "CAM"] {
            let folder = FolderPattern::default().render(local(2026, 10, 10), "Alpes", label);
            assert_eq!(session_of_folder(&folder), (day, "Alpes".into()), "{folder}");
        }
    }

    #[test]
    fn renders_the_pilots_example() {
        let pattern = FolderPattern::default();
        assert_eq!(pattern.render(local(2026, 10, 10), "Alpes", "FATSHARK"), "2026-10-10 Alpes FATSHARK");
        assert_eq!(pattern.render(local(2026, 10, 10), "Alpes", "GOPRO"), "2026-10-10 Alpes GOPRO");
    }

    #[test]
    fn leaves_no_stray_space_for_an_undated_session() {
        assert_eq!(FolderPattern::default().render(None, "Alpes", "FATSHARK"), "Alpes FATSHARK");
        assert_eq!(FolderPattern::parse("{date} - {session}").unwrap().render(None, "Alpes", "X"),
                   "- Alpes");
    }

    #[test]
    fn keeps_folder_names_legal() {
        assert_eq!(FolderPattern::session_only().render(None, "23/08 : Letz", "X"), "23-08 - Letz");
        assert_eq!(folder_name("   "), "session");
    }

    #[test]
    fn never_names_a_folder_that_leaves_the_import_folder() {
        for name in ["..", ".", " .. ", "..."] {
            assert_eq!(FolderPattern::session_only().render(None, name, "X"), "session", "{name:?}");
        }
        assert_eq!(folder_name("../Alpes"), "..-Alpes");
        assert_eq!(folder_name(".hidden"), ".hidden");
    }

    #[test]
    fn refuses_patterns_that_cannot_work() {
        assert!(FolderPattern::parse("{date} {source}").is_err());
        assert!(FolderPattern::parse("{session} {place}").is_err());
        assert!(FolderPattern::parse("{session} {date").is_err());
        assert_eq!(FolderPattern::parse("  {source}/{session}  ").unwrap().as_str(), "{source}/{session}");
    }

    #[test]
    fn tells_the_sources_apart() {
        let dvr = MediaInfo::new(640, 480, FrameRate::new(30, 1), 1.0, "mjpeg");
        let cam = MediaInfo::new(1920, 1080, FrameRate::new(60, 1), 1.0, "h264");
        let label = |name: &str, media: &MediaInfo| source_label(&DiscoveredFile::new(name), media);
        assert_eq!(label("PICT0030.AVI", &dvr), "FATSHARK");
        assert_eq!(label("MOV250823-142530-000001F.MOV", &dvr), "FATSHARK");
        assert_eq!(label("GH010042.MP4", &cam), "GOPRO");
        assert_eq!(label("DJI_0001.MP4", &cam), "DJI");
        assert_eq!(label("VID_20261010_101500_00_001.mp4", &cam), "INSTA360");
        assert_eq!(label("clip.mp4", &cam), "CAM");
    }
}
