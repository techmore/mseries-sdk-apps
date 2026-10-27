//! System services the runtime implements for `call` actions and `feeds`.
//!
//! Windows come from the compositor's Gala-compatible D-Bus API (`org.pantheon.gala.DesktopIntegration`), the same API the
//! Vala dock uses, so no new compositor protocol is needed for a window list, focus or launch.

use std::rc::Rc;
use std::time::Duration;

use futures_util::StreamExt;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};

const NAME: &str = "org.pantheon.gala";
const PATH: &str = "/org/pantheon/gala/DesktopInterface";
const IFACE: &str = "org.pantheon.gala.DesktopIntegration";

async fn proxy() -> Result<gio::DBusProxy, glib::Error> {
    gio::DBusProxy::for_bus_future(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        NAME,
        PATH,
        IFACE,
    )
    .await
}

/// Parses the reply of `GetWindows`: `a(ta{sv})` inside a one-tuple.
fn parse_windows(reply: &glib::Variant) -> Vec<Value> {
    let Some(list) = reply.try_child_value(0) else {
        return vec![];
    };
    (0..list.n_children())
        .filter_map(|i| {
            let entry = list.child_value(i);
            let uid = entry.child_value(0).get::<u64>()?;
            let props = glib::VariantDict::new(Some(&entry.child_value(1)));
            let text = |k: &str| props.lookup::<String>(k).ok().flatten().unwrap_or_default();
            let flag = |k: &str| props.lookup::<bool>(k).ok().flatten().unwrap_or(false);
            Some(json!({
                "id": uid.to_string(), "app_id": text("app-id"), "title": text("title"),
                "focused": flag("has-focus"), "attention": flag("demands-attention"),
            }))
        })
        .collect()
}

async fn poll(p: &gio::DBusProxy, on: &Rc<dyn Fn(Value)>) {
    match p
        .call_future("GetWindows", None, gio::DBusCallFlags::NONE, 800)
        .await
    {
        Ok(v) => on(Value::Array(parse_windows(&v))),
        Err(e) => eprintln!("msdk-run: GetWindows failed: {e}"),
    }
}

/// An owned watcher. Dropping it cancels its pending D-Bus call, signal listener
/// and safety polling task. It must be retained only while the feature is active.
pub struct WindowWatch(glib::JoinHandle<()>);

impl Drop for WindowWatch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct SignalConnection(gio::DBusProxy, Option<glib::SignalHandlerId>);
impl Drop for SignalConnection {
    fn drop(&mut self) {
        if let Some(handler) = self.1.take() {
            self.0.disconnect(handler);
        }
    }
}

/// Calls `on` now, on compositor change signals, and every two seconds while owned.
pub fn watch_windows(on: impl Fn(Value) + 'static) -> WindowWatch {
    let on: Rc<dyn Fn(Value)> = Rc::new(on);
    WindowWatch(glib::spawn_future_local(async move {
        let p = match proxy().await {
            Ok(p) => p,
            Err(e) => {
                return eprintln!("msdk-run: no window service ({e}); is the compositor running?");
            }
        };
        // The signal handler must be thread-safe, so it only pokes a channel that this thread listens to.
        let (poke, mut pokes) = futures_channel::mpsc::unbounded::<()>();
        let handler = p.connect_g_signal(move |_, _, signal, _| {
            if matches!(signal, "WindowsChanged" | "RunningApplicationsChanged") {
                let _ = poke.unbounded_send(());
            }
        });
        let _signals = SignalConnection(p.clone(), Some(handler));
        loop {
            poll(&p, &on).await;
            // One task owns all work; signals cannot create overlapping reads.
            futures_util::future::select(
                Box::pin(pokes.next()),
                Box::pin(glib::timeout_future(Duration::from_secs(2))),
            )
            .await;
        }
    }))
}

pub fn focus_window(id: &str) {
    let Ok(uid) = id.parse::<u64>() else { return };
    glib::spawn_future_local(async move {
        if let Ok(p) = proxy().await
            && let Err(e) = p
                .call_future(
                    "FocusWindow",
                    Some(&(uid,).to_variant()),
                    gio::DBusCallFlags::NONE,
                    800,
                )
                .await
        {
            eprintln!("msdk-run: FocusWindow failed: {e}");
        }
    });
}

/// Starts an installed application by desktop id (with or without `.desktop`).
pub fn launch(app: &str) -> Result<(), String> {
    let id = if app.ends_with(".desktop") {
        app.to_string()
    } else {
        format!("{app}.desktop")
    };
    let info = gio::AppInfo::all()
        .into_iter()
        .find(|a| a.id().is_some_and(|i| i == id.as_str()))
        .ok_or_else(|| format!("no application {id}"))?;
    info.launch(&[], gio::AppLaunchContext::NONE)
        .map_err(|e| e.to_string())
}
