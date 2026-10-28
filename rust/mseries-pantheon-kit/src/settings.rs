//! Per-app settings files: JSON under the user's config directory.
//!
//! One place for the path rule (`$XDG_CONFIG_HOME`, else `~/.config`; a relative `XDG_CONFIG_HOME` is
//! ignored as the XDG spec says) and for loading and saving, so an app's `Settings`, `WindowState` or
//! sidebar file is a `load_json` and a `save_json` call and not another copy of the path code.
//!
//! The environment is read on every call (not cached) so tests can point an app at a scratch directory.

use serde::{Serialize, de::DeserializeOwned};
use std::io;
use std::path::{Path, PathBuf};

/// The user's config directory.
pub fn user_config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        })
}

/// `<user config dir>/<app>`: the directory an app keeps its files in.
pub fn app_config_dir(app: &str) -> PathBuf {
    user_config_dir().join(app)
}

/// Reads `path` as JSON; a missing, unreadable or malformed file gives `None`.
pub fn try_load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// Reads `path` as JSON; a missing, unreadable or malformed file gives the default.
pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    try_load_json(path).unwrap_or_default()
}

/// Writes `value` as pretty JSON, creating the directory. Goes through a temporary file and a rename,
/// so a crash never leaves a half-written settings file.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(io::Error::other)?,
    )?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
    struct Sample {
        #[serde(default)]
        name: String,
        #[serde(default)]
        count: u32,
    }

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("kit-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn round_trips_and_creates_the_directory() {
        let path = scratch("round").join("nested").join("sample.json");
        let value = Sample {
            name: "a".into(),
            count: 3,
        };
        save_json(&path, &value).unwrap();
        assert_eq!(load_json::<Sample>(&path), value);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "no temporary file is left behind"
        );
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn missing_or_malformed_files_give_the_default() {
        let directory = scratch("bad");
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(
            load_json::<Sample>(&directory.join("none.json")),
            Sample::default()
        );
        std::fs::write(directory.join("bad.json"), "{ not json").unwrap();
        assert_eq!(
            load_json::<Sample>(&directory.join("bad.json")),
            Sample::default()
        );
        assert!(try_load_json::<Sample>(&directory.join("bad.json")).is_none());
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_partial_file_keeps_serde_defaults() {
        let directory = scratch("partial");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("p.json"), r#"{"name":"x"}"#).unwrap();
        assert_eq!(
            load_json::<Sample>(&directory.join("p.json")),
            Sample {
                name: "x".into(),
                count: 0
            }
        );
        std::fs::remove_dir_all(&directory).ok();
    }
}
