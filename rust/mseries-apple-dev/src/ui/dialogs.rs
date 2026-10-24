//! New App, Install Toolchain and Preferences. Each is a small modal window in the elementary dialog
//! layout: heading, explanation, fields, then Cancel and the primary action at the end.

use super::widgets::{button, label, tilde};
use crate::commands::InstallMode;
use crate::settings::Settings;
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{set_box_spacing_text_units, set_margins_text_units};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

struct Dialog {
    window: gtk::Window,
    fields: gtk::Box,
    accept: gtk::Button,
}

fn dialog(parent: &gtk::Window, icon: &str, title: &str, detail: &str, accept: &str) -> Dialog {
    let window = gtk::Window::builder().transient_for(parent).modal(true).resizable(false).title(title).build();
    window.add_css_class("dialog");
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    set_box_spacing_text_units(&body, 0.9);
    set_margins_text_units(&body, 1.1, 1.2, 1.0, 1.2);
    body.set_width_request(440);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(48);
    image.set_valign(gtk::Align::Start);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_hexpand(true);
    text.append(&label(title, &["title-3"]));
    text.append(&label(detail, &["dim-label"]));
    top.append(&image);
    top.append(&text);
    body.append(&top);

    let fields = gtk::Box::new(gtk::Orientation::Vertical, 0);
    set_box_spacing_text_units(&fields, 0.6);
    body.append(&fields);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = button("Cancel", &[]);
    let accept = button(accept, &["suggested-action"]);
    buttons.append(&cancel);
    buttons.append(&accept);
    body.append(&buttons);

    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&body));
    window.set_child(Some(&handle));
    window.set_default_widget(Some(&accept));

    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let esc = gtk::EventControllerKey::new();
    let w = window.clone();
    esc.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(esc);
    Dialog { window, fields, accept }
}

fn field(fields: &gtk::Box, title: &str, widget: &impl IsA<gtk::Widget>, hint: &str) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let l = label(title, &["heading"]);
    group.append(&l);
    group.append(widget);
    if !hint.is_empty() {
        group.append(&label(hint, &["dim-label", "pantheon-caption"]));
    }
    fields.append(&group);
}

/// A button that shows the chosen path and opens a file chooser.
fn path_button(
    parent: &gtk::Window,
    folder: bool,
    initial: Option<PathBuf>,
    filter: Option<(&str, &str)>,
) -> (gtk::Button, Rc<RefCell<Option<PathBuf>>>) {
    let chosen = Rc::new(RefCell::new(initial));
    let text = |p: &Option<PathBuf>| p.as_deref().map(tilde).unwrap_or_else(|| "Choose…".into());
    let b = gtk::Button::with_label(&text(&chosen.borrow()));
    b.set_halign(gtk::Align::Start);
    if let Some(child) = b.child().and_downcast::<gtk::Label>() {
        child.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        child.set_max_width_chars(48);
    }
    let parent = parent.clone();
    let chosen_c = chosen.clone();
    let filter = filter.map(|(name, pattern)| {
        let f = gtk::FileFilter::new();
        f.set_name(Some(name));
        f.add_pattern(pattern);
        f
    });
    b.connect_clicked(move |b| {
        let dialog = gtk::FileDialog::builder().modal(true).build();
        if let Some(p) = chosen_c.borrow().as_ref() {
            let start = if folder { p.clone() } else { p.parent().map(PathBuf::from).unwrap_or_default() };
            dialog.set_initial_folder(Some(&gio::File::for_path(start)));
        }
        if let Some(f) = &filter {
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(f);
            dialog.set_filters(Some(&filters));
        }
        let chosen = chosen_c.clone();
        let b = b.clone();
        let done = move |result: Result<gio::File, glib::Error>| {
            if let Some(path) = result.ok().and_then(|f| f.path()) {
                b.set_label(&tilde(&path));
                *chosen.borrow_mut() = Some(path);
            }
        };
        if folder {
            dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, done);
        } else {
            dialog.open(Some(&parent), None::<&gio::Cancellable>, done);
        }
    });
    (b, chosen)
}

/// Valid `xtool new` name: a Swift identifier.
pub fn valid_app_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn new_app(parent: &gtk::Window, default_folder: PathBuf, on_create: impl Fn(PathBuf, String) + 'static) {
    let d = dialog(
        parent,
        "phone-symbolic",
        "New iPhone App",
        "Creates a SwiftUI app with xtool new. It opens in the sidebar when the template is ready.",
        "Create",
    );
    let name = gtk::Entry::builder().placeholder_text("HelloOmarchy").activates_default(true).build();
    field(&d.fields, "Name", &name, "Letters, digits and underscores; it becomes the Swift module name.");
    let (folder, chosen) = path_button(&d.window, true, Some(default_folder), None);
    field(&d.fields, "Location", &folder, "");
    d.accept.set_sensitive(false);
    let accept = d.accept.clone();
    name.connect_changed(move |e| accept.set_sensitive(valid_app_name(e.text().trim())));
    let w = d.window.clone();
    d.accept.connect_clicked(move |_| {
        let n = name.text().trim().to_string();
        if let Some(dir) = chosen.borrow().clone().filter(|_| valid_app_name(&n)) {
            on_create(dir, n);
            w.close();
        }
    });
    d.window.present();
}

pub fn install(parent: &gtk::Window, xip_hint: &str, on_install: impl Fn(InstallMode, Option<PathBuf>) + 'static) {
    let d = dialog(
        parent,
        "applications-development",
        "Install the Toolchain",
        "Runs install-toolchain.sh in the Activity pane. It installs Swift, builds xtool from source (about 10 minutes) and registers the iOS SDK. It may ask for your password.",
        "Install",
    );
    let full = gtk::CheckButton::with_label("Full: iOS SDK from an Xcode download");
    let lite = gtk::CheckButton::with_label("No Xcode download (experimental)");
    lite.set_group(Some(&full));
    full.set_active(true);
    let mode_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    mode_box.append(&full);
    mode_box.append(&label(
        "All of SwiftUI and the system frameworks, extensions, TestFlight.",
        &["dim-label", "pantheon-caption"],
    ));
    mode_box.append(&lite);
    mode_box.append(&label(
        "A SwiftUI subset and Flutter; reads the SDK from Apple's public iOS update.",
        &["dim-label", "pantheon-caption"],
    ));
    field(&d.fields, "Mode", &mode_box, "");
    let (xip, chosen) = path_button(&d.window, false, None, Some(("Xcode archive", "*.xip")));
    field(&d.fields, "Xcode .xip (optional)", &xip, xip_hint);
    let xip_c = xip.clone();
    full.connect_toggled(move |b| xip_c.set_sensitive(b.is_active()));
    let w = d.window.clone();
    d.accept.connect_clicked(move |_| {
        let mode = if full.is_active() { InstallMode::Full } else { InstallMode::NoXcode };
        on_install(mode, chosen.borrow().clone());
        w.close();
    });
    d.window.present();
}

pub fn preferences(parent: &gtk::Window, settings: &Settings, on_save: impl Fn(Settings) + 'static) {
    let d = dialog(
        parent,
        "preferences-system",
        "App Store Connect",
        "Upload and notarize use an App Store Connect API key (App Manager role). The key stays in its .p8 file; Apple Dev stores only its path and IDs.",
        "Save",
    );
    let initial = Some(PathBuf::from(&settings.asc_key_path)).filter(|p| !p.as_os_str().is_empty());
    let (key, chosen) = path_button(&d.window, false, initial, Some(("App Store Connect key", "*.p8")));
    field(&d.fields, "Key file", &key, "AuthKey_XXXXXXXXXX.p8 from Users and Access › Integrations.");
    let key_id = gtk::Entry::builder().text(&settings.asc_key_id).placeholder_text("XXXXXXXXXX").build();
    field(&d.fields, "Key ID", &key_id, "");
    let issuer = gtk::Entry::builder().text(&settings.asc_issuer_id).placeholder_text("Issuer UUID").build();
    field(&d.fields, "Issuer ID", &issuer, "");
    let team =
        gtk::Entry::builder().text(&settings.team_id).placeholder_text("Optional").activates_default(true).build();
    field(&d.fields, "Team ID", &team, "Ten characters. Device Build fills $(AppIdentifierPrefix) with it.");
    let link = gtk::LinkButton::with_label(
        "https://appstoreconnect.apple.com/access/integrations/api",
        "Open App Store Connect",
    );
    link.set_halign(gtk::Align::Start);
    d.fields.append(&link);
    let base = settings.clone();
    let w = d.window.clone();
    d.accept.connect_clicked(move |_| {
        let mut s = base.clone();
        s.asc_key_path = chosen.borrow().as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        s.asc_key_id = key_id.text().trim().to_string();
        s.asc_issuer_id = issuer.text().trim().to_string();
        s.team_id = team.text().trim().to_string();
        on_save(s);
        w.close();
    });
    d.window.present();
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_names() {
        assert!(super::valid_app_name("HelloOmarchy"));
        assert!(super::valid_app_name("_x1"));
        assert!(!super::valid_app_name("1App"));
        assert!(!super::valid_app_name("My App"));
        assert!(!super::valid_app_name(""));
    }
}
