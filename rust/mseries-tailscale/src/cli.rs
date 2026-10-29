//! Runs the `tailscale` command line without blocking the UI.
//!
//! `MSERIES_TAILSCALE_BIN` overrides the binary. `MSERIES_TAILSCALE_FIXTURES=DIR` reads `status.json` and
//! `prefs.json` from DIR instead, and every change then fails with "access denied", for screenshots and tests.

use gtk::gio;
use gtk::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub enum CliError {
    /// No `tailscale` binary.
    NotInstalled,
    /// `tailscaled` is not running (or its socket is missing).
    NotRunning,
    /// Changing settings needs root or the operator setting.
    AccessDenied,
    Failed(String),
}

impl CliError {
    pub fn message(&self) -> String {
        match self {
            Self::NotInstalled => "Tailscale isn't installed.".into(),
            Self::NotRunning => "The Tailscale service isn't running.".into(),
            Self::AccessDenied => "Changing Tailscale settings needs a one-time setup.".into(),
            Self::Failed(text) => text.clone(),
        }
    }
}

/// Command that lets the current user change Tailscale settings without sudo.
pub fn operator_command() -> String {
    format!("sudo tailscale set --operator={}", gtk::glib::user_name().to_string_lossy())
}

pub const START_SERVICE_COMMAND: &str = "sudo systemctl enable --now tailscaled";

fn binary() -> String {
    std::env::var("MSERIES_TAILSCALE_BIN").unwrap_or_else(|_| "tailscale".into())
}

fn fixtures() -> Option<std::path::PathBuf> {
    std::env::var_os("MSERIES_TAILSCALE_FIXTURES").map(Into::into)
}

/// Sorts a failed command's output into a state the window can explain.
pub fn classify(stderr: &str) -> CliError {
    let lower = stderr.to_lowercase();
    if lower.contains("access denied") || lower.contains("permission denied") || lower.contains("prefs write access") {
        CliError::AccessDenied
    } else if lower.contains("failed to connect to local tailscaled")
        || lower.contains("tailscaled.sock")
        || lower.contains("is tailscaled running")
    {
        CliError::NotRunning
    } else {
        let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Tailscale didn't respond.");
        CliError::Failed(first.to_string())
    }
}

/// Runs `tailscale ARGS…` and returns its standard output.
pub async fn run(args: &[&str]) -> Result<String, CliError> {
    if let Some(dir) = fixtures() {
        return match args {
            ["status", "--json"] => std::fs::read_to_string(dir.join("status.json")).map_err(|_| CliError::NotRunning),
            ["debug", "prefs"] => std::fs::read_to_string(dir.join("prefs.json")).map_err(|_| CliError::NotRunning),
            _ => Err(CliError::AccessDenied),
        };
    }
    let bin = binary();
    let mut argv: Vec<&std::ffi::OsStr> = vec![bin.as_ref()];
    argv.extend(args.iter().map(|a| std::ffi::OsStr::new(*a)));
    let proc = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE)
        .map_err(|e| {
            if e.matches(gio::IOErrorEnum::NotFound) || e.message().contains("No such file") {
                CliError::NotInstalled
            } else {
                CliError::Failed(e.message().to_string())
            }
        })?;
    let (out, err) = proc.communicate_utf8_future(None).await.map_err(|e| CliError::Failed(e.message().to_string()))?;
    let out = out.map(|s| s.to_string()).unwrap_or_default();
    let err = err.map(|s| s.to_string()).unwrap_or_default();
    if proc.is_successful() {
        Ok(out)
    } else if args.first() == Some(&"status") && !out.trim().is_empty() {
        // `status --json` exits non-zero when stopped or logged out but still prints the state.
        Ok(out)
    } else {
        Err(classify(&err))
    }
}

/// Starts `tailscale login` and returns the sign-in URL it prints, without waiting for the sign-in to finish.
pub async fn login_url() -> Result<String, CliError> {
    if fixtures().is_some() {
        return Err(CliError::AccessDenied);
    }
    let proc = gio::Subprocess::newv(
        &[binary().as_ref(), std::ffi::OsStr::new("login")],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
    )
    .map_err(|e| CliError::Failed(e.message().to_string()))?;
    let stream = gio::DataInputStream::new(&proc.stdout_pipe().expect("stdout pipe"));
    let mut seen = String::new();
    loop {
        match stream.read_line_utf8_future(gtk::glib::Priority::DEFAULT).await {
            Ok(Some(line)) => {
                if let Some(url) = line.split_whitespace().find(|w| w.starts_with("https://")) {
                    return Ok(url.to_string());
                }
                seen.push_str(&line);
                seen.push('\n');
            }
            Ok(None) => break,
            Err(e) => return Err(CliError::Failed(e.message().to_string())),
        }
    }
    // Exited without a URL: already logged in, or an error.
    if seen.trim().is_empty() { Ok(String::new()) } else { Err(classify(&seen)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_errors() {
        let denied = "Access denied: checkprefs access denied\n\nUse 'sudo tailscale set --exit-node='.";
        assert_eq!(classify(denied), CliError::AccessDenied);
        let down = "failed to connect to local tailscaled; it doesn't appear to be running (sudo systemctl start tailscaled ?)";
        assert_eq!(classify(down), CliError::NotRunning);
        assert_eq!(classify("\nboom\nmore"), CliError::Failed("boom".into()));
    }
}
