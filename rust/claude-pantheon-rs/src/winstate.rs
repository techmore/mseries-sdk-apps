//! Window state across launches: size, maximised flag and the sidebar width, in
//! `~/.config/claude-pantheon-rs/window.json`. Read once before the window shows, written on close.

use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    pub sidebar: i32,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState { width: 1180, height: 760, maximized: false, sidebar: 280 }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs").join("window.json")
}

impl WindowState {
    pub fn load() -> WindowState {
        let s: WindowState =
            std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        s.sane()
    }

    /// Keeps a state that still fits on a screen: nothing below 640×420, a sidebar between 200 and 600.
    pub fn sane(mut self) -> WindowState {
        self.width = self.width.clamp(640, 8192);
        self.height = self.height.clamp(420, 8192);
        self.sidebar = self.sidebar.clamp(200, 600);
        self
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(p, bytes);
        }
    }

    pub fn apply(&self, window: &gtk::ApplicationWindow, paned: &gtk::Paned) {
        window.set_default_size(self.width, self.height);
        paned.set_position(self.sidebar);
        if self.maximized {
            window.maximize();
        }
    }

    /// What the window looks like now (GTK keeps the default size in step with the real size while
    /// the window is not maximised, so the unmaximised size survives a maximised close).
    pub fn capture(window: &gtk::ApplicationWindow, paned: &gtk::Paned) -> WindowState {
        let (width, height) = window.default_size();
        WindowState { width, height, maximized: window.is_maximized(), sidebar: paned.position() }.sane()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_and_round_trips() {
        let s = WindowState { width: 10, height: 20_000, maximized: true, sidebar: 50 }.sane();
        assert_eq!((s.width, s.height, s.sidebar), (640, 8192, 200));
        let json = serde_json::to_string(&s).unwrap();
        let back: WindowState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        let partial: WindowState = serde_json::from_str(r#"{"width": 900}"#).unwrap();
        assert_eq!((partial.width, partial.height, partial.sidebar, partial.maximized), (900, 760, 280, false));
    }
}
