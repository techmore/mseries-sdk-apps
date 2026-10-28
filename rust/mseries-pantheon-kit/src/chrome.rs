//! Window chrome every app shares: Quit and About actions with their shortcuts, the primary menu that reaches
//! them, and a Granite toast overlay. Keeps each app's startup to a couple of lines.

use granite::prelude::ToastExt;
use gtk::gio;
use gtk::prelude::*;

/// What the About dialog shows.
#[derive(Clone)]
pub struct About {
    pub program_name: String,
    /// Icon name; by convention the application id.
    pub icon: String,
    pub comments: String,
    /// Menu label, for example "About Disk Space".
    pub menu_label: String,
    pub website: Option<String>,
}

/// Registers `app.quit` (Ctrl+Q) and `app.about`. Call from `startup`.
pub fn install_app_actions(app: &gtk::Application, about: About) {
    let quit = gio::SimpleAction::new("quit", None);
    let handle = app.clone();
    quit.connect_activate(move |_, _| handle.quit());
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    let action = gio::SimpleAction::new("about", None);
    let handle = app.clone();
    action.connect_activate(move |_, _| show_about(&handle, &about));
    app.add_action(&action);
}

fn show_about(app: &gtk::Application, about: &About) {
    let dialog = gtk::AboutDialog::builder()
        .program_name(&about.program_name)
        .logo_icon_name(&about.icon)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(&about.comments)
        .license_type(gtk::License::Gpl30Only)
        .modal(true)
        .build();
    if let Some(website) = &about.website {
        dialog.set_website(Some(website));
    }
    if let Some(window) = app.active_window() {
        dialog.set_transient_for(Some(&window));
    }
    dialog.present();
}

/// The header bar's primary menu: About and Quit.
pub fn primary_menu(about_label: &str, quit_label: &str) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some(about_label), Some("app.about"));
    menu.append(Some(quit_label), Some("app.quit"));
    let button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .build();
    crate::set_accessible_label(&button, "Main menu");
    button
}

/// Wraps `child` in an overlay that carries a toast; returns both.
pub fn toast_overlay(child: &impl IsA<gtk::Widget>) -> (gtk::Overlay, granite::Toast) {
    let toast = granite::Toast::new("");
    toast.set_halign(gtk::Align::Center);
    toast.set_valign(gtk::Align::End);
    toast.set_margin_bottom(44);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(child));
    overlay.add_overlay(&toast);
    (overlay, toast)
}

/// Shows `message` in `toast`.
pub fn show_toast(toast: &granite::Toast, message: &str) {
    toast.set_title(message);
    toast.send_notification();
}
