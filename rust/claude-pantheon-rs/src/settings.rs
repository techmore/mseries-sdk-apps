//! The Settings window (issue #476): a Granite-style sidebar of pages with switches and dropdowns.
//! Appearance (follow system / light / dark), Defaults (provider and permission mode, per project),
//! Server (pairing state, pair/unpair, the MCP endpoint), Notifications, Advanced (renderer override,
//! keyring status). Persisted in `~/.config/claude-pantheon-rs/settings.json`; the renderer choice is
//! read by `main()` before GTK starts.

use crate::chat::label;
use gtk::{gio, glib, pango, prelude::*};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectDefaults {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub runtime_mode: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// "system", "light", "dark"
    #[serde(default = "default_system")]
    pub appearance: String,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default = "default_mode")]
    pub default_runtime_mode: String,
    /// project id → overrides
    #[serde(default)]
    pub projects: HashMap<String, ProjectDefaults>,
    #[serde(default = "yes")]
    pub notify_turn_finished: bool,
    #[serde(default = "yes")]
    pub notify_prompts: bool,
    #[serde(default = "yes")]
    pub dock_badge: bool,
    /// "cairo" (default), "gl", "ngl", "vulkan"
    #[serde(default = "default_renderer")]
    pub renderer: String,
}

fn default_system() -> String {
    "system".into()
}
fn default_mode() -> String {
    "approval-required".into()
}
fn default_renderer() -> String {
    "cairo".into()
}
fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: default_system(),
            default_provider: None,
            default_runtime_mode: default_mode(),
            projects: HashMap::new(),
            notify_turn_finished: true,
            notify_prompts: true,
            dock_badge: true,
            renderer: default_renderer(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs").join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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

    /// Provider and mode for a project: its overrides, else the global defaults.
    pub fn defaults_for(&self, project_id: &str) -> (Option<String>, String) {
        let o = self.projects.get(project_id);
        (
            o.and_then(|p| p.provider.clone()).or_else(|| self.default_provider.clone()),
            o.and_then(|p| p.runtime_mode.clone()).unwrap_or_else(|| self.default_runtime_mode.clone()),
        )
    }

    /// Applies the appearance choice to GTK (Granite's stylesheet follows the same flag).
    pub fn apply_appearance(&self) {
        let Some(settings) = gtk::Settings::default() else { return };
        #[allow(deprecated)]
        match self.appearance.as_str() {
            "light" => settings.set_gtk_application_prefer_dark_theme(false),
            "dark" => settings.set_gtk_application_prefer_dark_theme(true),
            // "system": Granite's own style manager already follows the portal; asking it here again
            // costs a synchronous D-Bus round trip at start-up, so leave it alone.
            _ => {}
        }
    }
}

/// What the window needs to show and act on; kept small so the settings module stays independent.
pub struct Context {
    pub providers: Vec<(String, String)>, // (instance id, name)
    pub projects: Vec<(String, String)>,  // (id, title)
    pub server: Option<String>,
    pub server_label: Option<String>,
    pub keyring: bool,
    pub on_pair: Rc<dyn Fn()>,
    pub on_unpair: Rc<dyn Fn()>,
}

pub struct SettingsWindow {
    pub window: gtk::Window,
    settings: Rc<RefCell<Settings>>,
}

const MODES: [(&str, &str); 4] = [
    ("approval-required", "Ask before actions"),
    ("auto-accept-edits", "Auto-accept edits"),
    ("auto", "Auto"),
    ("full-access", "Full access"),
];

fn row(title: &str, subtitle: Option<&str>, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    r.add_css_class("settings-row");
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    texts.append(&label(title, &[]));
    if let Some(s) = subtitle {
        let l = label(s, &["caption", "dim-label"]);
        l.set_wrap(true);
        l.set_ellipsize(pango::EllipsizeMode::None);
        texts.append(&l);
    }
    r.append(&texts);
    control.set_valign(gtk::Align::Center);
    r.append(control);
    r
}

fn page(title: &str, description: &str) -> gtk::Box {
    let p = gtk::Box::new(gtk::Orientation::Vertical, 10);
    p.set_margin_top(18);
    p.set_margin_bottom(18);
    p.set_margin_start(24);
    p.set_margin_end(24);
    let h = label(title, &["title-2"]);
    p.append(&h);
    let d = label(description, &["dim-label"]);
    d.set_wrap(true);
    d.set_ellipsize(pango::EllipsizeMode::None);
    p.append(&d);
    p
}

impl SettingsWindow {
    pub fn open(parent: &gtk::ApplicationWindow, settings: Rc<RefCell<Settings>>, ctx: Context) -> Rc<SettingsWindow> {
        let window = gtk::Window::builder()
            .transient_for(parent)
            .title("Settings")
            .default_width(760)
            .default_height(520)
            .build();
        let header = gtk::HeaderBar::builder().show_title_buttons(true).build();
        header.add_css_class(granite::STYLE_CLASS_FLAT);
        window.set_titlebar(Some(&header));
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&stack);
        sidebar.set_width_request(190);
        sidebar.add_css_class(granite::STYLE_CLASS_SIDEBAR);
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&sidebar)
            .end_child(&stack)
            .position(190)
            .shrink_start_child(false)
            .resize_start_child(false)
            .build();
        window.set_child(Some(&paned));
        let s = settings.clone();

        // ---- Appearance ----------------------------------------------------------------------
        let appearance = page(
            "Appearance",
            "Light and dark follow the system by default; Granite's accent colour comes from the system settings.",
        );
        let choices = gtk::DropDown::from_strings(&["Follow system", "Light", "Dark"]);
        choices.set_selected(match s.borrow().appearance.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        });
        let s2 = s.clone();
        choices.connect_selected_notify(move |d| {
            let mut st = s2.borrow_mut();
            st.appearance = ["system", "light", "dark"][d.selected() as usize].into();
            st.save();
            st.apply_appearance();
        });
        appearance.append(&row("Style", Some("Which stylesheet variant the window uses"), &choices));
        stack.add_titled(&appearance, Some("appearance"), "Appearance");

        // ---- Defaults ------------------------------------------------------------------------
        let defaults = page(
            "Defaults",
            "The provider and permission mode new threads start with. A project can override both; full access stays a per-thread choice in the composer.",
        );
        let provider_names: Vec<String> = std::iter::once("Server's first ready provider".to_string())
            .chain(ctx.providers.iter().map(|(_, n)| n.clone()))
            .collect();
        let refs: Vec<&str> = provider_names.iter().map(String::as_str).collect();
        let provider_dd = gtk::DropDown::from_strings(&refs);
        let current = s.borrow().default_provider.clone();
        provider_dd.set_selected(
            current.and_then(|c| ctx.providers.iter().position(|(id, _)| *id == c)).map(|i| i as u32 + 1).unwrap_or(0),
        );
        let s2 = s.clone();
        let ids: Vec<String> = ctx.providers.iter().map(|(id, _)| id.clone()).collect();
        provider_dd.connect_selected_notify(move |d| {
            let mut st = s2.borrow_mut();
            st.default_provider = if d.selected() == 0 { None } else { ids.get(d.selected() as usize - 1).cloned() };
            st.save();
        });
        defaults.append(&row("Default provider", None, &provider_dd));
        let mode_names: Vec<&str> = MODES.iter().map(|(_, l)| *l).collect();
        let mode_dd = gtk::DropDown::from_strings(&mode_names);
        mode_dd.set_selected(MODES.iter().position(|(k, _)| *k == s.borrow().default_runtime_mode).unwrap_or(0) as u32);
        let s2 = s.clone();
        mode_dd.connect_selected_notify(move |d| {
            let mut st = s2.borrow_mut();
            st.default_runtime_mode = MODES[d.selected() as usize].0.into();
            st.save();
        });
        defaults.append(&row(
            "Default permission mode",
            Some("Supervised (ask before actions) unless you choose otherwise"),
            &mode_dd,
        ));
        if !ctx.projects.is_empty() {
            defaults.append(&label("Per project", &["title-4"]));
            for (pid, title) in &ctx.projects {
                let pm = gtk::DropDown::from_strings(
                    &std::iter::once("Use default").chain(mode_names.iter().copied()).collect::<Vec<_>>(),
                );
                let o = s.borrow().projects.get(pid).and_then(|p| p.runtime_mode.clone());
                pm.set_selected(
                    o.and_then(|m| MODES.iter().position(|(k, _)| *k == m)).map(|i| i as u32 + 1).unwrap_or(0),
                );
                let s2 = s.clone();
                let pid2 = pid.clone();
                pm.connect_selected_notify(move |d| {
                    let mut st = s2.borrow_mut();
                    let entry = st.projects.entry(pid2.clone()).or_default();
                    entry.runtime_mode =
                        if d.selected() == 0 { None } else { Some(MODES[d.selected() as usize - 1].0.into()) };
                    st.save();
                });
                defaults.append(&row(title, Some("Permission mode for new threads in this project"), &pm));
            }
        }
        stack.add_titled(&defaults, Some("defaults"), "Defaults");

        // ---- Server --------------------------------------------------------------------------
        let server = page(
            "Server",
            "The T3 Code server this window talks to. Pairing stores a session credential in the keyring.",
        );
        let state = match (&ctx.server, &ctx.server_label) {
            (Some(url), Some(l)) => format!("Paired with {l} at {url}"),
            (Some(url), None) => format!("Paired with {url}"),
            _ => "Not paired".to_string(),
        };
        server.append(&row("Connection", Some(&state), &gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        let pair = gtk::Button::with_label(if ctx.server.is_some() { "Pair again…" } else { "Pair…" });
        let on_pair = ctx.on_pair.clone();
        pair.connect_clicked(move |_| on_pair());
        server.append(&row("Pairing", Some("Run `t3 pair` on the server and paste the token"), &pair));
        if ctx.server.is_some() {
            let unpair = gtk::Button::with_label("Forget pairing");
            unpair.add_css_class("destructive-action");
            let on_unpair = ctx.on_unpair.clone();
            unpair.connect_clicked(move |_| on_unpair());
            server.append(&row(
                "Unpair",
                Some("Removes the stored credential; the server keeps its own record until revoked there"),
                &unpair,
            ));
            let mcp = format!("{}/mcp", ctx.server.clone().unwrap_or_default());
            let copy = gtk::Button::with_label("Copy");
            let m = mcp.clone();
            copy.connect_clicked(move |b| b.clipboard().set_text(&m));
            server.append(&row(
                "MCP endpoint",
                Some(&format!("{mcp} · for agents the server did not launch (OAuth sign-in)")),
                &copy,
            ));
        }
        server.append(&row(
            "Keyring",
            Some(if ctx.keyring {
                "secret-tool found; credentials go to the keyring"
            } else {
                "No secret-tool; credentials go to ~/.config/claude-pantheon-rs (0600)"
            }),
            &gtk::Box::new(gtk::Orientation::Horizontal, 0),
        ));
        stack.add_titled(&server, Some("server"), "Server");

        // ---- Notifications -------------------------------------------------------------------
        let notes = page("Notifications", "Shown only while the window is not the active one.");
        for (title, sub, get, set) in [
            (
                "Turn finished",
                "When a thread's run completes",
                Box::new(|s: &Settings| s.notify_turn_finished) as Box<dyn Fn(&Settings) -> bool>,
                Box::new(|s: &mut Settings, v| s.notify_turn_finished = v) as Box<dyn Fn(&mut Settings, bool)>,
            ),
            (
                "Permission prompts",
                "When a thread waits for your answer",
                Box::new(|s: &Settings| s.notify_prompts),
                Box::new(|s: &mut Settings, v| s.notify_prompts = v),
            ),
            (
                "Dock badge",
                "Running threads on the dock icon, with a progress bar while a turn runs",
                Box::new(|s: &Settings| s.dock_badge),
                Box::new(|s: &mut Settings, v| s.dock_badge = v),
            ),
        ] {
            let sw = gtk::Switch::builder().active(get(&s.borrow())).build();
            let s2 = s.clone();
            sw.connect_state_set(move |_, v| {
                let mut st = s2.borrow_mut();
                set(&mut st, v);
                st.save();
                glib::Propagation::Proceed
            });
            notes.append(&row(title, Some(sub), &sw));
        }
        stack.add_titled(&notes, Some("notifications"), "Notifications");

        // ---- Advanced ------------------------------------------------------------------------
        let advanced = page(
            "Advanced",
            "The cairo renderer draws this window in a third of the memory of the GL renderers on Mesa/Asahi (docs/PERFORMANCE.md). Changes apply at the next launch.",
        );
        let renderers = ["cairo", "gl", "ngl", "vulkan"];
        let rd = gtk::DropDown::from_strings(&renderers);
        rd.set_selected(renderers.iter().position(|r| *r == s.borrow().renderer).unwrap_or(0) as u32);
        let s2 = s.clone();
        rd.connect_selected_notify(move |d| {
            let mut st = s2.borrow_mut();
            st.renderer = renderers[d.selected() as usize].into();
            st.save();
        });
        advanced.append(&row(
            "Renderer",
            Some("GSK renderer; CLAUDE_PANTHEON_RENDERER in the environment still wins"),
            &rd,
        ));
        let open_config = gtk::Button::with_label("Open folder");
        open_config.connect_clicked(|_| {
            if let Some(dir) = path().parent() {
                gtk::FileLauncher::new(Some(&gio::File::for_path(dir))).launch(
                    gtk::Window::NONE,
                    gio::Cancellable::NONE,
                    |_| {},
                );
            }
        });
        advanced.append(&row(
            "Configuration",
            Some(&crate::look::tilde(&path().parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())),
            &open_config,
        ));
        stack.add_titled(&advanced, Some("advanced"), "Advanced");

        window.present();
        Rc::new(SettingsWindow { window, settings })
    }

    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let mut s = Settings::default();
        assert_eq!(s.default_runtime_mode, "approval-required");
        assert_eq!(s.renderer, "cairo");
        s.default_provider = Some("codex".into());
        s.projects.insert("p1".into(), ProjectDefaults { provider: None, runtime_mode: Some("auto".into()) });
        assert_eq!(s.defaults_for("p1"), (Some("codex".into()), "auto".into()));
        assert_eq!(s.defaults_for("other"), (Some("codex".into()), "approval-required".into()));
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_provider.as_deref(), Some("codex"));
    }
}
