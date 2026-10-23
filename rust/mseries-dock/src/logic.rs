//! Dock logic for the SDK renderer: pins plus the window feed become the item list; activating an item focuses or launches.

use std::path::PathBuf;

use msdk::logic::{Effect, Logic, Store};
use serde_json::{Value, json};

use crate::apps::{AppEntry, Resolver};

/// Pinned desktop ids used when the user has no pins file, filtered to what is installed.
pub const DEFAULT_PINS: &[&str] = &[
    "io.elementary.files",
    "brave-browser",
    "com.mitchellh.ghostty",
    "io.elementary.terminal",
    "claude-pantheon-rs",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub indicator: &'static str,
    pub badge: String,
    /// Window ids of this item, focused first.
    pub windows: Vec<String>,
    pub pinned: bool,
}

pub struct DockLogic {
    resolver: Resolver,
    pins: Vec<String>,
    items: Vec<Item>,
}

pub fn pins_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mseries/dock-pins.json"))
}

pub fn load_pins(resolver: &Resolver) -> Vec<String> {
    if let Some(f) = pins_file()
        && let Ok(text) = std::fs::read_to_string(f)
        && let Ok(ids) = serde_json::from_str::<Vec<String>>(&text)
    {
        return ids;
    }
    DEFAULT_PINS
        .iter()
        .filter(|id| resolver.by_id(id).is_some())
        .map(|s| (*s).to_string())
        .collect()
}

impl DockLogic {
    pub fn new(entries: Vec<AppEntry>, pins: Vec<String>) -> DockLogic {
        DockLogic {
            resolver: Resolver::new(entries),
            pins,
            items: Vec::new(),
        }
    }

    pub fn from_system() -> DockLogic {
        let resolver = Resolver::new(crate::apps::load_installed());
        let pins = load_pins(&resolver);
        DockLogic {
            resolver,
            pins,
            items: Vec::new(),
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Builds the item list: pinned apps in order (running or not), then running apps that are not pinned.
    pub fn build(&self, windows: &[Value]) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let slot =
            |items: &mut Vec<Item>, id: &str, name: &str, icon: &str, pinned: bool| -> usize {
                if let Some(i) = items.iter().position(|it| it.id == id) {
                    return i;
                }
                items.push(Item {
                    id: id.into(),
                    name: name.into(),
                    icon: icon.into(),
                    indicator: "none",
                    badge: String::new(),
                    windows: vec![],
                    pinned,
                });
                items.len() - 1
            };
        for pin in &self.pins {
            if let Some(e) = self.resolver.by_id(pin) {
                slot(&mut items, &e.id, &e.name, &e.icon, true);
            }
        }
        for w in windows {
            let app_id = w["app_id"].as_str().unwrap_or("");
            let i = match self.resolver.resolve(app_id) {
                Some(e) => slot(
                    &mut items,
                    &e.id,
                    &e.name,
                    &e.icon,
                    self.pins
                        .iter()
                        .any(|p| e.short_id() == p.strip_suffix(".desktop").unwrap_or(p)),
                ),
                None => slot(
                    &mut items,
                    &format!("tmp:{app_id}"),
                    if app_id.is_empty() { "Window" } else { app_id },
                    "application-x-executable",
                    false,
                ),
            };
            let (focused, attention) = (
                w["focused"].as_bool().unwrap_or(false),
                w["attention"].as_bool().unwrap_or(false),
            );
            let id = w["id"].as_str().unwrap_or("").to_string();
            let it = &mut items[i];
            if focused {
                it.windows.insert(0, id);
                it.indicator = "focused";
            } else {
                it.windows.push(id);
                if it.indicator == "none" {
                    it.indicator = "open";
                }
            }
            if attention {
                it.badge = "!".into();
            }
        }
        items
    }

    fn refresh(&mut self, store: &mut Store) {
        let windows = store
            .get("windows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.items = self.build(&windows);
        let rows: Vec<Value> = self.items.iter().map(|i| json!({ "id": i.id, "name": i.name, "icon": i.icon, "indicator": i.indicator, "badge": i.badge })).collect();
        store.insert("items".into(), Value::Array(rows));
    }

    /// Clicking an item: focus its next window (cycling when several), or launch a pinned app that is not running.
    fn activate(&self, item_id: &str) -> Vec<Effect> {
        let Some(it) = self.items.iter().find(|i| i.id == item_id) else {
            return vec![];
        };
        let call = |service: &str, key: &str, val: &str| Effect::Call {
            service: service.into(),
            args: [(key.to_string(), json!(val))].into_iter().collect(),
            then: None,
            error: None,
        };
        if it.windows.is_empty() {
            return if it.pinned {
                vec![call("windows.launch", "app", &it.id)]
            } else {
                vec![]
            };
        }
        // `windows` lists the focused window first; focusing the second cycles through the app's windows.
        let target = if it.indicator == "focused" && it.windows.len() > 1 {
            &it.windows[1]
        } else {
            &it.windows[0]
        };
        vec![call("windows.activate", "id", target)]
    }
}

impl Logic for DockLogic {
    fn update(&mut self, store: &mut Store, msg: &str, payload: Option<Value>) -> Vec<Effect> {
        match msg {
            "_start" | "_feed" => {
                self.refresh(store);
                vec![]
            }
            "Activate" => {
                let id = payload
                    .and_then(|p| p.as_str().map(String::from))
                    .unwrap_or_default();
                self.activate(&id)
            }
            _ => vec![],
        }
    }
}
