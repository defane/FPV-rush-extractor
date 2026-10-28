use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

/// The two bundled tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegTool {
    Ffmpeg,
    Ffprobe,
}

impl FfmpegTool {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    /// The executable: in `directory` when the binaries are bundled, else
    /// whatever `PATH` resolves, which is how the devcontainer runs.
    pub fn command(&self, directory: Option<&Path>) -> Command {
        let mut command = match directory {
            Some(directory) => Command::new(
                directory.join(format!("{}{}", self.name(), std::env::consts::EXE_SUFFIX))),
            None => Command::new(self.name()),
        };
        // Media children are pipes, not console applications the user opened.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        // Keep the binding mutable only on Windows without a platform warning.
        let _ = &mut command;
        command
    }
}

#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl ProcessResult {
    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    // The full argument vector, every time: silent ffmpeg failures are the
    // likeliest source of hard-to-diagnose bugs here (CLAUDE.md 8).
    #[error("{tool} exited {status}: {message}\n  {tool} {}", arguments.join(" "))]
    ToolFailed { tool: String, arguments: Vec<String>, status: i32, message: String },
    #[error("could not read {tool} output: {detail}")]
    UnreadableOutput { tool: String, detail: String },
    #[error("could not run {tool}: {source}")]
    Launch { tool: String, source: std::io::Error },
}

impl FfmpegError {
    pub fn tool_failed(tool: FfmpegTool, arguments: &[String], result: &ProcessResult) -> Self {
        Self::ToolFailed {
            tool: tool.name().into(), arguments: arguments.to_vec(),
            status: result.status, message: result.stderr.clone(),
        }
    }
}

/// The single boundary to the bundled binaries. Everything that shells out goes
/// through here so tests can substitute a stub instead of real media
/// (CLAUDE.md 3.1, 8).
pub trait FfmpegInvoking: Send + Sync {
    fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError>;
}

#[derive(Debug, Clone, Default)]
pub struct FfmpegRunner {
    executable_directory: Option<PathBuf>,
}

impl FfmpegRunner {
    /// `executable_directory`: where the bundled binaries live. `None` resolves
    /// the tools on `PATH`.
    pub fn new(executable_directory: Option<PathBuf>) -> Self {
        Self { executable_directory }
    }

    pub fn executable_directory(&self) -> Option<&Path> {
        self.executable_directory.as_deref()
    }
}

impl FfmpegInvoking for FfmpegRunner {
    fn run(&self, tool: FfmpegTool, arguments: &[String]) -> Result<ProcessResult, FfmpegError> {
        log::debug!("{} {}", tool.name(), arguments.join(" "));
        let mut child = tool.command(self.executable_directory())
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| FfmpegError::Launch { tool: tool.name().into(), source })?;

        // Drain stderr on its own thread: a full pipe buffer deadlocks the child,
        // and ffprobe's JSON for a long file is comfortably larger than the pipe.
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let errors = thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text);
            String::from_utf8_lossy(&text).into_owned()
        });
        let mut stdout = Vec::new();
        let read = child.stdout.take().expect("stdout is piped").read_to_end(&mut stdout);
        let status = child.wait();
        let stderr = errors.join().unwrap_or_default();
        let launch = |source| FfmpegError::Launch { tool: tool.name().into(), source };
        read.map_err(launch)?;
        let status = status.map_err(launch)?;
        // A child killed by a signal has no code; report it as a failure.
        Ok(ProcessResult { status: status.code().unwrap_or(-1), stdout, stderr })
    }
}
