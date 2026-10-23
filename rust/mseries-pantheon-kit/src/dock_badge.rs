//! Best-effort dock badges for GTK applications on Pantheon.
//!
//! The dock consumes the Unity LauncherEntry `Update` signal on the session
//! bus. Connecting is asynchronous, and updates made before the bus is ready
//! are coalesced to the latest count/progress state.

use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

struct Inner {
    application_uri: String,
    connection: RefCell<Option<gio::DBusConnection>>,
    connecting: Cell<bool>,
    count: Cell<u64>,
    progress: Cell<Option<f64>>,
    last_sent: Cell<Option<(u64, Option<f64>)>>,
}

/// A nonblocking badge/progress channel to the Pantheon dock.
///
/// Pass the GTK application ID or its desktop entry ID. For example,
/// `io.elementary.mseries.MessagesRust` maps to
/// `application://io.elementary.mseries.MessagesRust.desktop`. A full
/// `application://…desktop` URI is accepted too. The dock integration is
/// best-effort: if the session bus is not available, the latest state is kept
/// and connection is retried on the next update.
#[derive(Clone)]
pub struct DockBadge(Rc<Inner>);

impl DockBadge {
    /// Create a badge channel. The D-Bus connection is opened asynchronously.
    pub fn new(application_id: impl AsRef<str>) -> Self {
        let application_uri = application_uri(application_id.as_ref());
        let inner = Rc::new(Inner {
            application_uri,
            connection: RefCell::new(None),
            connecting: Cell::new(false),
            count: Cell::new(0),
            progress: Cell::new(None),
            last_sent: Cell::new(None),
        });
        connect(&inner);
        Self(inner)
    }

    /// Set the integer count shown on the dock icon. Zero hides the badge.
    pub fn set_count(&self, count: u64) {
        self.0.count.set(count.min(i64::MAX as u64));
        publish_or_connect(&self.0);
    }

    /// Set an optional 0.0–1.0 progress bar while preserving the badge count.
    pub fn set_progress(&self, progress: Option<f64>) {
        self.0.progress.set(
            progress
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 1.0)),
        );
        publish_or_connect(&self.0);
    }

    /// Atomically update the count and optional progress bar.
    pub fn update(&self, count: u64, progress: Option<f64>) {
        self.0.count.set(count.min(i64::MAX as u64));
        self.0.progress.set(
            progress
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 1.0)),
        );
        publish_or_connect(&self.0);
    }

    /// Clear the badge and progress bar.
    pub fn clear(&self) {
        self.update(0, None);
    }
}

fn application_uri(application_id: &str) -> String {
    if application_id.starts_with("application://") {
        application_id.to_string()
    } else if application_id.ends_with(".desktop") {
        format!("application://{application_id}")
    } else {
        format!("application://{application_id}.desktop")
    }
}

fn connect(inner: &Rc<Inner>) {
    if inner.connection.borrow().is_some() || inner.connecting.replace(true) {
        return;
    }

    let weak = Rc::downgrade(inner);
    gio::bus_get(
        gio::BusType::Session,
        None::<&gio::Cancellable>,
        move |result| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            inner.connecting.set(false);
            if let Ok(connection) = result {
                inner.connection.replace(Some(connection));
                inner.last_sent.set(None);
                publish_or_connect(&inner);
            }
        },
    );
}

fn publish_or_connect(inner: &Rc<Inner>) {
    let Some(connection) = inner.connection.borrow().as_ref().cloned() else {
        connect(inner);
        return;
    };

    let state = (inner.count.get(), inner.progress.get());
    if inner.last_sent.get() == Some(state) {
        return;
    }

    let properties = glib::VariantDict::new(None);
    properties.insert_value("count", &((state.0 as i64).to_variant()));
    properties.insert_value("count-visible", &(state.0 > 0).to_variant());
    properties.insert_value("progress", &state.1.unwrap_or(0.0).to_variant());
    properties.insert_value("progress-visible", &state.1.is_some().to_variant());

    let args =
        glib::Variant::tuple_from_iter([inner.application_uri.to_variant(), properties.end()]);
    let path = format!(
        "/com/canonical/unity/launcherentry/{}",
        launcher_hash(&inner.application_uri)
    );
    if connection
        .emit_signal(None, &path, INTERFACE, "Update", Some(&args))
        .is_ok()
    {
        inner.last_sent.set(Some(state));
    } else {
        inner.connection.borrow_mut().take();
        inner.last_sent.set(None);
        connect(inner);
    }
}

fn launcher_hash(uri: &str) -> u32 {
    let mut hash = 5381_u32;
    for byte in uri.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "requires an explicitly isolated session bus"]
    fn dock_badge_private_bus_updates() {
        assert_eq!(
            std::env::var("MSERIES_MESSAGES_ISOLATED_GUI").as_deref(),
            Ok("1")
        );
        let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
        let updates = Rc::new(RefCell::new(
            Vec::<(String, HashMap<String, glib::Variant>)>::new(),
        ));
        let received = updates.clone();
        let _subscription = connection.subscribe_to_signal(
            None,
            Some(INTERFACE),
            Some("Update"),
            None,
            Some("application://io.elementary.mseries.BadgeFixture.desktop"),
            gio::DBusSignalFlags::NONE,
            move |signal| received.borrow_mut().push(signal.parameters.get().unwrap()),
        );
        let wait_for = |count: usize| {
            let deadline = Instant::now() + Duration::from_secs(2);
            while updates.borrow().len() < count && Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(updates.borrow().len(), count);
        };
        let badge = DockBadge::new("io.elementary.mseries.BadgeFixture");
        badge.set_count(1);
        badge.set_count(7);
        wait_for(1);
        {
            let updates = updates.borrow();
            let (uri, properties) = &updates[0];
            assert_eq!(
                uri,
                "application://io.elementary.mseries.BadgeFixture.desktop"
            );
            assert_eq!(properties["count"].get::<i64>(), Some(7));
            assert_eq!(properties["count-visible"].get::<bool>(), Some(true));
            assert_eq!(properties["progress-visible"].get::<bool>(), Some(false));
        }
        badge.update(u64::MAX, Some(1.5));
        wait_for(2);
        {
            let updates = updates.borrow();
            let properties = &updates[1].1;
            assert_eq!(properties["count"].get::<i64>(), Some(i64::MAX));
            assert_eq!(properties["progress"].get::<f64>(), Some(1.0));
            assert_eq!(properties["progress-visible"].get::<bool>(), Some(true));
        }
        badge.set_progress(Some(f64::NAN));
        wait_for(3);
        assert_eq!(
            updates.borrow()[2].1["progress-visible"].get::<bool>(),
            Some(false)
        );
        badge.clear();
        wait_for(4);
        assert_eq!(
            updates.borrow()[3].1["count-visible"].get::<bool>(),
            Some(false)
        );
        assert_eq!(updates.borrow()[3].1["count"].get::<i64>(), Some(0));
        badge.clear();
        let deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            updates.borrow().len(),
            4,
            "unchanged state must not republish"
        );
    }
}
