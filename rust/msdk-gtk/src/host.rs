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

use crate::logic::{Effect, Logic, RulesLogic, Store};
use crate::render::{self, D, Dispatcher, Refresh};

/// A message from outside the widgets (a worker thread, a service): name and payload.
pub type Event = (String, Option<Value>);

/// Builds the app's Rust logic once the host exists; it gets a sender for events from its own threads.
/// System access an app can plug in: named services (`call`) and live feeds. Crates such as the T3 client implement it;
/// `FixtureServices` answers from a JSON file so an app can run before its real backend exists.
pub trait Services {
    /// Handles `service`; returns false when this provider does not own it. Replies go to `tx` as events.
    fn call(
        &self,
        service: &str,
        args: &std::collections::BTreeMap<String, Value>,
        then: Option<String>,
        error: Option<String>,
        tx: UnboundedSender<Event>,
    ) -> bool;
    /// Starts `feed`, calling `push` with the whole list now and whenever it changes. False when not owned.
    fn start_feed(&self, feed: &str, push: Rc<dyn Fn(Value)>) -> bool;
}

pub type LogicFactory = Box<dyn FnOnce(UnboundedSender<Event>) -> Box<dyn Logic>>;

pub struct Host {
    doc: RefCell<Value>,
    store: RefCell<Store>,
    logic: RefCell<Box<dyn Logic>>,
    custom_logic: bool,
    services: Option<Rc<dyn Services>>,
    extra: RefCell<std::collections::HashMap<String, gtk::Window>>,
    refreshers: RefCell<Vec<Refresh>>,
    window: gtk::ApplicationWindow,
    holder: gtk::Box,
    toast: gtk::Label,
    banner: gtk::Label,
    tx: UnboundedSender<Event>,
    path: Option<PathBuf>,
    surface: String,
    mtime: Cell<Option<SystemTime>>,
    me: RefCell<Weak<Host>>,
}

fn read_doc(path: &Path) -> Result<Value, String> {
    msdk::text::load(path).map(|(doc, _)| doc)
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
    let mut st = msdk::logic::initial_store(doc);
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
        logic: Option<LogicFactory>,
        watch: bool,
    ) -> Option<Rc<Host>> {
        let doc = match read_doc(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return None;
            }
        };
        Host::open_doc(app, doc, Some(path), surface, logic, watch)
    }

    /// Opens a window for `surface` of an in-memory description (for example one embedded in the binary).
    pub fn open_doc(
        app: &gtk::Application,
        doc: Value,
        path: Option<&Path>,
        surface: &str,
        logic: Option<LogicFactory>,
        watch: bool,
    ) -> Option<Rc<Host>> {
        Host::open_with(app, doc, path, surface, logic, None, watch)
    }

    /// As [`Host::open_doc`], with a plug-in services provider.
    pub fn open_with(
        app: &gtk::Application,
        doc: Value,
        path: Option<&Path>,
        surface: &str,
        logic: Option<LogicFactory>,
        services: Option<Rc<dyn Services>>,
        watch: bool,
    ) -> Option<Rc<Host>> {
        if report(&msdk::check(&doc)) {
            return None;
        }
        if doc["surfaces"].get(surface).is_none() {
            eprintln!("no surface `{surface}`");
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
        let logic: Box<dyn Logic> = match logic {
            Some(make) => make(tx.clone()),
            None => Box::new(RulesLogic::new(&doc)),
        };
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
        window.add_css_class("msdk-app");
        window.set_child(Some(&overlay));
        let host = Rc::new(Host {
            store: RefCell::new(initial_store(&doc)),
            doc: RefCell::new(doc),
            logic: RefCell::new(logic),
            custom_logic,
            services,
            extra: RefCell::new(Default::default()),

            refreshers: RefCell::new(Vec::new()),
            window,
            holder,
            toast,
            banner,
            tx,
            path: path.map(Path::to_path_buf),
            surface: surface.to_string(),
            mtime: Cell::new(path.and_then(mtime)),
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
        host.start_feeds();
        if watch && host.path.is_some() {
            let h = host.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                let now = h.path.as_deref().and_then(mtime);
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

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    /// Sends a message as if a control had (used for start-up messages and feeds).
    pub fn post(&self, msg: &str, payload: Option<Value>) {
        Dispatcher::send(self, msg, payload);
    }

    /// Starts the live lists the description declares under `feeds`.
    fn start_feeds(&self) {
        let feeds = self
            .doc
            .borrow()
            .get("feeds")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, name) in feeds {
            if let Some(feed) = name.as_str()
                && feed != "windows.list"
                && let Some(sv) = &self.services
            {
                let (weak, key) = (self.me.borrow().clone(), key.clone());
                let started = sv.start_feed(
                    feed,
                    Rc::new(move |list| {
                        if let Some(h) = weak.upgrade() {
                            h.set(&key, list);
                            h.post("_feed", Some(json!(key)));
                        }
                    }),
                );
                if !started {
                    eprintln!("msdk-run: nothing provides the feed `{feed}`");
                }
                continue;
            }
            if name.as_str() == Some("windows.list") {
                let weak = self.me.borrow().clone();
                crate::services::watch_windows(move |list| {
                    if let Some(h) = weak.upgrade() {
                        h.set(&key, list);
                        h.post("_feed", Some(json!(key)));
                    }
                });
            }
        }
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
                // The standard chrome: a slightly toned silver band, window controls at the top left, an options
                // menu beside them, the title in the middle and the app's own actions on the right.
                let header = gtk::HeaderBar::new();
                header.add_css_class("msdk-titlebar");
                header.set_decoration_layout(Some("close,minimize,maximize:"));
                header.pack_start(&self.options_button(&doc));
                let items: Vec<&Value> = surf["header"]
                    .as_array()
                    .or(surf["actions"].as_array())
                    .into_iter()
                    .flatten()
                    .collect();
                for n in items.into_iter().rev() {
                    let b = render::build_node(n, &d);
                    if n["type"] == "Tabs" {
                        header.set_title_widget(Some(&b.widget));
                    } else {
                        header.pack_end(&b.widget);
                    }
                    refreshers.extend(b.refresh);
                }
                self.window.set_titlebar(Some(&header));
                self.window.set_decorated(true);
                self.window.unfullscreen();
            }
            "fullscreen" => {
                self.window.set_decorated(false);
                self.window.add_css_class("msdk-kiosk");
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
            if kind == "fullscreen" {
                // A centred card, the shape of a login or lock screen.
                b.widget.set_halign(gtk::Align::Center);
                b.widget.set_valign(gtk::Align::Center);
                b.widget.set_size_request(360, -1);
                b.widget.add_css_class("msdk-card");
            }
            b.widget.set_vexpand(true);
            if matches!(kind, "window" | "dialog") {
                b.widget
                    .set_margin_start(msdk::tokens::layout_px("content_padding_x"));
                b.widget
                    .set_margin_end(msdk::tokens::layout_px("content_padding_x"));
                b.widget
                    .set_margin_top(msdk::tokens::layout_px("content_padding_y"));
                b.widget
                    .set_margin_bottom(msdk::tokens::layout_px("content_padding_y"));
            }
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
        let Some(path) = &self.path else { return };
        let doc = match read_doc(path) {
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

    /// The options menu beside the window controls: Settings (when the app has a `settings` dialog) and About.
    fn options_button(&self, doc: &Value) -> gtk::MenuButton {
        let button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .build();
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Options"));
        button.update_property(&[gtk::accessible::Property::Label("Options")]);
        let popover = gtk::Popover::new();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let item = |label: &str| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            b.set_halign(gtk::Align::Fill);
            b
        };
        if doc["surfaces"]["settings"]["kind"] == "dialog" {
            let b = item("Settings…");
            let (weak, pop) = (self.me.borrow().clone(), popover.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                if let Some(h) = weak.upgrade() {
                    h.open_surface("settings");
                }
            });
            column.append(&b);
        }
        let about = item("About");
        let (weak, pop) = (self.me.borrow().clone(), popover.clone());
        about.connect_clicked(move |_| {
            pop.popdown();
            if let Some(h) = weak.upgrade() {
                h.show_about();
            }
        });
        column.append(&about);
        popover.set_child(Some(&column));
        button.set_popover(Some(&popover));
        button
    }

    fn show_about(&self) {
        let doc = self.doc.borrow();
        let app = &doc["app"];
        let about = gtk::AboutDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .program_name(app["name"].as_str().unwrap_or(""))
            .logo_icon_name(app["icon"].as_str().unwrap_or("application-x-executable"))
            .build();
        if let Some(v) = app["version"].as_str() {
            about.set_version(Some(v));
        }
        if let Some(w) = app["website"].as_str() {
            about.set_website(Some(w));
        }
        if app["license"].is_string() {
            about.set_license_type(gtk::License::Gpl30);
        }
        about.present();
    }

    /// Renders the window to a PNG (the `shot` call of the introspection API).
    pub fn screenshot(&self, path: &Path) -> Result<(), String> {
        let paintable = gtk::WidgetPaintable::new(Some(&self.window));
        let (w, h) = (self.window.width(), self.window.height());
        if w <= 0 || h <= 0 {
            return Err("the window has no size yet".into());
        }
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(snapshot.upcast_ref::<gdk::Snapshot>(), w as f64, h as f64);
        let node = snapshot.to_node().ok_or("nothing to draw")?;
        let renderer = self
            .window
            .native()
            .and_then(|n| n.renderer())
            .ok_or("no renderer")?;
        renderer
            .render_texture(&node, None)
            .save_to_png(path)
            .map_err(|e| e.to_string())
    }

    /// Opens a `dialog` or `popover` surface in its own window, transient for the main one. The store is shared.
    fn open_surface(&self, name: &str) {
        if let Some(w) = self.extra.borrow().get(name) {
            w.present();
            return;
        }
        let doc = self.doc.borrow().clone();
        let Some(surf) = doc["surfaces"].get(name) else {
            return eprintln!("msdk-run: no surface `{name}`");
        };
        let kind = surf["kind"].as_str().unwrap_or("");
        if !matches!(kind, "dialog" | "popover") {
            return eprintln!(
                "msdk-run: `{name}` is a {kind}; only dialogs and popovers open on demand"
            );
        }
        let dialog = kind == "dialog";
        let win = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(dialog)
            .decorated(dialog)
            .title(surf["title"].as_str().unwrap_or(name))
            .default_width(360)
            .build();
        let d = self.dispatcher();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        column.set_margin_top(16);
        column.set_margin_bottom(16);
        column.set_margin_start(16);
        column.set_margin_end(16);
        let mut refreshers: Vec<Refresh> = Vec::new();
        if let Some(body) = surf.get("body") {
            let b = render::build_node(body, &d);
            column.append(&b.widget);
            refreshers.extend(b.refresh);
        }
        if dialog {
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            actions.set_halign(gtk::Align::End);
            for n in surf["actions"].as_array().into_iter().flatten() {
                let b = render::build_node(n, &d);
                actions.append(&b.widget);
                refreshers.extend(b.refresh);
            }
            column.append(&actions);
        }
        win.set_child(Some(&column));
        let snapshot = self.store.borrow().clone();
        for r in &refreshers {
            r(&snapshot);
        }
        self.refreshers.borrow_mut().extend(refreshers);
        let (weak, key) = (self.me.borrow().clone(), name.to_string());
        win.connect_close_request(move |_| {
            if let Some(h) = weak.upgrade() {
                h.extra.borrow_mut().remove(&key);
            }
            gtk::glib::Propagation::Proceed
        });
        self.extra
            .borrow_mut()
            .insert(name.to_string(), win.clone());
        win.present();
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
            Effect::Open(name) => self.open_surface(&name),
            Effect::Close(name) => {
                if let Some(w) = self.extra.borrow_mut().remove(&name) {
                    w.destroy();
                }
            }
            Effect::Call {
                service,
                args,
                then,
                error,
            } => {
                let tx = self.tx.clone();
                if let Some(sv) = &self.services
                    && sv.call(&service, &args, then.clone(), error.clone(), tx.clone())
                {
                    return;
                }
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
                    "windows.activate" => crate::services::focus_window(
                        args.get("id").and_then(Value::as_str).unwrap_or(""),
                    ),
                    "windows.launch" => {
                        if let Err(e) = crate::services::launch(
                            args.get("app").and_then(Value::as_str).unwrap_or(""),
                        ) {
                            fail(&tx, &error, e);
                        }
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
