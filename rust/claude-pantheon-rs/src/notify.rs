//! Desktop integration while the window is in the background: a `GLib.Notification` when a turn
//! finishes or a permission prompt appears (with Answer/Open actions that run `app.open-thread`),
//! and the dock badge and progress bar through the Unity LauncherEntry signal that Pantheon's dock
//! listens to (`com.canonical.Unity.LauncherEntry.Update` on the session bus).

use gtk::{gio, glib, prelude::*};

const APP_URI: &str = "application://io.github.techmore.ClaudePantheonRs.desktop";

/// Sends (or replaces) the notification for a thread; `id` keeps one notification per thread.
pub fn notify(app: &gtk::Application, id: &str, title: &str, body: &str, thread_id: &str, answer: bool) {
    let n = gio::Notification::new(title);
    n.set_body(Some(body));
    n.set_icon(&gio::ThemedIcon::new("io.github.techmore.ClaudePantheonRs"));
    let target = glib::Variant::from(thread_id);
    n.set_default_action_and_target_value("app.open-thread", Some(&target));
    n.add_button_with_target_value(if answer { "Answer" } else { "Open" }, "app.open-thread", Some(&target));
    if answer {
        n.set_priority(gio::NotificationPriority::High);
    }
    app.send_notification(Some(id), &n);
}

pub fn withdraw(app: &gtk::Application, id: &str) {
    app.withdraw_notification(id);
}

/// The dock badge (running threads) and progress bar (a turn in flight). Zero hides the badge; no
/// running turn hides the bar.
pub fn launcher_entry(count: i64, progress: Option<f64>) {
    let Some(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok() else { return };
    let mut props: Vec<(&str, glib::Variant)> = vec![
        ("count", count.to_variant()),
        ("count-visible", (count > 0).to_variant()),
        ("progress", progress.unwrap_or(0.0).to_variant()),
        ("progress-visible", progress.is_some().to_variant()),
    ];
    let dict = glib::VariantDict::new(None);
    for (k, v) in props.drain(..) {
        dict.insert_value(k, &v);
    }
    // (s a{sv}): a plain tuple of the two values, not a tuple wrapping the dict as a variant.
    let args = glib::Variant::tuple_from_iter([APP_URI.to_variant(), dict.end()]);
    let path = format!("/com/canonical/unity/launcherentry/{}", launcher_hash(APP_URI));
    let result = bus.emit_signal(None, &path, "com.canonical.Unity.LauncherEntry", "Update", Some(&args));
    if std::env::var_os("CLAUDE_PANTHEON_DEBUG").is_some() {
        eprintln!("launcher entry {path}: count {count}, progress {progress:?}: {result:?}");
    }
}

/// djb2 of the app URI, as libunity computes the object path.
fn launcher_hash(uri: &str) -> u32 {
    let mut h: u32 = 5381;
    for b in uri.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        assert_eq!(launcher_hash("application://x.desktop"), launcher_hash("application://x.desktop"));
        assert_ne!(launcher_hash("a"), launcher_hash("b"));
    }
}
