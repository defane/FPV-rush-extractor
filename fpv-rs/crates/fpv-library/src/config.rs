use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The app's own folder in the user's configuration directory:
/// `~/.config/fpv_rush_extractor` on Linux, `~/Library/Application
/// Support/fpv_rush_extractor` on macOS, `%APPDATA%\fpv_rush_extractor` on
/// Windows. It holds the library database and `config.json` — small, and
/// worth backing up. The footage lives in `media_root`, which is not.
pub const APP_DIRECTORY: &str = "fpv_rush_extractor";
const FILE: &str = "config.json";

/// What the user may change by editing `config.json`. Missing keys take their
/// defaults, so the file can hold only what differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Where imported recordings, posters and playback proxies go: the
    /// app-managed copy of every card (CLAUDE.md 9.3).
    pub media_root: PathBuf,
    /// The playback proxy cache's budget, least recently used evicted first.
    pub proxy_budget_gb: f64,
    /// The pictures' budget: hover strips, posters and the editor's frames
    /// together (`Library::trim_pictures`).
    pub picture_budget_gb: f64,
    /// How each session's folder is named in the import folder, from
    /// `{date}`, `{session}` and `{source}` — see `fpv_import::FolderPattern`.
    pub folder_pattern: String,
    /// Omit audio streams from exported video files.
    pub mute_videos: bool,
    /// Skip the startup splash screen.
    pub do_not_show_splash: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            media_root: dirs::video_dir().or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from(".")).join("FPV Rush"),
            proxy_budget_gb: 10.0,
            picture_budget_gb: 2.0,
            folder_pattern: fpv_import::FolderPattern::DEFAULT.into(),
            mute_videos: false,
            do_not_show_splash: false,
        }
    }
}

impl Config {
    /// The per-user cache folder, `~/.cache/fpv_rush_extractor` on Linux
    /// (`~/Library/Caches/…` on macOS, `%LOCALAPPDATA%\…` on Windows).
    pub fn default_cache_directory() -> PathBuf {
        dirs::cache_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_DIRECTORY)
    }

    pub fn default_directory() -> PathBuf {
        dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIRECTORY)
    }

    /// Reads `config.json` in `directory`, writing one with the defaults when
    /// there is none, so the user finds a file to edit rather than a blank.
    pub fn load_or_create(directory: &Path) -> std::io::Result<Self> {
        let path = directory.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| std::io::Error::new(
                std::io::ErrorKind::InvalidData, format!("{}: {error}", path.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save(directory)?;
                Ok(config)
            }
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        let text = serde_json::to_string_pretty(self).expect("a config always serialises");
        std::fs::write(directory.join(FILE), text + "\n")
    }

    pub fn proxy_budget_bytes(&self) -> i64 {
        (self.proxy_budget_gb.max(0.0) * (1u64 << 30) as f64) as i64
    }

    pub fn picture_budget_bytes(&self) -> u64 {
        (self.picture_budget_gb.max(0.0) * (1u64 << 30) as f64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_defaults_on_first_run_and_reads_edits_back() {
        let directory = tempfile::tempdir().unwrap();
        let first = Config::load_or_create(directory.path()).unwrap();
        assert_eq!(first, Config::default());
        assert!(directory.path().join("config.json").exists());

        std::fs::write(directory.path().join("config.json"),
                       r#"{ "mediaRoot": "/data/fpv" }"#).unwrap();
        let edited = Config::load_or_create(directory.path()).unwrap();
        assert_eq!(edited.media_root, PathBuf::from("/data/fpv"));
        assert_eq!(edited.proxy_budget_gb, 10.0); // a missing key keeps its default
        assert_eq!(edited.proxy_budget_bytes(), 10 << 30);
        assert_eq!(edited.picture_budget_bytes(), 2 << 30);
    }

    #[test]
    fn refuses_a_broken_file_rather_than_overwriting_it() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), "{ mediaRoot: ").unwrap();
        assert!(Config::load_or_create(directory.path()).is_err());
        assert_eq!(std::fs::read_to_string(directory.path().join("config.json")).unwrap(),
                   "{ mediaRoot: ");
    }
}
