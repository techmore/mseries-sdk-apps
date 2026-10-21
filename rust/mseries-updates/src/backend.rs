//! How the app talks to the updater. Reading status needs no privilege; every change goes through
//! `pkexec` to the allowlisting helper under the polkit action `org.techmore.mseries.update`.

use mseries_update_core::command::{Command, UPDATER};
use mseries_update_core::model::Status;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Command as Process;

/// Where the root helper is installed; must equal the action's `exec.path` annotation.
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

pub trait Backend: Send + Sync {
    fn status(&self) -> Result<Status, Failure>;
    fn run(&self, command: Command) -> Result<(), Failure>;
}

pub struct CliBackend {
    updater: PathBuf,
    helper: PathBuf,
}

impl CliBackend {
    pub fn with_paths(updater: impl Into<PathBuf>, helper: impl Into<PathBuf>) -> CliBackend {
        CliBackend { updater: updater.into(), helper: helper.into() }
    }

    /// `MSERIES_UPDATE_BIN` points the unprivileged status read at another updater (development installs);
    /// it never affects what runs as root, which is always the helper.
    pub fn system() -> CliBackend {
        let updater = std::env::var_os("MSERIES_UPDATE_BIN").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(UPDATER));
        CliBackend { updater, helper: PathBuf::from(HELPER) }
    }
}

/// `mseries-update: reason` on stderr -> `reason`; the last non-empty line only.
pub fn error_text(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    line.strip_prefix("mseries-update: ").unwrap_or(line).to_string()
}

/// Exit status of the `pkexec ... helper` run -> result (pkexec: 126 dismissed, 127 not authorised).
pub fn classify(code: Option<i32>, stderr: &[u8]) -> Result<(), Failure> {
    match code {
        Some(0) => Ok(()),
        Some(126) => Err(Failure::Cancelled),
        Some(127) => Err(Failure::NotAuthorized),
        Some(75) => Err(Failure::Busy),
        Some(3) => Err(Failure::RollbackFailed(error_text(stderr))),
        _ => {
            let text = error_text(stderr);
            Err(Failure::Failed(if text.is_empty() { format!("exit status {code:?}") } else { text }))
        }
    }
}

impl Backend for CliBackend {
    fn status(&self) -> Result<Status, Failure> {
        let output = match Process::new(&self.updater).args(["status", "--json"]).output() {
            Ok(output) => output,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(Failure::NotInstalled),
            Err(e) => return Err(Failure::Failed(e.to_string())),
        };
        if !output.status.success() {
            return Err(Failure::Failed(error_text(&output.stderr)));
        }
        Status::parse(&String::from_utf8_lossy(&output.stdout)).map_err(|e| Failure::Failed(e.to_string()))
    }

    fn run(&self, command: Command) -> Result<(), Failure> {
        let output = Process::new("pkexec")
            .arg(&self.helper)
            .args(command.helper_args())
            .output()
            .map_err(|e| Failure::Failed(format!("cannot run pkexec: {e}")))?;
        classify(output.status.code(), &output.stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_map_to_what_the_user_should_see() {
        assert_eq!(classify(Some(0), b""), Ok(()));
        assert_eq!(classify(Some(126), b""), Err(Failure::Cancelled));
        assert_eq!(classify(Some(127), b""), Err(Failure::NotAuthorized));
        assert_eq!(classify(Some(75), b"mseries-update: another mseries-update is running\n"), Err(Failure::Busy));
        assert_eq!(
            classify(Some(3), b"mseries-update: ROLLBACK FAILED, the package set needs attention: x\n"),
            Err(Failure::RollbackFailed("ROLLBACK FAILED, the package set needs attention: x".into()))
        );
        assert_eq!(
            classify(Some(1), b"INFO noise\nmseries-update: update server unreachable\n\n"),
            Err(Failure::Failed("update server unreachable".into()))
        );
        assert_eq!(classify(None, b""), Err(Failure::Failed("exit status None".into())));
    }
}
