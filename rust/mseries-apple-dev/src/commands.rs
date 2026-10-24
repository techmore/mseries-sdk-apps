//! Every button maps to one toolkit command, built here as a shell script for the Activity terminal.
//! Nothing in this module runs anything; the window hands a [`Job`] to the console.

use crate::project::{self, Kind, Project};
use crate::settings::Settings;
use std::path::{Path, PathBuf};

/// One command for the Activity terminal: runs as `bash --login -c script` in `cwd`.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub title: String,
    pub cwd: PathBuf,
    pub script: String,
    pub env: Vec<(String, String)>,
    /// What to do when it succeeds.
    pub after: After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    Nothing,
    /// Re-run the toolchain probe (installs, sign-in, repair).
    RecheckToolchain,
    /// Add this folder to the project list (new app, Xcode conversion).
    AddProject(PathBuf),
    /// Refresh the project page (a new build artifact).
    RefreshProject,
    /// Refresh the project page, then show this project's newest builds in App Store Connect.
    CheckTestFlight(PathBuf),
}

/// Environment shared by every job and the toolchain probe:
/// - Swift's backtracer cannot protect its helper's path on a 16K-page kernel and says so on every
///   command, so it is off.
/// - Builds write gigabytes of temporaries; /tmp is RAM on M-series laptops, so they go to disk.
/// - A libimobiledevice built from source into [`crate::toolkit::LIMD_PREFIX`] (when the distro's
///   packages are missing or too old) is found by pkg-config, the loader and PATH.
pub const ENV_SETUP: &str = r#"export SWIFT_BACKTRACE=enable=no
TMPDIR="$HOME/.cache/apple-dev-tmp"; mkdir -p "$TMPDIR" 2>/dev/null && export TMPDIR || unset TMPDIR
LIMD="$HOME/.local/opt/libimobiledevice"
if [ -d "$LIMD/lib" ]; then
  PATH="$LIMD/bin:$PATH"
  LD_LIBRARY_PATH="$LIMD/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  PKG_CONFIG_PATH="$LIMD/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  export PATH LD_LIBRARY_PATH PKG_CONFIG_PATH
fi
"#;

/// [`ENV_SETUP`], then the toolchain's own clang/lldb first and a raised open-file limit, as the
/// toolkit README asks for every build ("Notes" and "Real projects").
const PREAMBLE_TAIL: &str = r#"set -o pipefail
if command -v swift >/dev/null 2>&1; then
  PATH="$(dirname "$(readlink -f "$(command -v swift)")"):$HOME/.local/bin:$PATH"; export PATH
fi
ulimit -n 65536 2>/dev/null || ulimit -n "$(ulimit -Hn)" 2>/dev/null || true
"#;

pub fn shell_quote(value: &str) -> String {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b)) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn q(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

fn job(title: impl Into<String>, cwd: &Path, command: String, after: After) -> Job {
    Job {
        title: title.into(),
        cwd: cwd.into(),
        script: format!("{ENV_SETUP}{PREAMBLE_TAIL}{command}\n"),
        env: Vec::new(),
        after,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Build,
    Run,
    Debug,
    Attach,
    DeviceBuild,
    AppStoreBuild,
    Upload,
    TailnetInstall,
    FlutterBuild,
    FlutterInstall,
    FlutterUpload,
    MacBuild,
    MacNotarize,
    ConvertXcode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Develop,
    Distribute,
}

impl Action {
    pub fn title(self) -> &'static str {
        match self {
            Action::Build | Action::FlutterBuild => "Build",
            Action::Run => "Run on iPhone",
            Action::Debug => "Debug with LLDB",
            Action::Attach => "Attach Debugger",
            Action::DeviceBuild => "Device Build",
            Action::AppStoreBuild => "App Store Build",
            Action::Upload | Action::FlutterUpload => "Upload to TestFlight",
            Action::TailnetInstall => "Install over Tailscale",
            Action::FlutterInstall => "Install on iPhone",
            Action::MacBuild => "Build Mac App",
            Action::MacNotarize => "Notarize",
            Action::ConvertXcode => "Convert to xtool",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::Build => "Compile a debug build with xtool dev build.",
            Action::Run => "Build, install and launch on the connected iPhone.",
            Action::Debug => "Install, then attach LLDB with Swift values (first run copies the phone's runtime).",
            Action::Attach => "Debug the copy already on the phone without reinstalling.",
            Action::DeviceBuild => "Release layout with every resource at the bundle root, like Xcode.",
            Action::AppStoreBuild => {
                "Signed .ipa validated offline. Without App Store Connect keys it uses a test identity."
            }
            Action::Upload => "Build, sign with your distribution identity and upload. Never submits for review.",
            Action::TailnetInstall => "Install the newest signed .ipa on the phone over your tailnet (iOS 27).",
            Action::FlutterBuild => "Release arm64 build to build/ios-linux/Runner.ipa.",
            Action::FlutterInstall => "Build, then sign and install with xtool (free Apple ID).",
            Action::FlutterUpload => "Sign for the App Store, validate offline and upload.",
            Action::MacBuild => "Build and sign build/<App>.app (Developer ID if present, else ad hoc).",
            Action::MacNotarize => "Build, sign with Developer ID and notarize with Apple.",
            Action::ConvertXcode => "Write an xtool adapter (omarchy-xtool) next to the project.",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Action::Build | Action::FlutterBuild | Action::MacBuild | Action::DeviceBuild => "system-run-symbolic",
            Action::Run | Action::FlutterInstall => "media-playback-start-symbolic",
            Action::Debug | Action::Attach => "system-search-symbolic",
            Action::AppStoreBuild => "package-x-generic-symbolic",
            Action::Upload | Action::FlutterUpload | Action::MacNotarize => "send-to-symbolic",
            Action::TailnetInstall => "network-wireless-symbolic",
            Action::ConvertXcode => "document-revert-symbolic",
        }
    }

    pub fn section(self) -> Section {
        match self {
            Action::DeviceBuild
            | Action::AppStoreBuild
            | Action::Upload
            | Action::TailnetInstall
            | Action::FlutterUpload
            | Action::MacNotarize => Section::Distribute,
            _ => Section::Develop,
        }
    }

    /// Uses the phone; the selected device is passed with `--udid` where the script takes it.
    pub fn uses_device(self) -> bool {
        matches!(self, Action::Run | Action::Debug | Action::Attach | Action::FlutterInstall)
    }
}

pub fn actions_for(kind: Kind) -> &'static [Action] {
    match kind {
        Kind::Xtool => &[
            Action::Build,
            Action::Run,
            Action::Debug,
            Action::Attach,
            Action::DeviceBuild,
            Action::AppStoreBuild,
            Action::Upload,
            Action::TailnetInstall,
        ],
        Kind::Flutter => &[Action::FlutterBuild, Action::FlutterInstall, Action::FlutterUpload],
        Kind::MacPackage => &[Action::MacBuild, Action::MacNotarize],
        Kind::XcodeProject => &[Action::ConvertXcode],
    }
}

/// The header bar's Build and Run buttons for a project kind.
pub fn primary_actions(kind: Kind) -> (Option<Action>, Option<Action>) {
    match kind {
        Kind::Xtool => (Some(Action::Build), Some(Action::Run)),
        Kind::Flutter => (Some(Action::FlutterBuild), Some(Action::FlutterInstall)),
        Kind::MacPackage => (Some(Action::MacBuild), None),
        Kind::XcodeProject => (Some(Action::ConvertXcode), None),
    }
}

/// Why an action cannot run right now; `None` when it can.
pub fn blocker(action: Action, project: &Project, settings: &Settings) -> Option<&'static str> {
    match action {
        Action::Upload | Action::FlutterUpload | Action::MacNotarize if !settings.asc_ready() => {
            Some("Add your App Store Connect key first (main menu › App Store Connect…).")
        }
        Action::TailnetInstall if settings.tailnet_host.trim().is_empty() => {
            Some("Set the phone's tailnet address on the Devices page first.")
        }
        Action::TailnetInstall if project::latest_ipa(project).is_none() => {
            Some("Needs a signed .ipa in xtool/. Run App Store Build or Upload first.")
        }
        _ => None,
    }
}

pub fn project_job(action: Action, project: &Project, toolkit: &Path, settings: &Settings, udid: Option<&str>) -> Job {
    let tk = |script: &str| q(&toolkit.join(script));
    let dir = &project.path;
    let udid_arg = udid.map(|u| format!(" --udid {}", shell_quote(u))).unwrap_or_default();
    let title = format!("{} · {}", action.title(), project.name);
    let mut job = match action {
        Action::Build => job(title, dir, "xtool dev build".into(), After::RefreshProject),
        Action::Run => job(title, dir, format!("{}{udid_arg}", tk("device-run.sh")), After::RefreshProject),
        Action::Debug => job(title, dir, format!("{} --lldb{udid_arg}", tk("device-run.sh")), After::Nothing),
        Action::Attach => job(title, dir, format!("{} --attach{udid_arg}", tk("device-run.sh")), After::Nothing),
        Action::DeviceBuild => {
            let team = settings.team_id.trim();
            let cmd = if team.is_empty() {
                format!("{} --device", tk("ship.sh"))
            } else {
                format!("{} --device {}", tk("ship.sh"), shell_quote(team))
            };
            job(title, dir, cmd, After::RefreshProject)
        }
        Action::AppStoreBuild => job(title, dir, tk("ship.sh"), After::RefreshProject),
        Action::Upload => job(title, dir, format!("{} --upload", tk("ship.sh")), After::CheckTestFlight(dir.clone())),
        Action::TailnetInstall => {
            let ipa = project::latest_ipa(project).unwrap_or_default();
            let cmd = format!(
                "{} --over-tailnet {} {}",
                tk("device-run.sh"),
                shell_quote(settings.tailnet_host.trim()),
                q(&ipa)
            );
            job(title, dir, cmd, After::Nothing)
        }
        Action::FlutterBuild => {
            job(title, dir, format!("{} {}", tk("flutter/build.sh"), q(dir)), After::RefreshProject)
        }
        Action::FlutterInstall => {
            job(title, dir, format!("{} --install {}", tk("flutter/build.sh"), q(dir)), After::RefreshProject)
        }
        Action::FlutterUpload => job(
            title,
            dir,
            format!("{} --upload {}", tk("flutter/ship.sh"), q(dir)),
            After::CheckTestFlight(dir.clone()),
        ),
        Action::MacBuild => job(title, dir, tk("ship-mac.sh"), After::RefreshProject),
        Action::MacNotarize => job(title, dir, format!("{} --notarize", tk("ship-mac.sh")), After::RefreshProject),
        Action::ConvertXcode => {
            let parent = dir.parent().unwrap_or(dir);
            job(
                title,
                parent,
                format!("python3 {} {}", tk("tools/xcodeproj2xtool.py"), q(dir)),
                After::AddProject(project::xtool_adapter_dir(dir)),
            )
        }
    };
    if matches!(action, Action::Upload | Action::FlutterUpload | Action::MacNotarize | Action::AppStoreBuild)
        && settings.asc_ready()
    {
        job.env = settings.asc_env();
    }
    job
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    Full,
    NoXcode,
}

impl InstallMode {
    pub fn flag(self) -> &'static str {
        match self {
            InstallMode::Full => "full",
            InstallMode::NoXcode => "no-xcode",
        }
    }
}

pub fn install_job(toolkit: &Path, mode: InstallMode, xip: Option<&Path>) -> Job {
    let mut job = job(
        "Install toolchain",
        toolkit,
        format!("./install-toolchain.sh --mode {}", mode.flag()),
        After::RecheckToolchain,
    );
    if let (InstallMode::Full, Some(xip)) = (mode, xip) {
        job.env.push(("XCODE_XIP".into(), xip.to_string_lossy().into_owned()));
    }
    job
}

pub fn repair_job(toolkit: &Path) -> Job {
    job("Repair toolchain", toolkit, "./install-toolchain.sh --repair".into(), After::RecheckToolchain)
}

/// Install the apt packages in the Activity terminal; sudo asks for the password there.
pub fn apt_job(home: &Path, packages: &[String]) -> Job {
    let list: Vec<String> = packages.iter().map(|p| shell_quote(p)).collect();
    job("Install packages", home, format!("sudo apt-get install -y {}", list.join(" ")), After::RecheckToolchain)
}

/// Download a swift.org toolchain into `~/.local/share/swift` (signature checked against
/// swift.org's keys in a throwaway keyring), link `~/.local/bin/swift` to it and, when an iOS SDK
/// is already cached, re-register it with the new compiler (`install-toolchain.sh --repair`).
pub fn swift_install_job(home: &Path, tarball: &crate::distro::Tarball, toolkit: Option<&Path>) -> Job {
    let repair = toolkit
        .map(|tk| {
            format!(
                "if [ -n \"$fresh\" ] && ls -d \"$HOME\"/.cache/xtool/darwin-*.xtoolsdk >/dev/null 2>&1; then\n  (cd {} && ./install-toolchain.sh --repair)\nfi\n",
                q(tk)
            )
        })
        .unwrap_or_default();
    let script = format!(
        r#"set -e
name={name}; url={url}
dest="$HOME/{dir}"; fresh=
if [ ! -x "$dest/$name/usr/bin/swift" ]; then
  command -v gpg >/dev/null || {{ echo "gpg is missing: install the packages under This Computer first (gnupg2)."; exit 1; }}
  work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
  echo "Downloading $name from swift.org (about 1 GB)"
  curl -fL --progress-bar -o "$work/swift.tar.gz" "$url"
  curl -fsSL -o "$work/swift.tar.gz.sig" "$url.sig"
  export GNUPGHOME="$work/gnupg"; mkdir -m 700 "$GNUPGHOME"
  curl -fsSL https://www.swift.org/keys/all-keys.asc | gpg --batch --quiet --import
  gpg --batch --verify "$work/swift.tar.gz.sig" "$work/swift.tar.gz"
  mkdir -p "$dest"; tar -xzf "$work/swift.tar.gz" -C "$dest"; fresh=1
fi
mkdir -p "$HOME/.local/bin"; ln -sfn "$dest/$name/usr/bin/swift" "$HOME/.local/bin/swift"
PATH="$dest/$name/usr/bin:$HOME/.local/bin:$PATH"; hash -r
swift --version
{repair}"#,
        name = shell_quote(&tarball.name),
        url = shell_quote(&tarball.url),
        dir = crate::toolkit::SWIFT_DIR,
    );
    job(format!("Install Swift {}", tarball.swift), home, script, After::RecheckToolchain)
}

pub fn sign_in_job(home: &Path) -> Job {
    job("Sign in to Apple ID", home, "xtool auth".into(), After::RecheckToolchain)
}

pub fn flutter_setup_job(toolkit: &Path) -> Job {
    job("Set up Flutter for iOS", toolkit, "./flutter/setup.sh".into(), After::RecheckToolchain)
}

pub fn clone_job(dest: &Path) -> Job {
    let parent = dest.parent().unwrap_or(Path::new("/"));
    job(
        "Download omarchy-apple-dev",
        parent,
        format!("mkdir -p {} && git clone {} {}", q(parent), crate::toolkit::UPSTREAM, q(dest)),
        After::RecheckToolchain,
    )
}

pub fn update_job(toolkit: &Path) -> Job {
    job("Update omarchy-apple-dev", toolkit, "git pull --ff-only".into(), After::RecheckToolchain)
}

pub fn new_app_job(parent: &Path, name: &str) -> Job {
    job(
        format!("New app · {name}"),
        parent,
        format!("xtool new {}", shell_quote(name)),
        After::AddProject(parent.join(name)),
    )
}

const PMD3: &str = r#"PMD3="$HOME/pymobile3-venv/bin/pymobiledevice3"; [ -x "$PMD3" ] || PMD3=$(command -v pymobiledevice3 || echo "$PMD3")
"#;

pub fn pair_job(home: &Path, udid: &str) -> Job {
    let u = shell_quote(udid);
    job(
        "Pair iPhone",
        home,
        format!(
            "{PMD3}if [ -x \"$PMD3\" ]; then \"$PMD3\" lockdown pair --udid {u}; else idevicepair -u {u} pair; fi\necho 'Tap Trust on the phone if it asks, then pair again.'"
        ),
        After::Nothing,
    )
}

pub fn info_job(home: &Path, udid: &str) -> Job {
    let u = shell_quote(udid);
    job(
        "iPhone details",
        home,
        format!("{PMD3}if [ -x \"$PMD3\" ]; then \"$PMD3\" lockdown info --udid {u}; else ideviceinfo -u {u}; fi"),
        After::Nothing,
    )
}

pub fn syslog_job(home: &Path, udid: &str) -> Job {
    let u = shell_quote(udid);
    job(
        "Live log",
        home,
        format!("{PMD3}if [ -x \"$PMD3\" ]; then \"$PMD3\" syslog live --udid {u}; else idevicesyslog -u {u}; fi"),
        After::Nothing,
    )
}

pub fn tailnet_pair_job(toolkit: &Path) -> Job {
    job(
        "Pair for Tailscale",
        toolkit,
        format!(
            "{PMD3}PY=\"$(dirname \"$PMD3\")/python\"; [ -x \"$PY\" ] || PY=python3\n\"$PY\" tools/tailnet-tunnel.py pair"
        ),
        After::Nothing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xtool_project() -> Project {
        Project {
            path: "/home/me/My Apps/Hello".into(),
            name: "Hello".into(),
            kind: Kind::Xtool,
            bundle_id: Some("com.example.Hello".into()),
            product: None,
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("plain-path/x.sh"), "plain-path/x.sh");
        assert_eq!(shell_quote("has space"), "'has space'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn run_targets_the_selected_phone_from_the_project_dir() {
        let tk = Path::new("/opt/omarchy apple");
        let job = project_job(Action::Run, &xtool_project(), tk, &Settings::default(), Some("00008110-ABC"));
        assert_eq!(job.cwd, PathBuf::from("/home/me/My Apps/Hello"));
        assert!(job.script.ends_with("'/opt/omarchy apple/device-run.sh' --udid 00008110-ABC\n"), "{}", job.script);
        assert!(job.script.contains("ulimit -n 65536"));
        assert!(job.env.is_empty());
    }

    #[test]
    fn upload_is_blocked_until_keys_exist_and_then_passes_them() {
        let p = xtool_project();
        let mut s = Settings::default();
        assert!(blocker(Action::Upload, &p, &s).is_some());
        assert!(blocker(Action::Build, &p, &s).is_none());
        s.asc_key_id = "KEYID".into();
        s.asc_issuer_id = "ISSUER".into();
        s.asc_key_path = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        assert!(blocker(Action::Upload, &p, &s).is_none());
        let job = project_job(Action::Upload, &p, Path::new("/tk"), &s, None);
        assert!(job.script.ends_with("/tk/ship.sh --upload\n"));
        assert_eq!(job.after, After::CheckTestFlight("/home/me/My Apps/Hello".into()));
        assert!(job.env.iter().any(|(k, v)| k == "ASC_KEY_ID" && v == "KEYID"));
    }

    #[test]
    fn every_job_gets_the_shared_environment() {
        let job = project_job(Action::Build, &xtool_project(), Path::new("/tk"), &Settings::default(), None);
        assert!(job.script.starts_with("export SWIFT_BACKTRACE=enable=no\n"));
        assert!(job.script.contains(r#"TMPDIR="$HOME/.cache/apple-dev-tmp""#));
        for var in ["PATH=\"$LIMD/bin:", "LD_LIBRARY_PATH=\"$LIMD/lib", "PKG_CONFIG_PATH=\"$LIMD/lib/pkgconfig"] {
            assert!(job.script.contains(var), "{var}");
        }
        assert!(job.script.find("LIMD=").unwrap() < job.script.find("xtool dev build").unwrap());
    }

    #[test]
    fn env_setup_runs_in_bash() {
        let home = std::env::temp_dir().join(format!("apple-dev-env-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".local/opt/libimobiledevice/lib")).unwrap();
        let out = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                &format!("{ENV_SETUP}echo \"$SWIFT_BACKTRACE|$TMPDIR|$PKG_CONFIG_PATH|${{PATH%%:*}}\""),
            ])
            .env("HOME", &home)
            .env_remove("PKG_CONFIG_PATH")
            .output()
            .unwrap();
        let h = home.display();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!(
                "enable=no|{h}/.cache/apple-dev-tmp|{h}/.local/opt/libimobiledevice/lib/pkgconfig|{h}/.local/opt/libimobiledevice/bin"
            )
        );
        assert!(home.join(".cache/apple-dev-tmp").is_dir());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn swift_install_verifies_links_and_repairs() {
        let t = crate::distro::swift_tarball("6.4.1", "26.04", "aarch64").unwrap();
        let job = swift_install_job(Path::new("/h"), &t, Some(Path::new("/tk")));
        assert_eq!(job.title, "Install Swift 6.4.1");
        assert!(job.script.contains("name=swift-6.4.1-RELEASE-ubuntu26.04-aarch64;"));
        assert!(job.script.contains("gpg --batch --verify"));
        assert!(job.script.contains(r#"ln -sfn "$dest/$name/usr/bin/swift" "$HOME/.local/bin/swift""#));
        assert!(job.script.contains("(cd /tk && ./install-toolchain.sh --repair)"));
        assert!(!swift_install_job(Path::new("/h"), &t, None).script.contains("--repair"));
        let out = std::process::Command::new("bash").args(["-n", "-c", &job.script]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn apt_job_quotes_packages() {
        let job = apt_job(Path::new("/h"), &["llvm".into(), "libz3-dev".into()]);
        assert!(job.script.ends_with("sudo apt-get install -y llvm libz3-dev\n"));
        assert_eq!(job.after, After::RecheckToolchain);
    }

    #[test]
    fn device_build_passes_the_team() {
        let s = Settings { team_id: "ABCDE12345".into(), ..Settings::default() };
        let job = project_job(Action::DeviceBuild, &xtool_project(), Path::new("/tk"), &s, None);
        assert!(job.script.ends_with("/tk/ship.sh --device ABCDE12345\n"));
    }

    #[test]
    fn install_passes_the_xip_only_in_full_mode() {
        let xip = Path::new("/dl/Xcode 27.xip");
        let full = install_job(Path::new("/tk"), InstallMode::Full, Some(xip));
        assert!(full.script.ends_with("./install-toolchain.sh --mode full\n"));
        assert_eq!(full.env, vec![("XCODE_XIP".to_string(), "/dl/Xcode 27.xip".to_string())]);
        let lite = install_job(Path::new("/tk"), InstallMode::NoXcode, Some(xip));
        assert!(lite.env.is_empty());
        assert!(lite.script.contains("--mode no-xcode"));
    }

    #[test]
    fn xcode_conversion_adds_the_adapter() {
        let p = Project {
            path: "/w/Demo/Demo.xcodeproj".into(),
            name: "Demo".into(),
            kind: Kind::XcodeProject,
            bundle_id: None,
            product: None,
        };
        let job = project_job(Action::ConvertXcode, &p, Path::new("/tk"), &Settings::default(), None);
        assert_eq!(job.cwd, PathBuf::from("/w/Demo"));
        assert_eq!(job.after, After::AddProject("/w/Demo/omarchy-xtool".into()));
    }

    #[test]
    fn every_kind_has_actions_and_a_primary_build() {
        for kind in [Kind::Xtool, Kind::Flutter, Kind::MacPackage, Kind::XcodeProject] {
            assert!(!actions_for(kind).is_empty());
            assert!(primary_actions(kind).0.is_some());
        }
    }
}
