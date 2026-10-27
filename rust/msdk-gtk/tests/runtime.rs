//! Actual host dispatch and process ownership in an isolated display/session.
use gtk::prelude::*;
use msdk_gtk::host::{Event, Host, ServiceCall, Services};
use msdk_gtk::render::Dispatcher;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

struct Versioned {
    version: Rc<Cell<u32>>,
    calls: Rc<Cell<u32>>,
}
impl Services for Versioned {
    fn api_version(&self) -> u32 {
        self.version.get()
    }
    fn call(
        &self,
        _: &str,
        _: &std::collections::BTreeMap<String, Value>,
        then: Option<String>,
        _: Option<String>,
        tx: futures_channel::mpsc::UnboundedSender<Event>,
    ) -> bool {
        assert_eq!(
            self.version.get(),
            1,
            "incompatible provider must never be dispatched"
        );
        self.calls.set(self.calls.get() + 1);
        tx.unbounded_send((then.unwrap(), Some(json!("recovered"))))
            .unwrap();
        true
    }
    fn start_feed(&self, _: &str, _: Rc<dyn Fn(Value)>) -> bool {
        false
    }
}
struct Typed(Rc<Cell<u8>>);
impl Services for Typed {
    fn invoke(&self, call: ServiceCall) -> Result<bool, msdk::runtime::ServiceFailure> {
        let mode = self.0.get();
        std::thread::spawn(move || {
            if mode == 1 {
                call.reply
                    .success(Some(json!({"private":"synthetic-private-payload"})));
            } else if mode == 2 {
                call.reply.fail(
                    msdk::runtime::ServiceFailure::new(
                        msdk::runtime::FailureKind::PermissionDenied,
                        "",
                        "",
                        0,
                    )
                    .with_detail("synthetic-private-detail".into()),
                );
            } else {
                call.reply.success(Some(json!("typed recovery")));
                // Contradictory duplicate completion cannot overwrite success.
                call.reply.fail(msdk::runtime::ServiceFailure::new(
                    msdk::runtime::FailureKind::Io,
                    "",
                    "",
                    0,
                ));
            }
        });
        Ok(true)
    }
}

struct ContextServices {
    context: msdk::context::RequestContext,
    pending: Rc<RefCell<Vec<msdk_gtk::host::ServiceReply>>>,
    deny: Rc<Cell<bool>>,
    decline: Rc<Cell<bool>>,
}
impl Services for ContextServices {
    fn invoke(&self, call: ServiceCall) -> Result<bool, msdk::runtime::ServiceFailure> {
        if call.service == "sdktest.context.switch" {
            self.context.advance().unwrap();
            call.reply.success(None);
        } else {
            let reply = call.reply.in_context(self.context.token().unwrap())?;
            if self.decline.get() {
                return Ok(false);
            }
            if self.deny.get() {
                return Err(msdk::runtime::ServiceFailure::new(
                    msdk::runtime::FailureKind::PermissionDenied,
                    "",
                    "",
                    0,
                ));
            }
            self.pending.borrow_mut().push(reply);
        }
        Ok(true)
    }
    fn stop(&self) {
        self.context.close();
        self.pending.borrow_mut().clear();
    }
}

fn pump_until(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let context = gtk::glib::MainContext::default();
    while !ready() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(ready(), "native runtime operation did not settle");
}
fn visible_labels(widget: &gtk::Widget) -> String {
    if !widget.is_visible() {
        return String::new();
    }
    let mut text = widget
        .downcast_ref::<gtk::Label>()
        .map(|label| label.text().to_string())
        .unwrap_or_default();
    let mut child = widget.first_child();
    while let Some(node) = child {
        text.push('\n');
        text.push_str(&visible_labels(&node));
        child = node.next_sibling();
    }
    text
}
fn document() -> Value {
    let mut doc: Value =
        serde_json::from_str(include_str!("../../msdk/templates/settings/app.ui.json")).unwrap();
    doc["app"]["id"] = json!("com.example.MsdkRuntime");
    doc["persist"] = json!([]);
    doc["capabilities"] = json!(["subprocess"]);
    doc["state"]["out"] = json!("text");
    doc["state"]["failed"] = json!("bool");
    doc["messages"]["Run"] = json!("none");
    doc["messages"]["Done"] = json!("text");
    doc["messages"]["Failed"] = json!("text");
    doc["update"]["Run"] = json!([{"call":"process.run","args":{"command":"printf synthetic-private-argument"},"then":"Done","error":"Failed"}]);
    doc["update"]["Done"] = json!([{"set":"out","to":"$payload"},{"set":"failed","to":false}]);
    doc["update"]["Failed"] = json!([{"set":"out","to":"$payload"},{"set":"failed","to":true}]);
    doc["surfaces"]["main"]["body"]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"Text","text":"{out}"}));
    doc
}
#[test]
fn extension_mismatch_recovers_and_closed_hosts_kill_owned_commands() {
    assert_eq!(std::env::var("MSDK_PRIVATE_TEST_BUS").as_deref(), Ok("1"));
    gtk::init().unwrap();
    let app = gtk::Application::builder()
        .application_id("com.example.MsdkRuntimeTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(gtk::gio::Cancellable::NONE).unwrap();
    let version = Rc::new(Cell::new(2));
    let calls = Rc::new(Cell::new(0));
    let provider = Rc::new(Versioned {
        version: version.clone(),
        calls: calls.clone(),
    });
    let host =
        Host::open_with(&app, document(), None, "main", None, Some(provider), false).unwrap();
    host.post("Run", None);
    pump_until(|| host.get("failed") == Some(json!(true)));
    assert_eq!(calls.get(), 0);
    assert!(
        host.get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("incompatible")
    );
    version.set(1);
    host.post("Run", None);
    pump_until(|| host.get("out") == Some(json!("recovered")));
    assert_eq!(calls.get(), 1);
    assert_eq!(host.get("failed"), Some(json!(false)));
    host.window().close();
    drop(host);

    msdk::vocab::load_manifest(&json!({"services":[{"name":"sdktest.absent","capability":"subprocess","then":"text","args":{}}]})).unwrap();
    let mut unavailable_doc = document();
    unavailable_doc["update"]["Run"][0] =
        json!({"call":"sdktest.absent","args":{},"then":"Done","error":"Failed"});
    let unavailable = Host::open_doc(&app, unavailable_doc, None, "main", None, false).unwrap();
    unavailable.post("Run", None);
    pump_until(|| unavailable.get("failed") == Some(json!(true)));
    assert!(
        unavailable
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("unavailable")
    );
    unavailable.window().close();
    drop(unavailable);

    let mode = Rc::new(Cell::new(1));
    let typed = Host::open_with(
        &app,
        document(),
        None,
        "main",
        None,
        Some(Rc::new(Typed(mode.clone()))),
        false,
    )
    .unwrap();
    typed.post("Run", None);
    pump_until(|| typed.get("failed") == Some(json!(true)));
    assert!(
        typed
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("invalid data")
    );
    assert!(
        !typed
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("synthetic-private-payload")
    );
    mode.set(2);
    typed.post("Run", None);
    pump_until(|| {
        typed
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Access was denied")
    });
    assert!(
        typed
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("synthetic-private-detail")
    );
    mode.set(0);
    typed.post("Run", None);
    pump_until(|| typed.get("out") == Some(json!("typed recovery")));
    assert_eq!(typed.get("failed"), Some(json!(false)));
    typed.window().close();
    drop(typed);

    msdk::vocab::load_manifest(&json!({"services":[{"name":"sdktest.context.switch","capability":"subprocess","args":{}}]})).unwrap();
    let scope = msdk::context::RequestContext::new();
    let pending = Rc::new(RefCell::new(Vec::new()));
    let deny = Rc::new(Cell::new(false));
    let decline = Rc::new(Cell::new(false));
    let provider = Rc::new(ContextServices {
        context: scope.clone(),
        pending: pending.clone(),
        deny: deny.clone(),
        decline: decline.clone(),
    });
    let mut scoped_doc = document();
    scoped_doc["state"]["applied"] = json!("number");
    for message in ["Done", "Failed"] {
        scoped_doc["update"][message]
            .as_array_mut()
            .unwrap()
            .push(json!({"add":"applied","by":1}));
    }
    scoped_doc["messages"]["RunSwitch"] = json!("none");
    scoped_doc["messages"]["SwitchNow"] = json!("text");
    scoped_doc["update"]["RunSwitch"] = json!([{"call":"process.run","args":{"command":"not executed"},"then":"SwitchNow","error":"Failed"}]);
    scoped_doc["update"]["SwitchNow"] = json!([{"call":"sdktest.context.switch","args":{}}]);
    let scoped =
        Host::open_with(&app, scoped_doc, None, "main", None, Some(provider), false).unwrap();
    scoped.post("Run", None);
    scoped.post("Run", None);
    let old = std::mem::take(&mut *pending.borrow_mut());
    old[0].success(Some(json!("obsolete success")));
    old[1].fail(msdk::runtime::ServiceFailure::new(
        msdk::runtime::FailureKind::PermissionDenied,
        "",
        "",
        0,
    ));
    scope.advance().unwrap();
    scoped.post("Run", None);
    let current = pending.borrow_mut().pop().unwrap();
    assert_eq!(
        current
            .clone()
            .in_context(scope.token().unwrap())
            .err()
            .unwrap()
            .kind,
        msdk::runtime::FailureKind::InvalidInput
    );
    current.success(Some(json!("fresh scoped result")));
    pump_until(|| scoped.get("out") == Some(json!("fresh scoped result")));
    assert_eq!(scoped.get("applied"), Some(json!(1)));
    assert_eq!(scoped.get("failed"), Some(json!(false)));
    let labels = visible_labels(scoped.window().upcast_ref());
    assert!(labels.contains("fresh scoped result") && !labels.contains("obsolete success"));

    // Host retains a reply clone before invoke(). A binding made inside the
    // provider must also protect the Host's synchronous Err path.
    deny.set(true);
    scoped.post("Run", None);
    scope.advance().unwrap();
    deny.set(false);
    scoped.post("Run", None);
    pending
        .borrow_mut()
        .pop()
        .unwrap()
        .success(Some(json!("after synchronous denial")));
    pump_until(|| scoped.get("out") == Some(json!("after synchronous denial")));
    assert_eq!(scoped.get("applied"), Some(json!(2)));
    assert_eq!(scoped.get("failed"), Some(json!(false)));

    // A current error is processed first, enqueueing its callback. The next
    // delivery switches contexts BEFORE that callback runs. The callback must
    // carry its token across the second queue boundary too.
    scoped.post("Run", None);
    scoped.post("RunSwitch", None);
    let transition = std::mem::take(&mut *pending.borrow_mut());
    let previous = scope.token().unwrap();
    transition[0].fail(msdk::runtime::ServiceFailure::new(
        msdk::runtime::FailureKind::Io,
        "",
        "",
        0,
    ));
    transition[1].success(Some(json!("switch")));
    pump_until(|| !previous.is_current());
    scoped.post("Run", None);
    pending
        .borrow_mut()
        .pop()
        .unwrap()
        .success(Some(json!("after callback race")));
    pump_until(|| scoped.get("out") == Some(json!("after callback race")));
    assert_eq!(
        scoped.get("applied"),
        Some(json!(3)),
        "obsolete error callback must not apply after a switch"
    );
    assert_eq!(scoped.get("failed"), Some(json!(false)));
    decline.set(true);
    scoped.post("Run", None);
    pump_until(|| {
        scoped
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("service is unavailable")
    });
    assert_eq!(scoped.get("failed"), Some(json!(true)));
    assert_eq!(
        scoped.get("applied"),
        Some(json!(4)),
        "a bound request cannot execute the real built-in fallback"
    );
    decline.set(false);
    scoped.post("Run", None);
    pending
        .borrow_mut()
        .pop()
        .unwrap()
        .success(Some(json!("after declined binding")));
    pump_until(|| scoped.get("out") == Some(json!("after declined binding")));
    assert_eq!(scoped.get("failed"), Some(json!(false)));
    scoped.window().close();
    assert!(scope.token().is_none());
    let weak = Rc::downgrade(&scoped);
    drop(scoped);
    assert!(weak.upgrade().is_none());

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("msdk-host-{}-{stamp}.pid", std::process::id()));
    let quoted = path.to_string_lossy().replace('\'', "'\\''");
    let mut fixture_doc = document();
    fixture_doc["update"]["Run"][0]["args"]["command"] = json!(format!("touch '{quoted}'"));
    let fixture = Rc::new(msdk_gtk::fixture::FixtureServices::from_json(
        &json!({"calls":{}}),
    ));
    let fixture_host =
        Host::open_with(&app, fixture_doc, None, "main", None, Some(fixture), false).unwrap();
    fixture_host.post("Run", None);
    pump_until(|| fixture_host.get("failed") == Some(json!(true)));
    assert!(
        !path.exists(),
        "missing fixture reply must never execute a real fallback"
    );
    fixture_host.window().close();
    drop(fixture_host);

    let incompatible =
        msdk_gtk::fixture::FixtureServices::try_from_json(&json!({"api_version":2,"calls":{}}))
            .unwrap();
    assert!(
        msdk_gtk::fixture::FixtureServices::try_from_json(&json!({"api_version":"2"})).is_err()
    );
    assert!(
        msdk_gtk::fixture::FixtureServices::try_from_json(&json!({"api_version":4294967296_u64}))
            .is_err()
    );
    let mut mismatch_doc = document();
    mismatch_doc["update"]["Run"][0]["args"]["command"] = json!(format!("touch '{quoted}'"));
    let mismatch = Host::open_with(
        &app,
        mismatch_doc,
        None,
        "main",
        None,
        Some(Rc::new(incompatible)),
        false,
    )
    .unwrap();
    mismatch.post("Run", None);
    pump_until(|| mismatch.get("failed") == Some(json!(true)));
    assert!(
        mismatch
            .get("out")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("incompatible")
    );
    assert!(
        !path.exists(),
        "incompatible fixture must never execute a real fallback"
    );
    mismatch.window().close();
    drop(mismatch);

    let mut doc = document();
    doc["update"]["Run"][0]["args"]["command"] =
        json!(format!("printf '%s' \"$$\" > '{quoted}'; exec sleep 20"));
    let host = Host::open_doc(&app, doc, None, "main", None, false).unwrap();
    host.post("Run", None);
    pump_until(|| path.exists());
    let pid = std::fs::read_to_string(&path).unwrap();
    host.window().close();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    pump_until(|| !alive());
    assert_ne!(host.get("out"), Some(json!("done")));
    std::fs::remove_file(path).unwrap();
}
