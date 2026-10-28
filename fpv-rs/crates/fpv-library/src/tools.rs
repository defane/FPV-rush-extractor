use std::path::{Path, PathBuf};
use std::process::Stdio;

use fpv_media::FfmpegTool;

/// Where the app finds ffmpeg and ffprobe: the bundle's own `bin` folder when
/// both are shipped there; on macOS, Homebrew's prefixes — an app started from
/// the Finder gets a bare `PATH` without them; else `None`, resolving the tools
/// on `PATH`, which is how a Linux install with the distribution's ffmpeg runs.
/// Resolved here and nowhere else, so packaging changes one function.
pub fn tool_directory(resources: Option<&Path>) -> Option<PathBuf> {
    let homebrew: &[&str] = if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin", "/usr/local/bin"]
    } else {
        &[]
    };
    first_with_both(resources.map(|r| r.join("bin")).into_iter()
        .chain(homebrew.iter().map(PathBuf::from)))
}

/// The first directory holding both tools.
fn first_with_both(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|directory| {
        [FfmpegTool::Ffmpeg, FfmpegTool::Ffprobe].iter().all(|tool| {
            directory.join(format!("{}{}", tool.name(), std::env::consts::EXE_SUFFIX)).is_file()
        })
    })
}

/// Whether both tools actually run — the app cannot do anything useful
/// without them, and says so up front rather than failing on first import.
pub fn ffmpeg_available(directory: Option<&Path>) -> bool {
    [FfmpegTool::Ffmpeg, FfmpegTool::Ffprobe].iter().all(|tool| {
        tool.command(directory).arg("-version")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status().is_ok_and(|status| status.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_the_bundled_tools_only_when_both_are_there() {
        let resources = tempfile::tempdir().unwrap();
        let bin = resources.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX)), b"").unwrap();
        assert_eq!(tool_directory(Some(resources.path())), tool_directory(None));
        std::fs::write(bin.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)), b"").unwrap();
        assert_eq!(tool_directory(Some(resources.path())), Some(bin));
        // Elsewhere than macOS there is no fallback: the tools come from PATH.
        if !cfg!(target_os = "macos") {
            assert_eq!(tool_directory(None), None);
        }
    }

    #[test]
    fn falls_back_through_the_candidates_in_order() {
        let (empty, half, full) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(),
                                   tempfile::tempdir().unwrap());
        let suffix = std::env::consts::EXE_SUFFIX;
        std::fs::write(half.path().join(format!("ffmpeg{suffix}")), b"").unwrap();
        for tool in ["ffmpeg", "ffprobe"] {
            std::fs::write(full.path().join(format!("{tool}{suffix}")), b"").unwrap();
        }
        let candidates = [empty.path(), half.path(), full.path()].map(Path::to_path_buf);
        assert_eq!(first_with_both(candidates), Some(full.path().to_path_buf()));
        assert_eq!(first_with_both([empty.path().to_path_buf()]), None);
    }
}
