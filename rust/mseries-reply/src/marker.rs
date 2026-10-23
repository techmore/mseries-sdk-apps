//! A chat app declares itself in its desktop entry, so the system never has to guess what a window is.
//!
//! ```ini
//! [Desktop Entry]
//! X-MSeries-Chat=true
//! X-MSeries-Chat-Adapter=blueferry   ; optional
//! ```
//! An unmarked app gets nothing: no read, no popup, no response to the hotkey.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChatMarker {
    /// Names a structured transcript source (for example `blueferry`); `None` means the bounded AT-SPI fallback.
    pub adapter: Option<String>,
}

/// Reads the marker from the text of a desktop entry. Only the `[Desktop Entry]` group counts.
pub fn parse_desktop_entry(text: &str) -> Option<ChatMarker> {
    let mut in_entry = false;
    let mut marked = false;
    let mut adapter = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "X-MSeries-Chat" => {
                marked = matches!(value.to_ascii_lowercase().as_str(), "true" | "1")
            }
            "X-MSeries-Chat-Adapter" if !value.is_empty() => adapter = Some(value.to_string()),
            _ => {}
        }
    }
    marked.then_some(ChatMarker { adapter })
}

pub fn default_dirs() -> Vec<PathBuf> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let mut dirs = Vec::new();
    dirs.extend(data_home.map(|dir| dir.join("applications")));
    dirs.push(PathBuf::from("/usr/local/share/applications"));
    dirs.push(PathBuf::from("/usr/share/applications"));
    dirs
}

/// Finds the marker for an application id. An id that could escape the directory is treated as unmarked.
pub fn lookup(app_id: &str, dirs: &[PathBuf]) -> Option<ChatMarker> {
    if app_id.is_empty() || app_id.contains(['/', '\\', '\0']) || app_id.contains("..") {
        return None;
    }
    dirs.iter()
        .map(|dir| Path::new(dir).join(format!("{app_id}.desktop")))
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| parse_desktop_entry(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_true_in_the_desktop_entry_group_marks_a_chat_app() {
        let marked =
            "[Desktop Entry]\nName=Chat\nX-MSeries-Chat=true\nX-MSeries-Chat-Adapter=blueferry\n";
        assert_eq!(
            parse_desktop_entry(marked),
            Some(ChatMarker {
                adapter: Some("blueferry".into())
            })
        );
        assert_eq!(parse_desktop_entry("[Desktop Entry]\nName=Chat\n"), None);
        assert_eq!(
            parse_desktop_entry("[Desktop Entry]\nX-MSeries-Chat=false\n"),
            None
        );
        // A key in another group (for example an action) does not mark the app.
        assert_eq!(
            parse_desktop_entry(
                "[Desktop Entry]\nName=X\n[Desktop Action a]\nX-MSeries-Chat=true\n"
            ),
            None
        );
    }

    #[test]
    fn lookup_reads_the_entry_and_refuses_paths() {
        let dir = std::env::temp_dir().join(format!("mseries-reply-marker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("chat.app.desktop"),
            "[Desktop Entry]\nX-MSeries-Chat=true\n",
        )
        .unwrap();
        std::fs::write(dir.join("plain.desktop"), "[Desktop Entry]\nName=Plain\n").unwrap();
        let dirs = [dir.clone()];
        assert!(lookup("chat.app", &dirs).is_some());
        assert!(lookup("plain", &dirs).is_none());
        assert!(lookup("missing", &dirs).is_none());
        assert!(lookup("../chat.app", &dirs).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
