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
    /// The command did not finish in time and was stopped.
    TimedOut(String),
    Failed(String),
}

impl CliError {
    pub fn message(&self) -> String {
        match self {
            Self::NotInstalled => "Tailscale isn't installed.".into(),
            Self::NotRunning => "The Tailscale service isn't running.".into(),
            Self::AccessDenied => "Changing Tailscale settings needs a one-time setup.".into(),
            Self::TimedOut(what) => format!("{what} didn't answer in time. Check your network and try again."),
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
    let mut argv: Vec<&str> = vec![&bin];
    argv.extend_from_slice(args);
    // Changes can wait on the network (`up` after a link change); reads should be instant.
    let limit = match args.first() {
        Some(&"up") | Some(&"down") => 25,
        Some(&"set") => 15,
        _ => 8,
    };
    match run_program(&argv, limit).await {
        Ok((true, out, _)) => Ok(out),
        // `status --json` exits non-zero when stopped or logged out but still prints the state.
        Ok((false, out, _)) if args.first() == Some(&"status") && !out.trim().is_empty() => Ok(out),
        Ok((false, _, err)) => Err(classify(&err)),
        Err(CliError::TimedOut(_)) => Err(CliError::TimedOut("Tailscale".into())),
        Err(e) => Err(e),
    }
}

/// Runs ARGV with a time limit in seconds; returns (succeeded, stdout, stderr). Stops the process on timeout.
pub async fn run_program(argv: &[&str], limit_secs: u64) -> Result<(bool, String, String), CliError> {
    if let (Some(dir), Some(&"nmcli")) = (fixtures(), argv.first()) {
        // Fixture VPNs: `nmcli.txt` is the connection list; every VPN sends all traffic unless listed in
        // `split-tunnel.txt`; changes are refused like Tailscale's.
        let list = std::fs::read_to_string(dir.join("nmcli.txt")).unwrap_or_default();
        return match argv.get(1..4) {
            Some(["-t", "-f", _]) => Ok((true, list, String::new())),
            Some(["-g", "ipv4.never-default", _]) => {
                let split = std::fs::read_to_string(dir.join("split-tunnel.txt")).unwrap_or_default();
                let uuid = argv.last().copied().unwrap_or_default();
                Ok((true, if split.lines().any(|l| l == uuid) { "yes" } else { "no" }.into(), String::new()))
            }
            // `unsaved-passwords.txt` lists VPNs whose secrets are "always ask".
            Some(["-g", "vpn.data", _]) => {
                let unsaved = std::fs::read_to_string(dir.join("unsaved-passwords.txt")).unwrap_or_default();
                let uuid = argv.last().copied().unwrap_or_default();
                let flags = if unsaved.lines().any(|l| l == uuid) { 2 } else { 0 };
                Ok((true, format!("password-flags = {flags}, ipsec-psk-flags = {flags}"), String::new()))
            }
            _ => Ok((false, String::new(), "Error: not allowed in fixture mode".into())),
        };
    }
    let os: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let proc = gio::Subprocess::newv(&os, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE)
        .map_err(|e| {
            if e.matches(gio::IOErrorEnum::NotFound) || e.message().contains("No such file") {
                CliError::NotInstalled
            } else {
                CliError::Failed(e.message().to_string())
            }
        })?;
    let running = proc.communicate_utf8_future(None);
    match gtk::glib::future_with_timeout(std::time::Duration::from_secs(limit_secs), running).await {
        Err(_) => {
            proc.force_exit();
            Err(CliError::TimedOut(argv.first().copied().unwrap_or_default().to_string()))
        }
        Ok(Err(e)) => Err(CliError::Failed(e.message().to_string())),
        Ok(Ok((out, err))) => Ok((
            proc.is_successful(),
            out.map(|s| s.to_string()).unwrap_or_default(),
            err.map(|s| s.to_string()).unwrap_or_default(),
        )),
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
