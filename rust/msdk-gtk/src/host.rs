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

/// A typed request. An owned async task keeps the reply, which never retains GTK.
pub struct ServiceCall {
    pub service: String,
    pub args: std::collections::BTreeMap<String, Value>,
    pub reply: ServiceReply,
}

enum ReplyOutcome {
    Event(Event),
    Failure(msdk::runtime::ServiceFailure, Option<String>),
}
struct ReplyDelivery {
    outcome: ReplyOutcome,
    context: Option<msdk::context::ContextToken>,
}
#[derive(Default)]
struct ReplyState {
    completed: bool,
    context: Option<msdk::context::ContextToken>,
}
#[derive(Clone)]
pub struct ServiceReply {
    state: std::sync::Arc<std::sync::Mutex<ReplyState>>,
    service: String,
    source: String,
    operation: u64,
    expected: Option<msdk::vocab::Payload>,
    then: Option<String>,
    error: Option<String>,
    events: UnboundedSender<Event>,
    deliveries: UnboundedSender<ReplyDelivery>,
}
impl ServiceReply {
    /// Bind once, before starting work. All clones share this binding and the
    /// first-completion state. Host checks freshness at final callback delivery.
    pub fn in_context(
        self,
        token: msdk::context::ContextToken,
    ) -> Result<Self, msdk::runtime::ServiceFailure> {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.completed || state.context.is_some() {
                return Err(msdk::runtime::ServiceFailure::new(
                    msdk::runtime::FailureKind::InvalidInput,
                    &self.service,
                    &self.source,
                    self.operation,
                ));
            }
            state.context = Some(token);
        }
        Ok(self)
    }
    fn claim(&self) -> Option<Option<msdk::context::ContextToken>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.completed {
            return None;
        }
        state.completed = true;
        Some(state.context.clone())
    }
    fn is_completed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .completed
    }
    fn is_bound(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .context
            .is_some()
    }
    /// Validate the declared result before delivering it into app state.
    pub fn success(&self, payload: Option<Value>) {
        use msdk::vocab::Payload;
        let valid = match self.expected {
            None => true,
            Some(Payload::None) => payload.is_none(),
            Some(Payload::Text) => payload.as_ref().is_some_and(Value::is_string),
            Some(Payload::Bool) => payload.as_ref().is_some_and(Value::is_boolean),
        };
        if !valid {
            return self.fail(msdk::runtime::ServiceFailure::new(
                msdk::runtime::FailureKind::MalformedData,
                &self.service,
                &self.source,
                self.operation,
            ));
        }
        let Some(context) = self.claim() else {
            return;
        };
        if let Some(message) = &self.then {
            let _ = self.deliveries.unbounded_send(ReplyDelivery {
                outcome: ReplyOutcome::Event((message.clone(), payload)),
                context,
            });
        }
    }
    /// Private detail may reach the app UI; it is omitted from diagnostics.
    pub fn fail(&self, mut failure: msdk::runtime::ServiceFailure) {
        let Some(context) = self.claim() else {
            return;
        };
        failure.service = self.service.clone();
        failure.source = self.source.clone();
        failure.operation = self.operation;
        let _ = self.deliveries.unbounded_send(ReplyDelivery {
            outcome: ReplyOutcome::Failure(failure, self.error.clone()),
            context,
        });
    }
}

/// A feed delivery endpoint used on the GTK/state-owner thread. The context
/// binding is opt-in and does not stop the provider's subscription or reads.
#[derive(Clone)]
pub struct FeedReply {
    push: Rc<dyn Fn(Value)>,
    context: Option<msdk::context::ContextToken>,
}
impl FeedReply {
    /// Bind a fresh endpoint before starting work for an account or view.
    /// Rebinding an endpoint would revive stale work, so it is rejected.
    pub fn in_context(mut self, token: msdk::context::ContextToken) -> Result<Self, &'static str> {
        if self.context.is_some() {
            return Err("feed reply already has a context");
        }
        self.context = Some(token);
        Ok(self)
    }
    /// Deliver on the same thread that owns the context and host state.
    pub fn push(&self, value: Value) {
        if self.context.as_ref().is_none_or(|token| token.is_current()) {
            (self.push)(value);
        }
    }
}

/// Builds the app's Rust logic once the host exists; it gets a sender for events from its own threads.
/// System access an app can plug in: named services (`call`) and live feeds. Crates such as the T3 client implement it;
/// `FixtureServices` answers from a JSON file so an app can run before its real backend exists.
pub trait Services {
    /// Primary typed dispatch. Return true once owned (including async work).
    /// Returning false permits fallback, so start no work in that case.
    fn invoke(&self, call: ServiceCall) -> Result<bool, msdk::runtime::ServiceFailure> {
        self.call_checked(
            &call.service,
            &call.args,
            call.reply.then,
            call.reply.error,
            call.reply.events,
        )
    }
    /// Version of the SDK extension interface (not the backend's OS/network API).
    /// Legacy providers implement this existing v1 interface implicitly.
    fn api_version(&self) -> u32 {
        msdk::runtime::SERVICES_API_VERSION
    }
    /// Handles `service`; returns false when this provider does not own it. Replies go to `tx` as events.
    fn call(
        &self,
        _service: &str,
        _args: &std::collections::BTreeMap<String, Value>,
        _then: Option<String>,
        _error: Option<String>,
        _tx: UnboundedSender<Event>,
    ) -> bool {
        false
    }
    /// Typed dispatch. An error is owned by this provider: the Host reports it
    /// without falling through to a real system action. Legacy call() remains
    /// the default implementation for existing providers.
    fn call_checked(
        &self,
        service: &str,
        args: &std::collections::BTreeMap<String, Value>,
        then: Option<String>,
        error: Option<String>,
        tx: UnboundedSender<Event>,
    ) -> Result<bool, msdk::runtime::ServiceFailure> {
        Ok(self.call(service, args, then, error, tx))
    }
    /// Starts `feed`, calling `push` with the whole list now and whenever it changes. False when not owned.
    fn start_feed(&self, _feed: &str, _push: Rc<dyn Fn(Value)>) -> bool {
        false
    }
    /// Typed feed entry point. Override to bind a context before starting work;
    /// legacy providers continue through `start_feed` without automatic scoping.
    fn start_feed_typed(&self, feed: &str, reply: FeedReply) -> bool {
        self.start_feed(feed, Rc::new(move |value| reply.push(value)))
    }
    /// Called once when this host closes. Implementations must cancel owned work
    /// and retire callbacks; adapters shared across hosts must scope ownership.
    /// The default is for providers with no ongoing work (e.g. fixtures).
    fn stop(&self) {}
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
    closed: Cell<bool>,
    reload_source: RefCell<Option<glib::SourceId>>,
    window_watches: RefCell<Vec<crate::services::WindowWatch>>,
    processes: RefCell<Vec<msdk::process::Process>>,
    failures: UnboundedSender<(msdk::runtime::ServiceFailure, Option<String>)>,
    operation: Cell<u64>,
    replies: UnboundedSender<ReplyDelivery>,
}

// Widgets keep a dispatcher, never the Host itself. Otherwise Host -> widgets
// -> signal closures -> Host forms a permanent reference cycle after closing.
struct WeakDispatcher(Weak<Host>);
impl Dispatcher for WeakDispatcher {
    fn send(&self, msg: &str, payload: Option<Value>) {
        if let Some(host) = self.0.upgrade() {
            Dispatcher::send(host.as_ref(), msg, payload);
        }
    }
    fn get(&self, key: &str) -> Option<Value> {
        self.0
            .upgrade()
            .and_then(|host| Dispatcher::get(host.as_ref(), key))
    }
    fn set(&self, key: &str, value: Value) {
        if let Some(host) = self.0.upgrade() {
            Dispatcher::set(host.as_ref(), key, value);
        }
    }
    fn confirm(&self, text: &str, then: Box<dyn FnOnce()>) {
        if let Some(host) = self.0.upgrade() {
            Dispatcher::confirm(host.as_ref(), text, then);
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.dispose();
    }
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
        let (failures, mut failure_events) = unbounded();
        let (replies, mut reply_events) = unbounded::<ReplyDelivery>();
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
            closed: Cell::new(false),
            reload_source: RefCell::new(None),
            window_watches: RefCell::new(Vec::new()),
            processes: RefCell::new(Vec::new()),
            failures,
            operation: Cell::new(0),
            replies,
        });
        *host.me.borrow_mut() = Rc::downgrade(&host);
        host.rebuild();
        let weak = Rc::downgrade(&host);
        glib::spawn_future_local(async move {
            while let Some((msg, payload)) = rx.next().await {
                let Some(host) = weak.upgrade() else { break };
                host.send(&msg, payload);
            }
        });
        let weak = Rc::downgrade(&host);
        glib::spawn_future_local(async move {
            while let Some((failure, callback)) = failure_events.next().await {
                let Some(host) = weak.upgrade() else { break };
                host.runtime_failure(failure, callback);
            }
        });
        let weak = Rc::downgrade(&host);
        glib::spawn_future_local(async move {
            while let Some(delivery) = reply_events.next().await {
                let Some(host) = weak.upgrade() else { break };
                if delivery
                    .context
                    .as_ref()
                    .is_some_and(|token| !token.is_current())
                {
                    continue;
                }
                match delivery.outcome {
                    ReplyOutcome::Event((message, payload)) => host.send(&message, payload),
                    ReplyOutcome::Failure(failure, callback) => {
                        host.runtime_failure_in_context(failure, callback, delivery.context)
                    }
                }
            }
        });
        let weak = Rc::downgrade(&host);
        host.window.connect_unrealize(move |_| {
            if let Some(host) = weak.upgrade() {
                host.dispose();
            }
        });
        host.start_feeds();
        if watch && host.path.is_some() {
            let weak = Rc::downgrade(&host);
            let source =
                glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
                    let Some(h) = weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let now = h.path.as_deref().and_then(mtime);
                    if now != h.mtime.get() {
                        h.mtime.set(now);
                        h.reload();
                    }
                    glib::ControlFlow::Continue
                });
            *host.reload_source.borrow_mut() = Some(source);
        }
        if host.doc.borrow()["surfaces"][surface]["kind"] == "panel" {
            let weak = Rc::downgrade(&host);
            host.window.connect_realize(move |window| {
                if let Some(h) = weak.upgrade() {
                    crate::panel::attach(window, &h.doc.borrow()["surfaces"][&h.surface]);
                }
            });
        }
        host.window.present();
        Some(host)
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    /// Retire host-owned polling/reload work and provider callbacks exactly once.
    /// Late events are ignored. Closing the primary window calls this automatically.
    pub fn dispose(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(source) = self.reload_source.borrow_mut().take() {
            source.remove();
        }
        self.window_watches.borrow_mut().clear();
        self.processes.borrow_mut().clear();
        if let Some(services) = &self.services {
            services.stop();
        }
        let extra = std::mem::take(&mut *self.extra.borrow_mut());
        for (_, window) in extra {
            window.destroy();
        }
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
                if let Err(failure) = msdk::runtime::negotiate_service(
                    sv.api_version(),
                    feed,
                    &format!("/feeds/{key}"),
                    self.next_operation(),
                ) {
                    self.runtime_failure(failure, None);
                    continue;
                }
                let (weak, key) = (self.me.borrow().clone(), key.clone());
                let started = sv.start_feed_typed(
                    feed,
                    FeedReply {
                        context: None,
                        push: Rc::new(move |list| {
                            if let Some(h) = weak.upgrade() {
                                h.set(&key, list);
                                h.post("_feed", Some(json!(key)));
                            }
                        }),
                    },
                );
                if !started {
                    eprintln!("msdk-run: nothing provides the feed `{feed}`");
                }
                continue;
            }
            if name.as_str() == Some("windows.list") {
                let weak = self.me.borrow().clone();
                let watch = crate::services::watch_windows(move |list| {
                    if let Some(h) = weak.upgrade() {
                        h.set(&key, list);
                        h.post("_feed", Some(json!(key)));
                    }
                });
                self.window_watches.borrow_mut().push(watch);
            }
        }
    }

    fn dispatcher(&self) -> D {
        Rc::new(WeakDispatcher(self.me.borrow().clone()))
    }

    fn next_operation(&self) -> u64 {
        let operation = self.operation.get().wrapping_add(1);
        self.operation.set(operation);
        operation
    }

    fn runtime_failure(&self, failure: msdk::runtime::ServiceFailure, callback: Option<String>) {
        self.runtime_failure_in_context(failure, callback, None);
    }
    fn runtime_failure_in_context(
        &self,
        failure: msdk::runtime::ServiceFailure,
        callback: Option<String>,
        context: Option<msdk::context::ContextToken>,
    ) {
        if context.as_ref().is_some_and(|token| !token.is_current()) {
            return;
        }
        if self.closed.get() {
            return;
        }
        eprintln!("{}", failure.diagnostic());
        let mut message = failure.to_string();
        // Details belong to the app's private UI, never to diagnostic output.
        if let Some(detail) = failure.detail().filter(|detail| !detail.is_empty()) {
            message.push('\n');
            message.push_str(detail);
        }
        if let Some(callback) = callback {
            let _ = self.replies.unbounded_send(ReplyDelivery {
                outcome: ReplyOutcome::Event((callback, Some(json!(message)))),
                context,
            });
        } else {
            self.show_error(&message);
        }
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
        if self.closed.get() {
            return;
        }
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

    fn apply(&self, e: Effect, source: &str) {
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
                let operation = self.next_operation();
                if let Some(provider) = &self.services
                    && let Err(failure) = msdk::runtime::negotiate_service(
                        provider.api_version(),
                        &service,
                        source,
                        operation,
                    )
                {
                    self.runtime_failure(failure, error);
                    return;
                }
                let tx = self.tx.clone();
                if let Some(sv) = &self.services {
                    let reply = ServiceReply {
                        state: std::sync::Arc::new(std::sync::Mutex::new(ReplyState::default())),
                        service: service.clone(),
                        source: source.into(),
                        operation,
                        expected: msdk::vocab::service(&service).map(|service| service.then),
                        then: then.clone(),
                        error: error.clone(),
                        events: tx.clone(),
                        deliveries: self.replies.clone(),
                    };
                    let reply_errors = reply.clone();
                    match sv.invoke(ServiceCall {
                        service: service.clone(),
                        args: args.clone(),
                        reply,
                    }) {
                        Ok(true) => return,
                        Ok(false) if reply_errors.is_completed() => {
                            return;
                        }
                        Ok(false) if reply_errors.is_bound() => {
                            reply_errors.fail(msdk::runtime::ServiceFailure::new(
                                msdk::runtime::FailureKind::Unavailable,
                                &service,
                                source,
                                operation,
                            ));
                            return;
                        }
                        Ok(false) => {}
                        Err(failure) => {
                            reply_errors.fail(failure);
                            return;
                        }
                    }
                }
                let fail =
                    move |tx: &UnboundedSender<Event>, error: &Option<String>, why: String| {
                        if let Some(m) = error {
                            let _ = tx.unbounded_send((m.clone(), Some(json!(why))));
                        }
                    };
                match service.as_str() {
                    "process.run" => {
                        self.processes
                            .borrow_mut()
                            .retain(|process| !process.is_finished());
                        if self.processes.borrow().len() >= msdk::runtime::MAX_HOST_PROCESSES {
                            self.runtime_failure(
                                msdk::runtime::ServiceFailure::new(
                                    msdk::runtime::FailureKind::Busy,
                                    &service,
                                    source,
                                    operation,
                                ),
                                error,
                            );
                            return;
                        }
                        let cmd = args
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let failures = self.failures.clone();
                        let completion_source = source.to_string();
                        let error2 = error.clone();
                        let process = msdk::process::spawn(
                            cmd,
                            msdk::process::Limits::default(),
                            operation,
                            move |result| match result {
                                Ok(output) => {
                                    if let Some(m) = then {
                                        let _ = tx.unbounded_send((
                                            m,
                                            Some(json!(output.stdout.trim_end())),
                                        ));
                                    }
                                }
                                Err(mut failure) => {
                                    failure.source = completion_source;
                                    let _ = failures.unbounded_send((failure, error2));
                                }
                            },
                        );
                        match process {
                            Ok(process) => self.processes.borrow_mut().push(process),
                            Err(failure) => self.runtime_failure(failure, error),
                        }
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
                    other => self.runtime_failure(
                        msdk::runtime::ServiceFailure::new(
                            msdk::runtime::FailureKind::Unavailable,
                            other,
                            source,
                            operation,
                        ),
                        error,
                    ),
                }
            }
        }
    }
}

impl Dispatcher for Host {
    fn send(&self, msg: &str, payload: Option<Value>) {
        if self.closed.get() {
            return;
        }
        let effects = {
            let mut st = self.store.borrow_mut();
            self.logic.borrow_mut().update(&mut st, msg, payload)
        };
        let source = if self.custom_logic {
            "/logic".into()
        } else {
            format!("/update/{}", msg.replace('~', "~0").replace('/', "~1"))
        };
        for e in effects {
            self.apply(e, &source);
        }
        self.persist();
        self.refresh();
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.store.borrow().get(key).cloned()
    }

    fn set(&self, key: &str, v: Value) {
        if self.closed.get() {
            return;
        }
        self.store.borrow_mut().insert(key.to_string(), v);
    }

    fn confirm(&self, text: &str, then: Box<dyn FnOnce()>) {
        if self.closed.get() {
            return;
        }
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
