//! The command palette (Ctrl+K): one fuzzy list over threads, offline transcripts, projects, pinned
//! repos, cached issues and actions (new thread per provider, switch model, pages, pairing). Keyboard
//! only: type to filter, Up/Down to move, Enter to run, Escape to close. Modelled on Orchard's
//! `CommandPalette.vala` and T3's palette.

use crate::chat::label;
use gtk::{gdk, glib, pango, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;

/// One thing the palette can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    /// Shown dimmed after the title: the group ("Thread", "Project", "Issue #12", "Action").
    pub kind: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenId(String),
    NewThread { provider: Option<String> },
    NewThreadIn(String),
    Repo(String),
    SetModel { provider: String, model: String },
    Page(String),
    StartIssue { repo: String, number: u64 },
    WindowAction(String),
}

/// Orchard's scorer: a substring wins (earlier is better), else every query character must appear in
/// order (runs score higher); -1 means no match. An empty query matches everything at 0.
pub fn score(query: &str, text: &str) -> i32 {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 0;
    }
    let hay = text.to_lowercase();
    if let Some(i) = hay.find(&q) {
        return 1000 - i as i32;
    }
    let mut result = 0;
    let mut cursor = 0;
    let hay_chars: Vec<char> = hay.chars().collect();
    for c in q.chars() {
        let found = hay_chars[cursor.min(hay_chars.len())..].iter().position(|h| *h == c).map(|p| p + cursor);
        match found {
            Some(at) => {
                result += if at == cursor { 5 } else { 1 };
                cursor = at + 1;
            }
            None => return -1,
        }
    }
    result
}

/// Ranked matches, best first, ties by title.
pub fn rank<'a>(query: &str, catalog: &'a [Entry]) -> Vec<&'a Entry> {
    let mut scored: Vec<(i32, &Entry)> = catalog
        .iter()
        .filter_map(|e| {
            let s = score(query, &e.title).max(score(query, &e.kind) - 1);
            (s >= 0).then_some((s, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
    scored.into_iter().map(|(_, e)| e).take(60).collect()
}

pub struct Palette {
    window: gtk::Window,
    query: gtk::SearchEntry,
    list: gtk::ListBox,
    catalog: Vec<Entry>,
    shown: RefCell<Vec<Entry>>,
    on_run: Rc<dyn Fn(Action)>,
}

impl Palette {
    pub fn open(parent: &gtk::Window, catalog: Vec<Entry>, on_run: Rc<dyn Fn(Action)>) -> Rc<Palette> {
        let window = gtk::Window::builder()
            .transient_for(parent)
            .modal(true)
            .title("Command Palette")
            .default_width(640)
            .default_height(440)
            .decorated(false)
            .build();
        window.add_css_class("palette");
        let query = gtk::SearchEntry::builder().placeholder_text("Threads, projects, repos, issues, actions…").build();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).build();
        list.add_css_class("palette-list");
        let scroll =
            gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.set_margin_top(12);
        column.set_margin_bottom(12);
        column.set_margin_start(12);
        column.set_margin_end(12);
        column.append(&query);
        column.append(&scroll);
        window.set_child(Some(&column));
        let palette = Rc::new(Palette { window, query, list, catalog, shown: RefCell::new(Vec::new()), on_run });

        let p = palette.clone();
        palette.query.connect_search_changed(move |_| p.refresh());
        let p = palette.clone();
        palette.query.connect_activate(move |_| p.run_selected());
        let p = palette.clone();
        palette.list.connect_row_activated(move |_, row| {
            let i = row.index() as usize;
            p.run(i);
        });
        let keys = gtk::EventControllerKey::new();
        let p = palette.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                p.window.close();
                glib::Propagation::Stop
            }
            gdk::Key::Down | gdk::Key::Up => {
                p.step(if key == gdk::Key::Down { 1 } else { -1 });
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        palette.window.add_controller(keys);
        palette.refresh();
        palette.window.present();
        palette.query.grab_focus();
        palette
    }

    pub fn window(&self) -> &gtk::Window {
        &self.window
    }

    /// Sets the query and filters at once (the entry's own search-changed signal is debounced).
    pub fn set_query(&self, text: &str) {
        self.query.set_text(text);
        self.refresh();
    }

    pub fn shown_titles(&self) -> Vec<String> {
        self.shown.borrow().iter().map(|e| e.title.clone()).collect()
    }

    fn refresh(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let matches: Vec<Entry> = rank(&self.query.text(), &self.catalog).into_iter().cloned().collect();
        for e in &matches {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row_box.add_css_class("palette-row");
            let title = label(&e.title, &[]);
            title.set_hexpand(true);
            title.set_ellipsize(pango::EllipsizeMode::Middle);
            row_box.append(&title);
            row_box.append(&label(&e.kind, &["caption", "dim-label"]));
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&row_box));
            self.list.append(&row);
        }
        *self.shown.borrow_mut() = matches;
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        }
    }

    fn step(&self, delta: i32) {
        let n = self.shown.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let current = self.list.selected_row().map(|r| r.index()).unwrap_or(-1);
        let next = (current + delta).clamp(0, n - 1);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
            row.grab_focus();
            self.query.grab_focus();
        }
    }

    fn run_selected(&self) {
        let i = self.list.selected_row().map(|r| r.index()).unwrap_or(0);
        self.run(i.max(0) as usize);
    }

    pub fn run(&self, index: usize) {
        let entry = self.shown.borrow().get(index).cloned();
        if let Some(e) = entry {
            self.window.close();
            (self.on_run)(e.action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(title: &str, kind: &str) -> Entry {
        Entry { title: title.into(), kind: kind.into(), action: Action::Page("x".into()) }
    }

    #[test]
    fn scoring_follows_orchard() {
        assert_eq!(score("", "anything"), 0);
        assert!(score("usage", "Open Usage") > score("usg", "Open Usage"));
        assert_eq!(score("zz", "Open Usage"), -1);
        assert!(score("ou", "Open Usage") > 0);
    }

    #[test]
    fn ranking_prefers_substrings_then_fuzzy() {
        let catalog = vec![e("New thread on Claude", "Action"), e("Codex: fix build", "Thread"), e("Usage", "Page")];
        let r = rank("claude", &catalog);
        assert_eq!(r[0].title, "New thread on Claude");
        let r = rank("", &catalog);
        assert_eq!(r.len(), 3, "an empty query lists everything");
        let r = rank("thread", &catalog);
        assert_eq!(r.len(), 2, "kind matches too");
    }
}
