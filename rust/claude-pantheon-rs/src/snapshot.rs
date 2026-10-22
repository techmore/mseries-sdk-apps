//! SnapShot for Pantheon (INTEGRATIONS.md 4a): captures the screen through the Screenshot portal
//! (`org.freedesktop.portal.Screenshot` v2, served by the Pantheon backend; the mseries compositor
//! has no screenshot D-Bus), takes the focused window's title and PID from AT-SPI's active window
//! (title match only: portal captures carry no window identity), uploads the PNG as a chat
//! attachment and drops it on the composer draft. A global shortcut comes from the GlobalShortcuts
//! portal when the backend offers it; otherwise the app's own Ctrl+Shift+S works while it is focused.

use gtk::{gio, glib, prelude::*};
use std::path::PathBuf;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Clone, Default)]
pub struct Capture {
    pub path: PathBuf,
    pub window_title: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug)]
pub enum SnapError {
    Bus(String),
    Denied,
    NoUri,
    Io(String),
}

impl std::fmt::Display for SnapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapError::Bus(e) => write!(f, "portal: {e}"),
            SnapError::Denied => f.write_str("the screenshot was cancelled or denied"),
            SnapError::NoUri => f.write_str("the portal returned no image"),
            SnapError::Io(e) => f.write_str(e),
        }
    }
}

fn session_bus() -> Result<gio::DBusConnection, SnapError> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| SnapError::Bus(e.to_string()))
}

/// The portal's Request object path for this connection and token (xdg-desktop-portal convention).
fn request_path(bus: &gio::DBusConnection, token: &str) -> String {
    let sender = bus.unique_name().map(|n| n.to_string()).unwrap_or_default();
    let sender = sender.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

/// One screenshot through the portal; resolves with the PNG path the portal wrote.
#[allow(deprecated)]
pub async fn capture_screen(parent_window: &str) -> Result<PathBuf, SnapError> {
    let bus = session_bus()?;
    let token = format!("claudepantheon{}", glib::random_int());
    let path = request_path(&bus, &token);
    let (tx, rx) = futures_channel::oneshot::channel::<(u32, glib::Variant)>();
    let tx = std::cell::RefCell::new(Some(tx));
    // Subscribe to the Response before calling, as the portal docs require.
    let sub = bus.signal_subscribe(
        Some(PORTAL),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&path),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            if let (Some(code), Some(results)) = (params.child_value(0).get::<u32>(), Some(params.child_value(1)))
                && let Some(tx) = tx.borrow_mut().take()
            {
                let _ = tx.send((code, results));
            }
        },
    );
    let options = glib::VariantDict::new(None);
    options.insert("handle_token", token.as_str());
    options.insert("interactive", false);
    options.insert("modal", false);
    let args = (parent_window, options.end()).to_variant();
    let call = bus
        .call_future(
            Some(PORTAL),
            PORTAL_PATH,
            "org.freedesktop.portal.Screenshot",
            "Screenshot",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            30_000,
        )
        .await;
    if let Err(e) = call {
        bus.signal_unsubscribe(sub);
        return Err(SnapError::Bus(e.to_string()));
    }
    let (code, results) = rx.await.map_err(|_| SnapError::Denied)?;
    bus.signal_unsubscribe(sub);
    if code != 0 {
        return Err(SnapError::Denied);
    }
    let dict = glib::VariantDict::new(Some(&results));
    let uri: String = dict.lookup::<String>("uri").ok().flatten().ok_or(SnapError::NoUri)?;
    let file = gio::File::for_uri(&uri);
    file.path().ok_or(SnapError::NoUri)
}

/// The active window's title and PID from AT-SPI: the desktop's applications, each one's windows,
/// the one with the ACTIVE state. Best effort; None when the a11y bus is not there.
pub async fn active_window() -> Option<(String, u32)> {
    let session = session_bus().ok()?;
    let addr = session
        .call_future(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            "org.a11y.Bus",
            "GetAddress",
            None,
            None,
            gio::DBusCallFlags::NONE,
            3_000,
        )
        .await
        .ok()?;
    let addr: String = addr.child_value(0).get()?;
    let a11y = gio::DBusConnection::for_address_future(&addr, gio::DBusConnectionFlags::AUTHENTICATION_CLIENT, None)
        .await
        .ok()?;
    let children = |bus_name: &str, path: &str| {
        let a11y = a11y.clone();
        let bus_name = bus_name.to_string();
        let path = path.to_string();
        async move {
            let v = a11y
                .call_future(
                    Some(&bus_name),
                    &path,
                    "org.a11y.atspi.Accessible",
                    "GetChildren",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    3_000,
                )
                .await
                .ok()?;
            let arr = v.child_value(0);
            let mut out: Vec<(String, String)> = Vec::new();
            for i in 0..arr.n_children() {
                let child = arr.child_value(i);
                let name: String = child.child_value(0).get()?;
                let obj: String = child.child_value(1).get()?;
                out.push((name, obj));
            }
            Some(out)
        }
    };
    let apps = children("org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root").await?;
    for (app_bus, app_path) in apps {
        let Some(windows) = children(&app_bus, &app_path).await else { continue };
        for (win_bus, win_path) in windows {
            let state = a11y
                .call_future(
                    Some(&win_bus),
                    &win_path,
                    "org.a11y.atspi.Accessible",
                    "GetState",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    2_000,
                )
                .await
                .ok();
            let Some(state) = state else { continue };
            let bits = state.child_value(0);
            let low: u32 = bits.child_value(0).get().unwrap_or(0);
            const ACTIVE: u32 = 1 << 1;
            if low & ACTIVE == 0 {
                continue;
            }
            let name = a11y
                .call_future(
                    Some(&win_bus),
                    &win_path,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&("org.a11y.atspi.Accessible", "Name").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2_000,
                )
                .await
                .ok()
                .and_then(|v| v.child_value(0).as_variant().and_then(|inner| inner.get::<String>()))
                .unwrap_or_default();
            let pid = a11y
                .call_future(
                    Some(&app_bus),
                    &app_path,
                    "org.a11y.atspi.Application",
                    "GetApplicationBusAddress",
                    None,
                    None,
                    gio::DBusCallFlags::NONE,
                    1_000,
                )
                .await
                .ok()
                .map(|_| 0u32);
            let pid = pid_of_bus_name(&a11y, &app_bus).await.or(pid);
            return Some((name, pid.unwrap_or(0)));
        }
    }
    None
}

async fn pid_of_bus_name(bus: &gio::DBusConnection, name: &str) -> Option<u32> {
    bus.call_future(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetConnectionUnixProcessID",
        Some(&(name,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1_000,
    )
    .await
    .ok()
    .and_then(|v| v.child_value(0).get::<u32>())
}

/// Capture plus identity, in the order that keeps the title honest: the active window is read first
/// (the portal's own dialog or flash would otherwise be the focused window).
pub async fn snap(parent_window: &str) -> Result<Capture, SnapError> {
    let identity = active_window().await;
    let path = capture_screen(parent_window).await?;
    Ok(Capture { path, window_title: identity.as_ref().map(|(t, _)| t.clone()), pid: identity.map(|(_, p)| p) })
}

/// A file name for the attachment: the window title when known, else the time.
pub fn attachment_name(c: &Capture) -> String {
    let stamp = glib::DateTime::now_local()
        .and_then(|d| d.format("%Y-%m-%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    match &c.window_title {
        Some(t) if !t.trim().is_empty() => {
            let safe: String = t
                .chars()
                .map(|ch| if ch.is_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' })
                .take(48)
                .collect();
            format!("snap-{}-{stamp}.png", safe.trim_matches('-'))
        }
        _ => format!("snap-{stamp}.png"),
    }
}

/// Tries to bind a global shortcut through the GlobalShortcuts portal. Returns false when the
/// backend does not implement it (Pantheon's does not today); the in-app accelerator still works.
pub async fn global_shortcut_available() -> bool {
    let Ok(bus) = session_bus() else { return false };
    let v = bus
        .call_future(
            Some(PORTAL),
            PORTAL_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.freedesktop.portal.GlobalShortcuts", "version").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2_000,
        )
        .await;
    v.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe() {
        let c = Capture {
            path: PathBuf::new(),
            window_title: Some("Code: main.rs — Claude/Pantheon".into()),
            pid: Some(1),
        };
        let n = attachment_name(&c);
        assert!(n.starts_with("snap-Code--main-rs") && n.ends_with(".png"), "{n}");
        assert!(attachment_name(&Capture::default()).starts_with("snap-"));
    }
}
