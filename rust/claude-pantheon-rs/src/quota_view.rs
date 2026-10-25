//! Supplemental local subscription reports within the existing Usage page.
use crate::{chat::label as base_label, quota};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = base_label(text, classes);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_ellipsize(gtk::pango::EllipsizeMode::None);
    label
}

pub struct QuotaView {
    root: gtk::Box,
    content: gtk::Box,
    status: gtk::Label,
    refresh: gtk::Button,
    generation: Cell<u64>,
}

impl QuotaView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_margin_start(24);
        root.set_margin_end(24);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Local subscription reports", &["title-4"]);
        title.set_hexpand(true);
        header.append(&title);
        let refresh = gtk::Button::with_label("Read reports");
        refresh.set_tooltip_text(Some("Read existing local quota files; does not contact providers"));
        header.append(&refresh);
        root.append(&header);
        let status = label("", &["caption", "dim-label"]);
        status.set_wrap(true);
        root.append(&status);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.append(&content);
        let view = Rc::new(Self { root, content, status, refresh, generation: Cell::new(0) });
        let weak = Rc::downgrade(&view);
        view.refresh.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.load();
            }
        });
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn load(self: &Rc<Self>) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.refresh.set_sensitive(false);
        self.status.set_text("Reading reports on this computer…");
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                quota::read(&quota::directory(), crate::look::now_unix(), |s| {
                    glib::DateTime::from_iso8601(s, None).ok().map(|d| d.to_unix())
                })
            })
            .await;
            let Some(view) = weak.upgrade() else { return };
            if view.generation.get() != generation {
                return;
            }
            view.refresh.set_sensitive(true);
            match result {
                Ok(snapshot) => view.render(&snapshot),
                Err(_) => view.status.set_text("Could not read local subscription reports. Try Read reports again."),
            }
        });
    }

    pub fn render(&self, snapshot: &quota::Snapshot) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.status.set_text("This computer only · separate from the paired T3 server · display only");
        if snapshot.reports.is_empty() {
            let empty = label(
                "No local subscription reports. Configure a supported collector on this computer to see subscription limits and prepaid balances here.",
                &["dim-label"],
            );
            empty.set_wrap(true);
            self.content.append(&empty);
        }
        for report in &snapshot.reports {
            let r = &report.record;
            let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
            card.add_css_class("card");
            card.add_css_class("pantheon-surface");
            card.add_css_class("usage-card");
            let title = label(&format!("{} · {}", r.name, r.id), &["title-4"]);
            title.set_wrap(true);
            card.append(&title);
            let state = if !r.ready {
                "Unavailable"
            } else if report.stale {
                "Stale · last-known values"
            } else {
                "Recent report"
            };
            let info = label(
                &format!("{state} · {}\nRecord updated {}", r.tier_label, r.updated_at),
                &["caption", "dim-label"],
            );
            info.set_wrap(true);
            card.append(&info);
            for text in [&r.usage_status_text, &r.auth_help_text] {
                if !text.is_empty() {
                    let note = label(text, &["caption"]);
                    note.set_wrap(true);
                    card.append(&note);
                }
            }
            // Failed records may carry old windows; do not draw them as valid quota.
            if r.ready {
                for w in &r.limits {
                    let reset = w
                        .resets_at
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!(" · resets {s}"))
                        .unwrap_or_default();
                    let text = label(&format!("{} · {:.0}% used{reset}", w.title, w.percent * 100.0), &[]);
                    text.set_wrap(true);
                    card.append(&text);
                }
                if let Some(balance) = &r.balance {
                    let text = label(
                        &format!(
                            "{} {:.2} remaining{}",
                            balance.currency,
                            balance.remaining,
                            if balance.estimated { " (estimated)" } else { "" }
                        ),
                        &[],
                    );
                    text.set_wrap(true);
                    card.append(&text);
                }
                if r.limits.is_empty() && r.balance.is_none() {
                    card.append(&label("No quota or balance published", &["dim-label"]));
                }
            }
            self.content.append(&card);
        }
        for problem in &snapshot.problems {
            let note = label(problem, &["caption"]);
            note.set_wrap(true);
            self.content.append(&note);
        }
    }
}
