//! Desktop notifications while the window is in the background. Dock badge and progress updates
//! are provided by the shared Pantheon SDK.

use gtk::{gio, glib, prelude::*};

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
