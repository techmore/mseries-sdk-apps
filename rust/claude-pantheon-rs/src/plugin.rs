//! Plugins: one core (the T3 connection, window, palette, status feed), many tools.
//! A plugin owns one page and may add palette entries and lines to the Wingpanel feed.
//! Built-ins are compiled in and can be switched off in `~/.config/claude-pantheon/plugins.json`
//! (`{"disabled": ["accounts"]}`), so an idle plugin costs nothing: its widget is built on first show.

use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::app::App;
use crate::repos::ReposPage;

pub struct StatusLine {
    pub label: String,
    pub detail: String,
}

pub trait Plugin {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn subtitle(&self) -> &'static str;
    fn icon(&self) -> &'static str;
    /// The page body; called once, on first show.
    fn build(&self) -> gtk::Widget;
    /// Built-in pages that already have their own navigation take over here; return true when handled.
    fn open(&self, _app: &Rc<App>) -> bool {
        false
    }
    /// Called every time the page is shown.
    fn refresh(&self) {}
    /// Lines for the Wingpanel indicator feed (`status.json` → `plugins.<id>`).
    fn status(&self) -> Vec<StatusLine> {
        Vec::new()
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("claude-pantheon").join("plugins.json")
}

pub fn disabled_ids() -> Vec<String> {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|v| disabled_from(&v))
        .unwrap_or_default()
}

/// Built-in plugins as (id, title); the menu is built before the registry exists.
pub const BUILTIN: &[(&str, &str)] = &[
    ("usage", "Usage"),
    ("repos", "Repos and Issues"),
    ("changes", "Changes in This Thread"),
    ("diagnostics", "Diagnostics"),
    ("accounts", "Accounts"),
];

/// Menu items `(title, detailed action)` for the enabled built-ins.
pub fn menu_items() -> Vec<(String, String)> {
    let off = disabled_ids();
    BUILTIN
        .iter()
        .filter(|(id, _)| !off.iter().any(|d| d == id))
        .map(|(id, title)| {
            // Usage keeps its own action so the menu still shows the Ctrl+Shift+U accelerator.
            let action = if *id == "usage" { "win.usage".to_string() } else { format!("win.plugin::{id}") };
            (title.to_string(), action)
        })
        .collect()
}

pub fn disabled_from(config: &Value) -> Vec<String> {
    config
        .get("disabled")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub struct Registry {
    plugins: Vec<Rc<dyn Plugin>>,
    built: RefCell<Vec<&'static str>>,
}

impl Registry {
    pub fn new(plugins: Vec<Rc<dyn Plugin>>, disabled: &[String]) -> Registry {
        let plugins = plugins.into_iter().filter(|p| !disabled.iter().any(|d| d == p.id())).collect();
        Registry { plugins, built: RefCell::new(Vec::new()) }
    }

    pub fn builtin(repos: Rc<ReposPage>) -> Registry {
        let mut plugins: Vec<Rc<dyn Plugin>> = vec![
            Rc::new(crate::plugin_repos::Repos::new(repos)),
            Rc::new(crate::plugin_accounts::Accounts::new()),
        ];
        plugins.extend(crate::plugin_pages::builtin());
        Registry::new(plugins, &disabled_ids())
    }

    pub fn all(&self) -> &[Rc<dyn Plugin>] {
        &self.plugins
    }

    pub fn get(&self, id: &str) -> Option<Rc<dyn Plugin>> {
        self.plugins.iter().find(|p| p.id() == id).cloned()
    }

    /// The widget for a plugin the first time it is needed; `None` afterwards (it is already in the stack).
    pub fn build_once(&self, id: &str) -> Option<gtk::Widget> {
        let p = self.get(id)?;
        if self.built.borrow().contains(&p.id()) {
            return None;
        }
        self.built.borrow_mut().push(p.id());
        Some(p.build())
    }

    pub fn status(&self) -> Value {
        let mut out = serde_json::Map::new();
        for p in &self.plugins {
            let lines: Vec<Value> =
                p.status().into_iter().map(|l| json!({"label": l.label, "detail": l.detail})).collect();
            if !lines.is_empty() {
                out.insert(p.id().to_string(), Value::Array(lines));
            }
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str);
    impl Plugin for Fake {
        fn id(&self) -> &'static str {
            self.0
        }
        fn title(&self) -> &'static str {
            "Fake"
        }
        fn subtitle(&self) -> &'static str {
            ""
        }
        fn icon(&self) -> &'static str {
            "dialog-information-symbolic"
        }
        fn build(&self) -> gtk::Widget {
            unreachable!("no display in unit tests")
        }
        fn status(&self) -> Vec<StatusLine> {
            vec![StatusLine { label: "a".into(), detail: "b".into() }]
        }
    }

    #[test]
    fn disabled_plugins_are_not_registered() {
        let r = Registry::new(vec![Rc::new(Fake("one")), Rc::new(Fake("two"))], &["one".to_string()]);
        assert!(r.get("one").is_none());
        assert!(r.get("two").is_some());
        assert_eq!(r.all().len(), 1);
    }

    #[test]
    fn status_is_keyed_by_plugin_id() {
        let r = Registry::new(vec![Rc::new(Fake("two"))], &[]);
        assert_eq!(r.status()["two"][0]["label"], "a");
    }

    #[test]
    fn builtin_ids_have_menu_actions() {
        for (id, _) in BUILTIN {
            let wanted = if *id == "usage" { "win.usage".to_string() } else { format!("win.plugin::{id}") };
            assert!(menu_items().iter().any(|(_, a)| *a == wanted) || disabled_ids().iter().any(|d| d == id));
        }
    }

    #[test]
    fn config_lists_disabled_ids() {
        assert_eq!(disabled_from(&json!({"disabled": ["x", 3, "y"]})), vec!["x", "y"]);
        assert!(disabled_from(&json!({})).is_empty());
    }
}
