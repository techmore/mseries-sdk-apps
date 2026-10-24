//! What the app remembers between runs: the toolkit location, the project list, App Store Connect
//! identifiers and window geometry. Stored as JSON in `~/.config/mseries-apple-dev/settings.json`.
//!
//! Nothing secret is stored here. The App Store Connect key stays in its own `.p8` file; only its
//! path and the two public identifiers are kept, exactly what `ship.sh` reads from the environment.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The omarchy-apple-dev checkout. `None` means "find it" (see `toolkit::locate`).
    pub toolkit_path: Option<PathBuf>,
    pub projects: Vec<PathBuf>,
    /// The sidebar selection: `toolchain`, `devices` or `project:<path>`.
    pub selected: String,
    pub asc_key_path: String,
    pub asc_key_id: String,
    pub asc_issuer_id: String,
    /// Ten-character Apple team id, used by `ship.sh --device TEAMID`.
    pub team_id: String,
    /// The phone's tailnet address (100.x.y.z) for `device-run.sh --over-tailnet`.
    pub tailnet_host: String,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// Height of the Activity terminal in logical pixels.
    pub console_height: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            toolkit_path: None,
            projects: Vec::new(),
            selected: "toolchain".into(),
            asc_key_path: String::new(),
            asc_key_id: String::new(),
            asc_issuer_id: String::new(),
            team_id: String::new(),
            tailnet_host: String::new(),
            width: 1100,
            height: 760,
            maximized: false,
            console_height: 240,
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"));
    base.join("mseries-apple-dev")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

impl Settings {
    /// `MSERIES_APPLE_DEV_CONFIG` points at another settings file (fixtures, screenshots).
    pub fn file() -> PathBuf {
        std::env::var_os("MSERIES_APPLE_DEV_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir().join("settings.json"))
    }

    pub fn load() -> Settings {
        Self::load_from(&Self::file())
    }

    pub fn load_from(path: &Path) -> Settings {
        std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::file())
    }

    /// Write through a temporary file so a crash never leaves half a settings file.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    /// All three App Store Connect values are present and the key file exists.
    pub fn asc_ready(&self) -> bool {
        !self.asc_key_id.trim().is_empty()
            && !self.asc_issuer_id.trim().is_empty()
            && Path::new(self.asc_key_path.trim()).is_file()
    }

    /// The environment `ship.sh --upload`, `flutter/ship.sh --upload` and `ship-mac.sh --notarize` read.
    pub fn asc_env(&self) -> Vec<(String, String)> {
        vec![
            ("ASC_KEY_PATH".into(), self.asc_key_path.trim().into()),
            ("ASC_KEY_ID".into(), self.asc_key_id.trim().into()),
            ("ASC_ISSUER_ID".into(), self.asc_issuer_id.trim().into()),
        ]
    }

    /// Add a project once; returns false when it was already listed.
    pub fn add_project(&mut self, path: PathBuf) -> bool {
        if self.projects.contains(&path) {
            return false;
        }
        self.projects.push(path);
        true
    }

    pub fn remove_project(&mut self, path: &Path) {
        self.projects.retain(|p| p != path);
        if self.selected == format!("project:{}", path.display()) {
            self.selected = "toolchain".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let dir = std::env::temp_dir().join(format!("apple-dev-settings-{}", std::process::id()));
        let file = dir.join("settings.json");
        let mut s = Settings::default();
        s.asc_key_id = "ABCDE12345".into();
        assert!(s.add_project("/tmp/Hello".into()));
        assert!(!s.add_project("/tmp/Hello".into()));
        s.save_to(&file).unwrap();
        assert_eq!(Settings::load_from(&file), s);

        std::fs::write(&file, br#"{"team_id":"TEAM123456"}"#).unwrap();
        let partial = Settings::load_from(&file);
        assert_eq!(partial.team_id, "TEAM123456");
        assert_eq!(partial.selected, "toolchain");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn asc_needs_all_three_values_and_a_real_key_file() {
        let mut s = Settings::default();
        s.asc_key_id = "KEY".into();
        s.asc_issuer_id = "ISSUER".into();
        s.asc_key_path = "/definitely/not/here.p8".into();
        assert!(!s.asc_ready());
        s.asc_key_path = "/etc/hostname".into();
        assert!(s.asc_ready() || !Path::new("/etc/hostname").exists());
    }

    #[test]
    fn removing_the_selected_project_falls_back_to_toolchain() {
        let mut s = Settings::default();
        s.add_project("/p/A".into());
        s.selected = "project:/p/A".into();
        s.remove_project(Path::new("/p/A"));
        assert!(s.projects.is_empty());
        assert_eq!(s.selected, "toolchain");
    }
}
