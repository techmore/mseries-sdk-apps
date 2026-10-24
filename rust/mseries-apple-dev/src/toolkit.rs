//! Finding the omarchy-apple-dev checkout and reporting the state of its toolchain.
//!
//! The probe reads the same signals the toolkit's own `install-toolchain.sh --repair` summary uses:
//! `swift`, `swift sdk list` (a `darwin` SDK), `xtool`, the Apple ID state in `~/.local/share/xtool`,
//! the pymobiledevice3 venv in `~/pymobile3-venv` and the install mode in
//! `~/.config/omarchy-apple-dev/mode`. Commands run in one login shell so the user's PATH
//! (mise, ~/.local/bin) applies, exactly as in a terminal. `probe` blocks; run it off the UI thread.

use crate::settings::home;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const UPSTREAM: &str = "https://github.com/joshuaswarren/omarchy-apple-dev";

/// Where Omarchy's Install menu puts the toolkit; also where "Download Toolkit" clones it.
pub fn default_location() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"));
    data.join("omarchy-apple-dev")
}

pub fn candidates() -> Vec<PathBuf> {
    let home = home();
    vec![
        default_location(),
        home.join("omarchy-apple-dev"),
        home.join("Projects/omarchy-apple-dev"),
        home.join("src/omarchy-apple-dev"),
        home.join("Code/omarchy-apple-dev"),
    ]
}

pub fn is_toolkit(dir: &Path) -> bool {
    dir.join("install-toolchain.sh").is_file() && dir.join("device-run.sh").is_file()
}

/// The configured checkout when it is valid, else the first known location that holds one.
pub fn locate(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = configured.filter(|d| is_toolkit(d)) {
        return Some(dir.to_path_buf());
    }
    candidates().into_iter().find(|d| is_toolkit(d))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Ready,
    /// Present but needs attention (a stale or partial state).
    Attention,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    Swift,
    DarwinSdk,
    Xtool,
    AppleId,
    PyMobileDevice,
    Usbmuxd,
    Flutter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub id: CheckId,
    pub title: &'static str,
    pub state: CheckState,
    pub detail: String,
    /// Optional checks never block the "ready" summary.
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Host {
    pub pretty_name: String,
    /// Arch or Arch-based: the installer's pacman steps apply.
    pub arch_based: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub toolkit: Option<PathBuf>,
    /// `abc1234 · 2026-10-23` of the checkout's HEAD.
    pub revision: Option<String>,
    pub mode: Option<String>,
    pub host: Host,
    pub checks: Vec<Check>,
}

impl Report {
    /// Build and run need Swift, the Darwin SDK and xtool.
    pub fn can_build(&self) -> bool {
        self.toolkit.is_some()
            && [CheckId::Swift, CheckId::DarwinSdk, CheckId::Xtool]
                .iter()
                .all(|id| self.state(*id) == CheckState::Ready)
    }

    pub fn state(&self, id: CheckId) -> CheckState {
        self.checks.iter().find(|c| c.id == id).map(|c| c.state).unwrap_or(CheckState::Missing)
    }

    /// How many required checks are not ready (the sidebar badge).
    pub fn problems(&self) -> usize {
        usize::from(self.toolkit.is_none())
            + self.checks.iter().filter(|c| !c.optional && c.state != CheckState::Ready).count()
    }
}

const PROBE_SCRIPT: &str = r#"
printf 'swift=%s\n' "$(swift --version 2>/dev/null | head -n1)"
if command -v swift >/dev/null 2>&1; then printf 'sdks=%s\n' "$(swift sdk list 2>/dev/null | tr '\n' ' ')"; fi
printf 'xtool=%s\n' "$(xtool --version 2>/dev/null | head -n1)"
printf 'xtool_path=%s\n' "$(command -v xtool)"
printf 'flutter=%s\n' "$(command -v flutter)"
printf 'usbmuxd=%s\n' "$(command -v usbmuxd || ls /usr/sbin/usbmuxd 2>/dev/null)"
printf 'ideviceinfo=%s\n' "$(command -v ideviceinfo)"
"#;

/// `key=value` lines from the probe shell. Later duplicates win; lines without `=` are ignored.
pub fn parse_probe(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub fn parse_os_release(text: &str) -> Host {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            values.insert(k.trim(), v.trim().trim_matches('"'));
        }
    }
    let ids = format!("{} {}", values.get("ID").unwrap_or(&""), values.get("ID_LIKE").unwrap_or(&""));
    Host {
        pretty_name: values.get("PRETTY_NAME").copied().unwrap_or("Linux").to_string(),
        arch_based: ids.split_whitespace().any(|id| id == "arch" || id == "archarm"),
    }
}

/// Run a program with a deadline; stdout on success, `None` on failure or timeout.
pub fn run_with_timeout(program: &str, args: &[&str], cwd: Option<&Path>, limit: Duration) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub fn probe(configured: Option<&Path>) -> Report {
    let toolkit = locate(configured);
    let host = parse_os_release(&std::fs::read_to_string("/etc/os-release").unwrap_or_default());
    let probe = run_with_timeout("bash", &["--login", "-c", PROBE_SCRIPT], None, Duration::from_secs(25))
        .map(|out| parse_probe(&out))
        .unwrap_or_default();
    let revision = toolkit.as_deref().and_then(|dir| {
        run_with_timeout("git", &["log", "-1", "--format=%h · %cs"], Some(dir), Duration::from_secs(5))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    let home = home();
    let mode = std::fs::read_to_string(home.join(".config/omarchy-apple-dev/mode"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let xtool_auth =
        std::fs::read_dir(home.join(".local/share/xtool")).map(|mut d| d.next().is_some()).unwrap_or(false);
    let venv = home.join("pymobile3-venv/bin/pymobiledevice3");
    Report { checks: checks_from(&probe, xtool_auth, venv.is_file()), toolkit, revision, mode, host }
}

/// Turn the probe values into the rows the Toolchain page shows. Pure, so it is unit tested.
pub fn checks_from(p: &BTreeMap<String, String>, xtool_auth: bool, venv: bool) -> Vec<Check> {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    let swift = get("swift");
    let sdks = get("sdks");
    let xtool = get("xtool");
    let xtool_path = get("xtool_path");
    let have = |s: &str| !s.is_empty();
    let row = |id, title, ready: bool, ok: String, missing: &str, optional| Check {
        id,
        title,
        state: if ready { CheckState::Ready } else { CheckState::Missing },
        detail: if ready { ok } else { missing.to_string() },
        optional,
    };
    vec![
        row(
            CheckId::Swift,
            "Swift toolchain",
            have(swift),
            swift.to_string(),
            "Not on PATH. Install Toolchain installs Swift 6.4 (AUR swift-bin on Arch).",
            false,
        ),
        if !have(swift) {
            row(CheckId::DarwinSdk, "iOS SDK", false, String::new(), "Needs Swift first.", false)
        } else {
            row(
                CheckId::DarwinSdk,
                "iOS SDK",
                sdks.split_whitespace().any(|s| s.starts_with("darwin")),
                "darwin SDK registered with Swift".into(),
                "No darwin SDK. Run Install Toolchain with an Xcode .xip, or Repair after a Swift swap.",
                false,
            )
        },
        if have(xtool) || have(xtool_path) {
            row(CheckId::Xtool, "xtool", true, if have(xtool) { xtool.into() } else { xtool_path.into() }, "", false)
        } else {
            row(
                CheckId::Xtool,
                "xtool",
                false,
                String::new(),
                "Not installed. Install Toolchain builds it from source.",
                false,
            )
        },
        row(
            CheckId::AppleId,
            "Apple ID",
            xtool_auth,
            "Signed in for device installs (~/.local/share/xtool)".into(),
            "Not signed in. Sign In runs `xtool auth` (a free Apple ID works).",
            false,
        ),
        row(
            CheckId::PyMobileDevice,
            "pymobiledevice3",
            venv,
            "~/pymobile3-venv (pairing, screenshots, logs, LLDB tunnel)".into(),
            "No ~/pymobile3-venv. Install Toolchain creates it.",
            false,
        ),
        row(
            CheckId::Usbmuxd,
            "usbmuxd",
            have(get("usbmuxd")),
            "USB multiplexer for iPhone connections".into(),
            "Not installed. Install the usbmuxd package.",
            false,
        ),
        row(
            CheckId::Flutter,
            "Flutter",
            have(get("flutter")),
            get("flutter").to_string(),
            "Optional. Needed only for Flutter apps (flutter/setup.sh).",
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe_lines() {
        let p = parse_probe("swift=Swift version 6.4.0\nsdks=darwin \nnoise\nxtool=\n");
        assert_eq!(p["swift"], "Swift version 6.4.0");
        assert_eq!(p["sdks"], "darwin");
        assert_eq!(p["xtool"], "");
    }

    #[test]
    fn empty_probe_reports_everything_required_missing() {
        let checks = checks_from(&BTreeMap::new(), false, false);
        assert!(checks.iter().filter(|c| !c.optional).all(|c| c.state == CheckState::Missing));
        let report = Report { toolkit: None, revision: None, mode: None, host: Host::default(), checks };
        assert!(!report.can_build());
        assert_eq!(report.problems(), 7);
    }

    #[test]
    fn full_toolchain_can_build() {
        let p = parse_probe(
            "swift=Swift version 6.4.0 (swift-6.4-RELEASE)\nsdks=darwin\nxtool=1.23.0\nusbmuxd=/usr/sbin/usbmuxd\n",
        );
        let checks = checks_from(&p, true, true);
        let report = Report {
            toolkit: Some("/x".into()),
            revision: None,
            mode: Some("full".into()),
            host: Host::default(),
            checks,
        };
        assert!(report.can_build());
        assert_eq!(report.problems(), 0, "flutter is optional");
    }

    #[test]
    fn sdk_without_darwin_is_missing() {
        let p = parse_probe("swift=Swift 6.4\nsdks=wasm32 \n");
        let sdk = checks_from(&p, false, false).into_iter().find(|c| c.id == CheckId::DarwinSdk).unwrap();
        assert_eq!(sdk.state, CheckState::Missing);
    }

    #[test]
    fn os_release_detects_arch_family() {
        assert!(parse_os_release("ID=arch\nPRETTY_NAME=\"Arch Linux\"").arch_based);
        assert!(parse_os_release("ID=omarchy\nID_LIKE=arch").arch_based);
        let el = parse_os_release("ID=elementary\nID_LIKE=ubuntu\nPRETTY_NAME=\"elementary OS 8\"");
        assert!(!el.arch_based);
        assert_eq!(el.pretty_name, "elementary OS 8");
    }

    #[test]
    fn timeout_kills_a_slow_command() {
        let start = Instant::now();
        assert!(run_with_timeout("sleep", &["5"], None, Duration::from_millis(200)).is_none());
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(run_with_timeout("echo", &["hi"], None, Duration::from_secs(2)).as_deref(), Some("hi\n"));
    }
}
