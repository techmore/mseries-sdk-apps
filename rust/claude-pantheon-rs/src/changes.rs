//! The Changes page (issue #474): the thread's full diff from `orchestration.getFullThreadDiff`
//! as a file tree with status icons and counts, an inline or side-by-side view of the selected file
//! in GtkSourceView (the `diff` language for inline; two synchronised views for side-by-side),
//! per-hunk copy, and Restore to a checkpoint through `checkpoint.rollback` after a confirmation.

use crate::chat::label;
use crate::render::apply_scheme;
use crate::t3::Rpc;
use gtk::{gio, glib, pango, prelude::*};
use serde_json::{Value, json};
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    /// "added", "deleted", "renamed", "modified"
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<String>,
    pub body: String,
}

/// Splits a unified diff into files; status from the `new file` / `deleted file` / `rename` markers.
pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.split_whitespace().nth(1).map(|b| b.trim_start_matches("b/")).unwrap_or(rest).to_string();
            files.push(FileDiff { path, status: "modified".into(), ..Default::default() });
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if line.starts_with("new file mode") {
            f.status = "added".into();
        } else if line.starts_with("deleted file mode") {
            f.status = "deleted".into();
        } else if line.starts_with("rename from") {
            f.status = "renamed".into();
        }
        if line.starts_with("@@") {
            f.hunks.push(String::new());
        }
        if let Some(h) = f.hunks.last_mut() {
            h.push_str(line);
            h.push('\n');
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            f.additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            f.deletions += 1;
        }
        f.body.push_str(line);
        f.body.push('\n');
    }
    files
}

/// Old and new text of a hunk for the side-by-side view, context lines on both sides.
pub fn split_sides(hunk: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for line in hunk.lines() {
        if line.starts_with("@@") {
            old.push_str(line);
            old.push('\n');
            new.push_str(line);
            new.push('\n');
        } else if let Some(l) = line.strip_prefix('-') {
            old.push_str(l);
            old.push('\n');
        } else if let Some(l) = line.strip_prefix('+') {
            new.push_str(l);
            new.push('\n');
        } else {
            let l = line.strip_prefix(' ').unwrap_or(line);
            old.push_str(l);
            old.push('\n');
            new.push_str(l);
            new.push('\n');
        }
    }
    (old, new)
}

pub struct ChangesPage {
    pub root: gtk::Box,
    tree: gtk::ListBox,
    view_box: gtk::Box,
    status: gtk::Label,
    side_by_side: gtk::ToggleButton,
    restore_btn: gtk::MenuButton,
    files: RefCell<Vec<FileDiff>>,
    checkpoints: RefCell<Vec<Value>>,
    thread_id: RefCell<Option<String>>,
    rpc: RefCell<Option<Rpc>>,
    on_restore: RefCell<Option<Rc<dyn Fn(String, String)>>>,
}

fn status_icon(status: &str) -> &'static str {
    match status {
        "added" => "list-add-symbolic",
        "deleted" => "list-remove-symbolic",
        "renamed" => "edit-find-replace-symbolic",
        _ => "document-edit-symbolic",
    }
}

impl ChangesPage {
    pub fn new() -> Rc<ChangesPage> {
        let tree = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).css_classes(["navigation-sidebar"]).build();
        let tree_scroll = gtk::ScrolledWindow::builder().child(&tree).hscrollbar_policy(gtk::PolicyType::Never).width_request(260).vexpand(true).build();
        let view_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        view_box.set_margin_top(12);
        view_box.set_margin_bottom(12);
        view_box.set_margin_start(12);
        view_box.set_margin_end(12);
        let view_scroll = gtk::ScrolledWindow::builder().child(&view_box).hexpand(true).vexpand(true).build();
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("changes-bar");
        let status = label("", &["caption", "dim-label"]);
        status.set_hexpand(true);
        bar.append(&status);
        let inline = gtk::ToggleButton::with_label("Inline");
        let side_by_side = gtk::ToggleButton::with_label("Side by side");
        side_by_side.set_group(Some(&inline));
        inline.set_active(true);
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        modes.append(&inline);
        modes.append(&side_by_side);
        bar.append(&modes);
        let restore_btn = gtk::MenuButton::builder().label("Restore to checkpoint…").build();
        restore_btn.set_sensitive(false);
        bar.append(&restore_btn);
        let paned = gtk::Paned::builder().orientation(gtk::Orientation::Horizontal).start_child(&tree_scroll).end_child(&view_scroll).position(260).shrink_start_child(false).build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&bar);
        root.append(&paned);
        let page = Rc::new(ChangesPage {
            root,
            tree,
            view_box,
            status,
            side_by_side,
            restore_btn,
            files: RefCell::new(Vec::new()),
            checkpoints: RefCell::new(Vec::new()),
            thread_id: RefCell::new(None),
            rpc: RefCell::new(None),
            on_restore: RefCell::new(None),
        });
        let p = page.clone();
        page.tree.connect_row_selected(move |_, row| {
            if let Some(r) = row {
                p.show_file(r.index() as usize);
            }
        });
        let p = page.clone();
        page.side_by_side.connect_toggled(move |_| {
            if let Some(r) = p.tree.selected_row() {
                p.show_file(r.index() as usize);
            }
        });
        page
    }

    pub fn file_count(&self) -> usize {
        self.files.borrow().len()
    }

    /// Called with (scopeId, checkpointId) after the owner confirmed a restore.
    pub fn on_restore(&self, f: Rc<dyn Fn(String, String)>) {
        *self.on_restore.borrow_mut() = Some(f);
    }

    /// Loads the thread's diff and its checkpoints (from the projection the app already holds).
    pub fn load(self: &Rc<Self>, rpc: Rpc, thread_id: String, turns: usize, checkpoints: Vec<Value>) {
        *self.rpc.borrow_mut() = Some(rpc.clone());
        *self.thread_id.borrow_mut() = Some(thread_id.clone());
        *self.checkpoints.borrow_mut() = checkpoints;
        self.status.set_text("Reading the thread's diff…");
        self.fill_restore_menu();
        let page = self.clone();
        glib::spawn_future_local(async move {
            let payload = json!({"threadId": thread_id, "toTurnCount": turns.max(1)});
            match rpc.call("orchestration.getFullThreadDiff", payload).await {
                Ok(v) => page.set_diff(v.get("diff").and_then(Value::as_str).unwrap_or("")),
                Err(e) => page.status.set_text(&format!("Diff failed: {}", e.message())),
            }
        });
    }

    pub fn set_diff(&self, diff: &str) {
        let files = parse(diff);
        while let Some(row) = self.tree.row_at_index(0) {
            self.tree.remove(&row);
        }
        for f in &files {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row_box.add_css_class("sb-row");
            row_box.append(&gtk::Image::from_icon_name(status_icon(&f.status)));
            let name = label(&f.path, &["sb-title"]);
            name.set_ellipsize(pango::EllipsizeMode::Start);
            name.set_hexpand(true);
            name.set_tooltip_text(Some(&f.path));
            row_box.append(&name);
            row_box.append(&label(&format!("+{} −{}", f.additions, f.deletions), &["caption", "dim-label"]));
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&row_box));
            self.tree.append(&row);
        }
        let (adds, dels) = files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        self.status.set_text(if files.is_empty() {
            "No file changes recorded for this thread.".to_string()
        } else {
            format!("{} files · +{adds} −{dels}", files.len())
        });
        *self.files.borrow_mut() = files;
        if let Some(first) = self.tree.row_at_index(0) {
            self.tree.select_row(Some(&first));
        } else {
            while let Some(c) = self.view_box.first_child() {
                self.view_box.remove(&c);
            }
        }
    }

    fn show_file(&self, index: usize) {
        while let Some(c) = self.view_box.first_child() {
            self.view_box.remove(&c);
        }
        let Some(f) = self.files.borrow().get(index).cloned() else { return };
        let head = label(&format!("{}  ({}, +{} −{})", f.path, f.status, f.additions, f.deletions), &["title-4"]);
        head.set_ellipsize(pango::EllipsizeMode::Start);
        self.view_box.append(&head);
        for (i, hunk) in f.hunks.iter().enumerate() {
            let hunk_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
            hunk_box.add_css_class("card");
            hunk_box.add_css_class("code-block");
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            bar.add_css_class("code-bar");
            let title = label(&format!("Hunk {}", i + 1), &["caption", "dim-label"]);
            title.set_hexpand(true);
            bar.append(&title);
            let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy this hunk").build();
            copy.add_css_class("flat");
            let text = hunk.clone();
            copy.connect_clicked(move |b| b.clipboard().set_text(&text));
            bar.append(&copy);
            hunk_box.append(&bar);
            if self.side_by_side.is_active() {
                let (old, new) = split_sides(hunk);
                let pair = gtk::Box::new(gtk::Orientation::Horizontal, 1);
                pair.set_homogeneous(true);
                pair.append(&diff_view(&old, Some("diff-old")));
                pair.append(&diff_view(&new, Some("diff-new")));
                hunk_box.append(&pair);
            } else {
                hunk_box.append(&diff_view(hunk, None));
            }
            self.view_box.append(&hunk_box);
        }
    }

    fn fill_restore_menu(self: &Rc<Self>) {
        let checkpoints = self.checkpoints.borrow().clone();
        let menu = gio::Menu::new();
        let mut shown = 0;
        for c in checkpoints.iter().rev().take(12) {
            let (Some(id), Some(scope)) = (c.get("id").and_then(Value::as_str), c.get("scopeId").and_then(Value::as_str)) else { continue };
            if c.get("status").and_then(Value::as_str) != Some("ready") {
                continue;
            }
            let files = c.get("files").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
            let when = c.get("capturedAt").and_then(Value::as_str).map(crate::look::clock).unwrap_or_default();
            let item = gio::MenuItem::new(Some(&format!("{when} · {files} files")), None);
            item.set_action_and_target_value(Some("changes.restore"), Some(&format!("{scope}\n{id}").to_variant()));
            menu.append_item(&item);
            shown += 1;
        }
        self.restore_btn.set_menu_model(Some(&menu));
        self.restore_btn.set_sensitive(shown > 0);
        // The action lives on the page's own group so the menu can target it.
        let group = gio::SimpleActionGroup::new();
        let action = gio::SimpleAction::new("restore", Some(glib::VariantTy::STRING));
        let page = self.clone();
        action.connect_activate(move |_, param| {
            let Some(v) = param.and_then(|p| p.get::<String>()) else { return };
            let Some((scope, id)) = v.split_once('\n') else { return };
            page.confirm_restore(scope.to_string(), id.to_string());
        });
        group.add_action(&action);
        self.root.insert_action_group("changes", Some(&group));
    }

    fn confirm_restore(self: &Rc<Self>, scope: String, id: String) {
        let dialog = gtk::AlertDialog::builder()
            .message("Restore files to this checkpoint?")
            .detail("Files the thread changed after it are rewritten on disk. The conversation keeps its history.")
            .buttons(["Cancel", "Restore"])
            .default_button(1)
            .cancel_button(0)
            .build();
        let page = self.clone();
        let root = self.root.root().and_then(|r| r.downcast::<gtk::Window>().ok());
        dialog.choose(root.as_ref(), gio::Cancellable::NONE, move |r| {
            if r == Ok(1)
                && let Some(f) = page.on_restore.borrow().as_ref()
            {
                f(scope.clone(), id.clone());
            }
        });
    }
}

/// A read-only GtkSourceView showing diff text with the `diff` language.
fn diff_view(text: &str, class: Option<&str>) -> gtk::Widget {
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_language(sourceview5::LanguageManager::default().language("diff").as_ref());
    buffer.set_highlight_syntax(true);
    buffer.set_text(text.trim_end_matches('\n'));
    apply_scheme(&buffer);
    let view = sourceview5::View::with_buffer(&buffer);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.add_css_class("code-view");
    if let Some(c) = class {
        view.add_css_class(c);
    }
    let scroll = gtk::ScrolledWindow::builder().child(&view).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-fn b() {}\n+fn c() {}\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hello\n";

    #[test]
    fn parses_files_and_counts() {
        let files = parse(DIFF);
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].path.as_str(), files[0].additions, files[0].deletions), ("src/a.rs", 1, 1));
        assert_eq!(files[1].status, "added");
        assert_eq!(files[0].hunks.len(), 1);
        let (old, new) = split_sides(&files[0].hunks[0]);
        assert!(old.contains("fn b() {}") && !old.contains("fn c()"));
        assert!(new.contains("fn c() {}") && !new.contains("fn b()"));
    }
}
