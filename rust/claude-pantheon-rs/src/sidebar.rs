//! Sidebar rows in Granite's `navigation-sidebar` style: a project header that collapses its section,
//! thread rows with the provider's accent dot, a running spinner, an unread badge and the relative
//! time, and a context menu (pin, archive, rename, hand off, copy link). Pinned projects can be
//! dragged into the order the owner wants; collapsed sections and that order persist in
//! `~/.config/claude-pantheon-rs/sidebar.json`.

use crate::chat::label;
use crate::look::{iso_to_unix, relative_time};
use crate::t3::model::{Provider, ThreadShell};
use gtk::{gdk, gio, glib, pango, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct SidebarConfig {
    #[serde(default)]
    pub collapsed: HashSet<String>,
    /// Project ids in the owner's order; projects not listed follow, newest first.
    #[serde(default)]
    pub project_order: Vec<String>,
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    base.join("claude-pantheon-rs").join("sidebar.json")
}

impl SidebarConfig {
    pub fn load() -> SidebarConfig {
        std::fs::read(config_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = config_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(p, bytes);
        }
    }

    /// Sorts project ids by the saved order; unknown ones keep their incoming order after the known ones.
    pub fn order_projects(&self, ids: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = self.project_order.iter().filter(|id| ids.contains(id)).cloned().collect();
        for id in ids {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    pub fn move_project(&mut self, current: Vec<String>, moved: &str, before: Option<&str>) {
        let mut order: Vec<String> = current.into_iter().filter(|id| id != moved).collect();
        let at = before.and_then(|b| order.iter().position(|id| id == b)).unwrap_or(order.len());
        order.insert(at, moved.to_string());
        self.project_order = order;
    }
}

/// The provider's accent, from the server when it sends one, else a fixed palette per driver.
pub fn accent_for(provider: Option<&Provider>, driver: &str) -> String {
    if let Some(c) = provider.and_then(|p| p.accent_color.clone()) {
        return c;
    }
    match driver {
        "claudeAgent" | "claude" => "#c4784a",
        "codex" => "#10a37f",
        "grok" => "#5f6368",
        "opencode" => "#e35d6a",
        "cursor" => "#8b5cf6",
        "antigravity" => "#3b82f6",
        "pi" => "#f59e0b",
        _ => "#7d8790",
    }
    .to_string()
}

/// A thread has activity the owner has not looked at: updated after the last visit.
pub fn is_unread(t: &ThreadShell) -> bool {
    let visited = t.rest.get("lastVisitedAt").and_then(|v| v.as_str()).map(iso_to_unix).unwrap_or(0);
    let latest = t
        .rest
        .get("latestRunCompletedAt")
        .and_then(|v| v.as_str())
        .map(iso_to_unix)
        .filter(|&u| u > 0)
        .unwrap_or_else(|| iso_to_unix(&t.updated_at));
    latest > visited && t.rest.get("createdBy").and_then(|v| v.as_str()) != Some("system")
}

pub fn is_pinned(t: &ThreadShell) -> bool {
    t.rest.get("pinnedAt").is_some_and(|v| !v.is_null())
}

fn dot(color: &str) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_width(8).content_height(8).valign(gtk::Align::Center).build();
    let rgba = gdk::RGBA::parse(color).unwrap_or(gdk::RGBA::new(0.5, 0.5, 0.5, 1.0));
    area.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgba(rgba.red() as f64, rgba.green() as f64, rgba.blue() as f64, 1.0);
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, 3.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    area
}

/// One thread row: accent dot, title, running spinner or unread badge, time, pin mark.
pub fn thread_row(t: &ThreadShell, title: &str, accent: &str, now: i64) -> gtk::ListBoxRow {
    let row_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row_box.add_css_class("sb-row");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.append(&dot(accent));
    let t_label = label(title, &["sb-title"]);
    t_label.set_hexpand(true);
    head.append(&t_label);
    if t.is_running() {
        let spinner = gtk::Spinner::builder().spinning(true).valign(gtk::Align::Center).build();
        spinner.add_css_class("sb-spinner");
        head.append(&spinner);
    } else if t.pending_requests() > 0 {
        let b = label("?", &["sb-unread", "sb-ask"]);
        b.set_tooltip_text(Some("Needs your answer"));
        head.append(&b);
    } else if is_unread(t) {
        let b = dot("#3689e6");
        b.set_tooltip_text(Some("New activity"));
        head.append(&b);
    }
    if is_pinned(t) {
        let pin = gtk::Image::from_icon_name("view-pin-symbolic");
        pin.add_css_class("dim-label");
        head.append(&pin);
    }
    row_box.append(&head);
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let when = label(&relative_time(iso_to_unix(&t.updated_at), now), &["caption", "dim-label"]);
    when.set_margin_start(14);
    foot.append(&when);
    row_box.append(&foot);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&row_box));
    row.set_tooltip_text(Some(&format!("{title}\n{}", t.id)));
    row.update_property(&[gtk::accessible::Property::Label(&format!(
        "{title}, {}{}{}",
        relative_time(iso_to_unix(&t.updated_at), now),
        if t.is_running() { ", running" } else { "" },
        if is_pinned(t) { ", pinned" } else { "" }
    ))]);
    row
}

/// An offline transcript row: title, "imported" mark when the server has it, time.
pub fn session_row(title: &str, mtime: i64, imported: bool, id: &str, now: i64) -> gtk::ListBoxRow {
    let row_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row_box.add_css_class("sb-row");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let t = label(title, &["sb-title"]);
    t.set_hexpand(true);
    head.append(&t);
    if imported {
        let mark = gtk::Image::from_icon_name("emblem-synchronizing-symbolic");
        mark.add_css_class("dim-label");
        mark.set_tooltip_text(Some("Imported on the server"));
        head.append(&mark);
    }
    row_box.append(&head);
    row_box.append(&label(&relative_time(mtime, now), &["caption", "dim-label"]));
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&row_box));
    row.set_tooltip_text(Some(&format!("{title}\n{id}")));
    row
}

/// A small Granite-style dialog asking for a new title.
pub fn rename_dialog(parent: &gtk::ApplicationWindow, current: &str, on_ok: impl Fn(String) + 'static) {
    let dialog =
        gtk::Window::builder().transient_for(parent).modal(true).title("Rename Thread").default_width(420).build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_margin_top(16);
    column.set_margin_bottom(16);
    column.set_margin_start(16);
    column.set_margin_end(16);
    column.append(&label("Rename Thread", &["title-3"]));
    let entry = gtk::Entry::builder().text(current).activates_default(true).build();
    column.append(&entry);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("Rename");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    column.append(&buttons);
    dialog.set_child(Some(&column));
    dialog.set_default_widget(Some(&ok));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let d = dialog.clone();
    ok.connect_clicked(move |_| {
        let title = entry.text().trim().to_string();
        if !title.is_empty() {
            on_ok(title);
        }
        d.close();
    });
    dialog.present();
}

/// A project header that toggles its section; `count` is how many threads it hides when collapsed.
pub fn project_header(
    title: &str,
    tooltip: Option<&str>,
    collapsed: bool,
    count: usize,
    on_toggle: impl Fn() + 'static,
) -> gtk::ListBoxRow {
    let header = gtk::ListBoxRow::builder().selectable(false).activatable(false).build();
    header.add_css_class("sb-header");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let arrow = gtk::Image::from_icon_name(if collapsed { "pan-end-symbolic" } else { "pan-down-symbolic" });
    arrow.add_css_class("dim-label");
    line.append(&arrow);
    let h = granite::HeaderLabel::new(title);
    h.set_hexpand(true);
    h.set_tooltip_text(tooltip);
    line.append(&h);
    if collapsed && count > 0 {
        line.append(&label(&count.to_string(), &["caption", "dim-label"]));
    }
    header.set_child(Some(&line));
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| on_toggle());
    header.add_controller(click);
    header
}

/// The thread context menu: actions take the thread id as their target.
pub fn thread_menu(thread_id: &str, pinned: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    let item = |label_text: &str, action: &str| {
        let i = gio::MenuItem::new(Some(label_text), None);
        i.set_action_and_target_value(Some(action), Some(&thread_id.to_variant()));
        i
    };
    menu.append_item(&item(if pinned { "Unpin" } else { "Pin" }, "win.thread-pin"));
    menu.append_item(&item("Rename…", "win.thread-rename"));
    menu.append_item(&item("Hand Off to Repo", "win.thread-handoff"));
    menu.append_item(&item("Copy Link", "win.thread-copy-link"));
    menu.append_item(&item("Archive", "win.thread-archive"));
    menu
}

/// Attaches a right-click (and long-press) popover menu to a row.
pub fn attach_menu(row: &gtk::ListBoxRow, menu: gio::Menu) {
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(row);
    popover.set_has_arrow(false);
    let right = gtk::GestureClick::builder().button(3).build();
    let p = popover.clone();
    right.connect_pressed(move |_, _, x, y| {
        p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        p.popup();
    });
    row.add_controller(right);
    let long = gtk::GestureLongPress::new();
    let p = popover.clone();
    long.connect_pressed(move |_, x, y| {
        p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        p.popup();
    });
    row.add_controller(long);
    // Keep the popover from outliving its row (GTK requires an explicit unparent).
    row.connect_destroy(move |_| popover.unparent());
}

/// Drag source and drop target for reordering pinned projects by their header rows.
pub fn make_header_draggable(header: &gtk::ListBoxRow, project_id: &str, on_drop: impl Fn(String, String) + 'static) {
    let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
    let id = project_id.to_string();
    source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_variant().to_value())));
    header.add_controller(source);
    let target = gtk::DropTarget::new(glib::Variant::static_type(), gdk::DragAction::MOVE);
    let id = project_id.to_string();
    target.connect_drop(move |_, value, _, _| {
        if let Ok(v) = value.get::<glib::Variant>()
            && let Some(moved) = v.get::<String>()
            && moved != id
        {
            on_drop(moved, id.clone());
            return true;
        }
        false
    });
    header.add_controller(target);
}

#[allow(dead_code)]
fn _keep(_: pango::EllipsizeMode) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_order_round_trip() {
        let mut c = SidebarConfig::default();
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(c.order_projects(ids.clone()), ids);
        c.move_project(ids.clone(), "c", Some("a"));
        assert_eq!(c.order_projects(ids.clone()), vec!["c", "a", "b"]);
        c.move_project(c.order_projects(ids.clone()), "a", None);
        assert_eq!(c.order_projects(ids), vec!["c", "b", "a"]);
    }

    #[test]
    fn unread_and_accent() {
        let t = ThreadShell {
            updated_at: "2026-10-22T01:00:00Z".into(),
            rest: serde_json::json!({"lastVisitedAt": "2026-10-22T00:00:00Z", "createdBy": "user"}),
            ..Default::default()
        };
        assert!(is_unread(&t));
        assert_eq!(accent_for(None, "codex"), "#10a37f");
    }
}
