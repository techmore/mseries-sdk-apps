//! The host: owns the document, the store and the logic; renders a surface into a window; runs services;
//! persists state; reloads the description when the file changes.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};

use crate::logic::{Effect, Logic, RulesLogic, Store, default_for};
use crate::render::{self, D, Dispatcher, Refresh};

type Event = (String, Option<Value>);

pub struct Host {
    doc: RefCell<Value>,
    store: RefCell<Store>,
    logic: RefCell<Box<dyn Logic>>,
    custom_logic: bool,
    refreshers: RefCell<Vec<Refresh>>,
    window: gtk::ApplicationWindow,
    holder: gtk::Box,
    toast: gtk::Label,
    banner: gtk::Label,
    tx: UnboundedSender<Event>,
    path: PathBuf,
    surface: String,
    mtime: Cell<Option<SystemTime>>,
    me: RefCell<Weak<Host>>,
}

fn read_doc(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("line {}: {e}", e.line()))
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn state_file(doc: &Value) -> Option<PathBuf> {
    let id = doc["app"]["id"].as_str()?;
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join(id).join("state.json"))
}

fn initial_store(doc: &Value) -> Store {
    let mut st = Store::new();
    for (k, t) in doc
        .get("state")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        st.insert(k.clone(), default_for(t.as_str().unwrap_or("text")));
    }
    if let Some(f) = state_file(doc)
        && let Ok(text) = std::fs::read_to_string(f)
        && let Ok(Value::Object(saved)) = serde_json::from_str(&text)
    {
        for k in doc
            .get("persist")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(v) = saved.get(k) {
                st.insert(k.to_string(), v.clone());
            }
        }
    }
    st
}

fn report(diags: &[msdk::Diagnostic]) -> bool {
    for d in diags {
        eprintln!("{d}");
    }
    diags.iter().any(|d| d.severity == msdk::Severity::Error)
}

impl Host {
    /// Opens a window for `surface` of the description at `path`. Returns None when the description has errors.
    pub fn open(
        app: &gtk::Application,
        path: &Path,
        surface: &str,
        logic: Option<Box<dyn Logic>>,
        watch: bool,
    ) -> Option<Rc<Host>> {
        let doc = match read_doc(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return None;
            }
        };
        if report(&msdk::check(&doc)) {
            return None;
        }
        if doc["surfaces"].get(surface).is_none() {
            eprintln!("{}: no surface `{surface}`", path.display());
            return None;
        }
        if let Some(display) = gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(&msdk::tokens::css());
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let (tx, mut rx) = unbounded::<Event>();
        let custom_logic = logic.is_some();
        let logic: Box<dyn Logic> = logic.unwrap_or_else(|| Box::new(RulesLogic::new(&doc)));
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .default_width(900)
            .default_height(620)
            .build();
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_vexpand(true);
        let toast = gtk::Label::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(24)
            .visible(false)
            .build();
        toast.add_css_class("app-notification");
        let banner = gtk::Label::builder()
            .halign(gtk::Align::Fill)
            .wrap(true)
            .xalign(0.0)
            .visible(false)
            .build();
        banner.add_css_class("msdk-error");
        let overlay = gtk::Overlay::new();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&banner);
        column.append(&holder);
        overlay.set_child(Some(&column));
        overlay.add_overlay(&toast);
        window.set_child(Some(&overlay));
        let host = Rc::new(Host {
            store: RefCell::new(initial_store(&doc)),
            doc: RefCell::new(doc),
            logic: RefCell::new(logic),
            custom_logic,
            refreshers: RefCell::new(Vec::new()),
            window,
            holder,
            toast,
            banner,
            tx,
            path: path.to_path_buf(),
            surface: surface.to_string(),
            mtime: Cell::new(mtime(path)),
            me: RefCell::new(Weak::new()),
        });
        *host.me.borrow_mut() = Rc::downgrade(&host);
        host.rebuild();
        let h = host.clone();
        glib::spawn_future_local(async move {
            while let Some((msg, payload)) = rx.next().await {
                h.send(&msg, payload);
            }
        });
        if watch {
            let h = host.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                let now = mtime(&h.path);
                if now != h.mtime.get() {
                    h.mtime.set(now);
                    h.reload();
                }
                glib::ControlFlow::Continue
            });
        }
        host.window.present();
        Some(host)
    }

    fn dispatcher(&self) -> D {
        self.me.borrow().upgrade().expect("host alive") as D
    }

    /// Builds the surface's widgets from the current document.
    fn rebuild(&self) {
        let doc = self.doc.borrow().clone();
        let surf = &doc["surfaces"][&self.surface];
        let kind = surf["kind"].as_str().unwrap_or("window");
        let d = self.dispatcher();
        while let Some(c) = self.holder.first_child() {
            self.holder.remove(&c);
        }
        let mut refreshers: Vec<Refresh> = Vec::new();
        let title = surf["title"]
            .as_str()
            .or(doc["app"]["name"].as_str())
            .unwrap_or("");
        self.window.set_title(Some(title));
        match kind {
            "window" | "dialog" => {
                let header = gtk::HeaderBar::new();
                for n in surf["header"]
                    .as_array()
                    .or(surf["actions"].as_array())
                    .into_iter()
                    .flatten()
                {
                    let b = render::build_node(n, &d);
                    header.pack_start(&b.widget);
                    refreshers.extend(b.refresh);
                }
                self.window.set_titlebar(Some(&header));
                self.window.set_decorated(true);
                self.window.unfullscreen();
            }
            "fullscreen" => {
                self.window.set_decorated(false);
                self.window.fullscreen();
            }
            _ => {
                // panel and popover: undecorated; anchoring to the screen edge needs the Pantheon shell protocol client.
                self.window.set_decorated(false);
                let h = surf["size"].as_i64().unwrap_or(64) as i32;
                if kind == "panel" {
                    self.window.set_default_size(900, h);
                }
            }
        }
        if let Some(body) = surf.get("body") {
            let b = render::build_node(body, &d);
            b.widget.set_vexpand(true);
            self.holder.append(&b.widget);
            refreshers.extend(b.refresh);
        }
        *self.refreshers.borrow_mut() = refreshers;
        self.refresh();
    }

    fn refresh(&self) {
        let snapshot = self.store.borrow().clone();
        let rs = self.refreshers.borrow().clone();
        for r in &rs {
            r(&snapshot);
        }
    }

    /// Re-reads the file. A description with errors keeps the previous UI and shows the first error in a banner.
    pub fn reload(&self) {
        let doc = match read_doc(&self.path) {
            Ok(d) => d,
            Err(e) => return self.show_error(&e),
        };
        let diags = msdk::check(&doc);
        if let Some(e) = diags.iter().find(|d| d.severity == msdk::Severity::Error) {
            report(&diags);
            return self.show_error(&e.to_string());
        }
        self.banner.set_visible(false);
        {
            let mut st = self.store.borrow_mut();
            let fresh = initial_store(&doc);
            for (k, v) in fresh {
                st.entry(k).or_insert(v);
            }
            let declared: Vec<String> = doc
                .get("state")
                .and_then(Value::as_object)
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            st.retain(|k, _| declared.contains(k));
        }
        if !self.custom_logic {
            *self.logic.borrow_mut() = Box::new(RulesLogic::new(&doc));
        }
        *self.doc.borrow_mut() = doc;
        self.rebuild();
    }

    fn show_error(&self, text: &str) {
        self.banner.set_text(text);
        self.banner.set_visible(true);
    }

    fn show_toast(&self, text: &str) {
        self.toast.set_text(text);
        self.toast.set_visible(true);
        let t = self.toast.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            t.set_visible(false)
        });
    }

    fn persist(&self) {
        let doc = self.doc.borrow();
        let keys: Vec<&str> = doc
            .get("persist")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if keys.is_empty() {
            return;
        }
        let st = self.store.borrow();
        let saved: serde_json::Map<String, Value> = keys
            .iter()
            .filter_map(|k| st.get(*k).map(|v| ((*k).to_string(), v.clone())))
            .collect();
        if let Some(f) = state_file(&doc) {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(f, Value::Object(saved).to_string());
        }
    }

    fn apply(&self, e: Effect) {
        match e {
            Effect::Toast(t) => self.show_toast(&t),
            Effect::Quit => self.window.close(),
            Effect::Open(_) | Effect::Close(_) => {
                eprintln!("msdk-run: open/close of other surfaces is not implemented yet")
            }
            Effect::Call {
                service,
                args,
                then,
                error,
            } => {
                let tx = self.tx.clone();
                let fail =
                    move |tx: &UnboundedSender<Event>, error: &Option<String>, why: String| {
                        if let Some(m) = error {
                            let _ = tx.unbounded_send((m.clone(), Some(json!(why))));
                        }
                    };
                match service.as_str() {
                    "process.run" => {
                        let cmd = args
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        std::thread::spawn(move || {
                            match std::process::Command::new("sh")
                                .arg("-c")
                                .arg(&cmd)
                                .output()
                            {
                                Ok(o) if o.status.success() => {
                                    if let Some(m) = then {
                                        let _ = tx.unbounded_send((
                                            m,
                                            Some(json!(
                                                String::from_utf8_lossy(&o.stdout)
                                                    .trim_end()
                                                    .to_string()
                                            )),
                                        ));
                                    }
                                }
                                Ok(o) => fail(
                                    &tx,
                                    &error,
                                    String::from_utf8_lossy(&o.stderr).trim().to_string(),
                                ),
                                Err(e) => fail(&tx, &error, e.to_string()),
                            }
                        });
                    }
                    "power.poweroff" | "power.reboot" => {
                        let verb = if service.ends_with("poweroff") {
                            "poweroff"
                        } else {
                            "reboot"
                        };
                        let _ = std::process::Command::new("systemctl").arg(verb).spawn();
                    }
                    other => fail(
                        &tx,
                        &error,
                        format!("service {other} is not available in this runtime yet"),
                    ),
                }
            }
        }
    }
}

impl Dispatcher for Host {
    fn send(&self, msg: &str, payload: Option<Value>) {
        let effects = {
            let mut st = self.store.borrow_mut();
            self.logic.borrow_mut().update(&mut st, msg, payload)
        };
        for e in effects {
            self.apply(e);
        }
        self.persist();
        self.refresh();
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.store.borrow().get(key).cloned()
    }

    fn set(&self, key: &str, v: Value) {
        self.store.borrow_mut().insert(key.to_string(), v);
    }

    fn confirm(&self, text: &str, then: Box<dyn FnOnce()>) {
        let dlg = gtk::AlertDialog::builder()
            .message(text)
            .buttons(["Cancel", "Continue"])
            .cancel_button(0)
            .default_button(1)
            .build();
        dlg.choose(Some(&self.window), gtk::gio::Cancellable::NONE, move |r| {
            if r == Ok(1) {
                then();
            }
        });
    }
}
