//! Native ownership checks. Run under a private D-Bus session and Xvfb.
use gtk::prelude::*;
use msdk_gtk::host::{FeedReply, Host, Services};
use msdk_gtk::render::Dispatcher;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Provider {
    stops: Rc<Cell<u32>>,
    feeds: Rc<Cell<u32>>,
}

struct ScopedProvider {
    context: msdk::context::RequestContext,
    sink: Rc<RefCell<Option<FeedReply>>>,
    stops: Rc<Cell<u32>>,
}
impl Services for ScopedProvider {
    fn start_feed_typed(&self, _: &str, reply: FeedReply) -> bool {
        *self.sink.borrow_mut() = Some(reply.in_context(self.context.token().unwrap()).unwrap());
        true
    }
    fn stop(&self) {
        self.context.close();
        self.stops.set(self.stops.get() + 1);
    }
}
impl Services for Provider {
    fn api_version(&self) -> u32 {
        // An incompatible optional extension must not disable the host's own
        // windows.list implementation.
        msdk::runtime::SERVICES_API_VERSION + 1
    }
    fn call(
        &self,
        _: &str,
        _: &std::collections::BTreeMap<String, Value>,
        _: Option<String>,
        _: Option<String>,
        _: futures_channel::mpsc::UnboundedSender<msdk_gtk::host::Event>,
    ) -> bool {
        false
    }
    fn start_feed(&self, _: &str, _: Rc<dyn Fn(Value)>) -> bool {
        self.feeds.set(self.feeds.get() + 1);
        false
    }
    fn stop(&self) {
        self.stops.set(self.stops.get() + 1);
    }
}

#[test]
fn closing_stops_owned_work_ignores_late_events_and_releases_the_host() {
    assert_eq!(
        std::env::var("MSDK_PRIVATE_TEST_BUS").as_deref(),
        Ok("1"),
        "run with MSDK_PRIVATE_TEST_BUS=1 under a new dbus-run-session"
    );
    gtk::init().expect("run native test under Xvfb");
    let bus =
        gtk::gio::bus_get_sync(gtk::gio::BusType::Session, gtk::gio::Cancellable::NONE).unwrap();
    let reply = bus
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&("org.pantheon.gala", 4_u32).to_variant()),
            None,
            gtk::gio::DBusCallFlags::NONE,
            1000,
            gtk::gio::Cancellable::NONE,
        )
        .unwrap();
    assert_eq!(
        reply.child_get::<u32>(0),
        1,
        "never replace an existing compositor service"
    );
    let node = gtk::gio::DBusNodeInfo::for_xml(
        r#"<node><interface name="org.pantheon.gala.DesktopIntegration">
      <method name="GetWindows"><arg type="a(ta{sv})" direction="out"/></method>
    </interface></node>"#,
    )
    .unwrap();
    let reads = Rc::new(Cell::new(0));
    let calls = reads.clone();
    let registration = bus
        .register_object("/org/pantheon/gala/DesktopInterface", &node.interfaces()[0])
        .method_call(move |_, _, _, _, method, _, invocation| {
            assert_eq!(method, "GetWindows");
            calls.set(calls.get() + 1);
            let properties: std::collections::HashMap<String, gtk::glib::Variant> = [
                ("app-id".into(), "com.example.Fixture".to_variant()),
                ("title".into(), "Fixture window".to_variant()),
                ("has-focus".into(), true.to_variant()),
                ("demands-attention".into(), false.to_variant()),
            ]
            .into_iter()
            .collect();
            invocation.return_value(Some(&(vec![(7_u64, properties)],).to_variant()));
        })
        .build()
        .unwrap();
    let pump = |duration: std::time::Duration| {
        let end = std::time::Instant::now() + duration;
        let context = gtk::glib::MainContext::default();
        while std::time::Instant::now() < end {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    let app = gtk::Application::builder()
        .application_id("com.example.MsdkLifecycleTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gtk::gio::Cancellable::NONE).unwrap();
    let mut doc: Value =
        serde_json::from_str(include_str!("../../msdk/templates/settings/app.ui.json")).unwrap();
    doc["app"]["id"] = json!("com.example.MsdkLifecycleTest");
    // This scenario has no persistent data and never invokes real services.
    doc["persist"] = json!([]);
    let disabled_doc = doc.clone();
    msdk::vocab::load_manifest(&json!({"feeds":[{"name":"sdktest.lifecycle.feed","capability":"network","fields":{"id":"text"}}]})).unwrap();
    doc["capabilities"] = json!(["windows", "network"]);
    doc["types"] = json!({"Win":{"id":"text","app_id":"text","title":"text","focused":"bool","attention":"bool"}});
    doc["types"]["Other"] = json!({"id":"text"});
    doc["state"]["windows"] = json!("list<Win>");
    doc["state"]["other"] = json!("list<Other>");
    doc["feeds"] = json!({"windows":"windows.list", "other":"sdktest.lifecycle.feed"});
    let stops = Rc::new(Cell::new(0));
    let feeds = Rc::new(Cell::new(0));
    let provider = Rc::new(Provider {
        stops: stops.clone(),
        feeds: feeds.clone(),
    });
    let host = Host::open_with(&app, doc, None, "main", None, Some(provider), false).unwrap();
    let weak = Rc::downgrade(&host);
    assert_eq!(host.get("enabled"), Some(json!(true)));
    pump(std::time::Duration::from_millis(300));
    assert!(
        reads.get() > 0,
        "the enabled feed must execute a real D-Bus read"
    );
    assert_eq!(host.get("windows").unwrap()[0]["id"], "7");
    assert_eq!(
        feeds.get(),
        0,
        "an incompatible provider must not receive its own feed"
    );
    let veto = host
        .window()
        .connect_close_request(|_| gtk::glib::Propagation::Stop);
    host.window().close();
    host.post("SetEnabled", Some(json!(false)));
    assert_eq!(
        host.get("enabled"),
        Some(json!(false)),
        "a vetoed close must leave the host usable"
    );
    assert_eq!(stops.get(), 0);
    host.post("SetEnabled", Some(json!(true)));
    host.window().disconnect(veto);
    let read_count = reads.get();
    host.window().close();
    pump(std::time::Duration::from_millis(2200));
    assert_eq!(
        reads.get(),
        read_count,
        "closing must stop real safety polling"
    );
    assert_eq!(stops.get(), 1);
    host.post("SetEnabled", Some(json!(false)));
    host.set("enabled", json!(false));
    assert_eq!(
        host.get("enabled"),
        Some(json!(true)),
        "late work must not mutate a retired host"
    );
    host.dispose();
    assert_eq!(stops.get(), 1, "provider teardown is idempotent");
    drop(host);
    let context = gtk::glib::MainContext::default();
    for _ in 0..20 {
        if !context.pending() {
            break;
        }
        context.iteration(false);
    }
    assert!(
        weak.upgrade().is_none(),
        "widget dispatchers and event receiver must not retain the host"
    );
    let disabled = Host::open_doc(&app, disabled_doc, None, "main", None, false).unwrap();
    pump(std::time::Duration::from_millis(2200));
    assert_eq!(
        reads.get(),
        read_count,
        "an undeclared optional feed must do no reads"
    );
    disabled.window().close();
    drop(disabled);

    let mut scoped_doc: Value =
        serde_json::from_str(include_str!("../../msdk/templates/settings/app.ui.json")).unwrap();
    scoped_doc["app"]["id"] = json!("com.example.MsdkScopedFeedTest");
    scoped_doc["persist"] = json!([]);
    scoped_doc["capabilities"] = json!(["network"]);
    scoped_doc["types"] = json!({"Other":{"id":"text"}});
    scoped_doc["state"]["other"] = json!("list<Other>");
    scoped_doc["feeds"] = json!({"other":"sdktest.lifecycle.feed"});
    let context = msdk::context::RequestContext::new();
    let sink = Rc::new(RefCell::new(None));
    let scoped_stops = Rc::new(Cell::new(0));
    let provider = Rc::new(ScopedProvider {
        context: context.clone(),
        sink: sink.clone(),
        stops: scoped_stops.clone(),
    });
    let scoped_host =
        Host::open_with(&app, scoped_doc, None, "main", None, Some(provider), false).unwrap();
    let old = sink.borrow().as_ref().unwrap().clone();
    old.push(json!([{"id":"current"}]));
    assert_eq!(scoped_host.get("other"), Some(json!([{"id":"current"}])));
    context.advance().unwrap();
    old.push(json!([{"id":"obsolete"}]));
    assert_eq!(scoped_host.get("other"), Some(json!([{"id":"current"}])));
    assert!(old.clone().in_context(context.token().unwrap()).is_err());
    scoped_host.window().close();
    assert_eq!(scoped_stops.get(), 1);
    old.push(json!([{"id":"after-close"}]));
    assert_eq!(scoped_host.get("other"), Some(json!([{"id":"current"}])));
    drop(scoped_host);
    bus.unregister_object(registration).unwrap();
}
