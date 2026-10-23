//! Unprivileged app adapter for the OS-owned software updater contract.
//!
//! Status is read through the updater CLI. Every operation goes through
//! `pkexec` and the installed allowlisting helper; this SDK never runs package
//! operations directly or decides what is safe to install.

use mseries_update_core::command::{Command, UPDATER};
use mseries_update_core::model::Status;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command as Process;

/// Where the root helper is installed; must match the polkit action annotation.
pub const HELPER: &str = match option_env!("MSERIES_UPDATE_HELPER") {
    Some(path) => path,
    None => "/usr/libexec/mseries-updates/mseries-update-helper",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// `mseries-update` is not installed.
    NotInstalled,
    /// The authentication prompt was dismissed.
    Cancelled,
    NotAuthorized,
    /// Another update task holds the updater's lock (exit 75).
    Busy,
    /// Restoring failed (exit 3): the package set needs attention.
    RollbackFailed(String),
    Failed(String),
}

/// Shared adapter interface used by first-party Rust apps.
pub trait Backend: Send + Sync {
    fn status(&self) -> Result<Status, Failure>;
    fn run(&self, command: Command) -> Result<(), Failure>;
}

pub struct CliBackend {
    updater: PathBuf,
    helper: PathBuf,
}

impl CliBackend {
    pub fn with_paths(updater: impl Into<PathBuf>, helper: impl Into<PathBuf>) -> Self {
        Self {
            updater: updater.into(),
            helper: helper.into(),
        }
    }

    /// `MSERIES_UPDATE_BIN` changes only the unprivileged status source. Root
    /// operations always call the installed helper path.
    pub fn system() -> Self {
        let updater = std::env::var_os("MSERIES_UPDATE_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(UPDATER));
        Self {
            updater,
            helper: PathBuf::from(HELPER),
        }
    }
}

/// `mseries-update: reason` on stderr -> `reason`; the last non-empty line.
pub fn error_text(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    line.strip_prefix("mseries-update: ")
        .unwrap_or(line)
        .to_string()
}

/// Maps `pkexec` and helper exit statuses to UI-safe outcomes.
pub fn classify(code: Option<i32>, stderr: &[u8]) -> Result<(), Failure> {
    match code {
        Some(0) => Ok(()),
        Some(126) => Err(Failure::Cancelled),
        Some(127) => Err(Failure::NotAuthorized),
        Some(75) => Err(Failure::Busy),
        Some(3) => Err(Failure::RollbackFailed(error_text(stderr))),
        _ => {
            let text = error_text(stderr);
            Err(Failure::Failed(if text.is_empty() {
                format!("exit status {code:?}")
            } else {
                text
            }))
        }
    }
}

impl Backend for CliBackend {
    fn status(&self) -> Result<Status, Failure> {
        let output = match Process::new(&self.updater)
            .args(["status", "--json"])
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(Failure::NotInstalled);
            }
            Err(error) => return Err(Failure::Failed(error.to_string())),
        };
        if !output.status.success() {
            return Err(Failure::Failed(error_text(&output.stderr)));
        }
        Status::parse(&String::from_utf8_lossy(&output.stdout))
            .map_err(|error| Failure::Failed(error.to_string()))
    }

    fn run(&self, command: Command) -> Result<(), Failure> {
        let output = Process::new("pkexec")
            .arg(&self.helper)
            .args(command.helper_args())
            .output()
            .map_err(|error| Failure::Failed(format!("cannot run pkexec: {error}")))?;
        classify(output.status.code(), &output.stderr)
    }
}
