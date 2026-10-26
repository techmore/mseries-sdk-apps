//! Pinned tstack request preparation. This UI never sends a turn or invokes a workflow.
use gtk::prelude::*;

pub const WORKFLOWS: &[(&str, &str)] = &[
    ("full-send", "Prove, review, file, watch and merge a change"),
    ("full-stop", "Prepare a reviewed PR and stop before merging"),
    ("council", "Review with three providers; requires cross-model delegation"),
    ("sol-loop", "Iterative Sol review; requires a reachable Sol delegate"),
    ("file-pr", "File a concise PR explaining the problem and result"),
    ("takeover", "Continue an existing PR or agent thread, preserving credit"),
    ("babysit-pr", "Watch CI and reviews; fix actionable findings"),
    ("modernize", "Rebase and reassess an older PR"),
    ("leaving-pr-comments", "Format review comments on your behalf"),
];

pub fn prepare(name: &str, draft: &str) -> String {
    format!("Use the {name} skill.\n\n{draft}")
}

pub fn show(parent: &gtk::ApplicationWindow, on_choose: impl Fn(&str) + 'static) -> gtk::Window {
    let window = gtk::Window::builder()
        .title("Tstack Workflows")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(600)
        .build();
    window.set_titlebar(Some(&gtk::HeaderBar::new()));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_margin_start(18);
    column.set_margin_end(18);
    column.set_margin_top(12);
    column.set_margin_bottom(18);
    let note = gtk::Label::new(Some(
        "Prepare a request in the current thread. Review it before sending. Skills must also be installed on the paired server for its Claude Code or Codex provider. OpenCode and other providers are not qualified.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.0);
    column.append(&note);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let choose = std::rc::Rc::new(on_choose);
    for &(name, description) in WORKFLOWS {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let title = gtk::Label::new(Some(name));
        title.set_xalign(0.0);
        title.add_css_class("heading");
        row.append(&title);
        let detail = gtk::Label::new(Some(description));
        detail.set_wrap(true);
        detail.set_xalign(0.0);
        row.append(&detail);
        let button = gtk::Button::builder().child(&row).tooltip_text(format!("Prepare {name} request")).build();
        let choose = choose.clone();
        let weak_window = window.downgrade();
        button.connect_clicked(move |_| {
            choose(name);
            if let Some(window) = weak_window.upgrade() {
                window.close();
            }
        });
        list.append(&button);
    }
    column.append(
        &gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build(),
    );
    let close = gtk::Button::with_label("Cancel");
    close.set_halign(gtk::Align::End);
    let weak = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak.upgrade() {
            window.close();
        }
    });
    column.append(&close);
    window.set_child(Some(&column));
    let escape = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(escape);
    window.present();
    window
}

#[cfg(test)]
mod tests {
    #[test]
    fn preparation_preserves_draft_and_names_skill_without_sending() {
        assert_eq!(super::prepare("full-stop", "Fix the dock"), "Use the full-stop skill.\n\nFix the dock");
        assert_eq!(super::WORKFLOWS.len(), 9);
    }
}
