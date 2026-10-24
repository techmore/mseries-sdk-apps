//! Small building blocks shared by the pages and dialogs.

use gtk::{gio, glib, pango, prelude::*};
use mseries_pantheon_kit::{set_box_spacing_text_units, set_margins_text_units};
use std::path::Path;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).wrap_mode(pango::WrapMode::WordChar).build();
    for class in classes {
        l.add_css_class(class);
    }
    l
}

/// A page body: a scrolled, width-limited column with text-relative margins.
pub fn page() -> (gtk::ScrolledWindow, gtk::Box) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("pantheon-page");
    set_box_spacing_text_units(&column, 1.1);
    set_margins_text_units(&column, 1.4, 1.8, 1.8, 1.8);
    let clamp = gtk::Box::new(gtk::Orientation::Vertical, 0);
    clamp.set_halign(gtk::Align::Fill);
    clamp.append(&column);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    (scroll, column)
}

/// Icon, title and subtitle at the top of a page, with optional trailing widgets.
pub fn page_header(
    icon: &str,
    title: &str,
    subtitle: &str,
) -> (gtk::Box, gtk::Image, gtk::Label, gtk::Label, gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    set_box_spacing_text_units(&row, 0.8);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(48);
    image.set_valign(gtk::Align::Center);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = label(title, &["title-2"]);
    let s = label(subtitle, &["dim-label"]);
    text.append(&t);
    text.append(&s);
    let trailing = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    trailing.set_valign(gtk::Align::Center);
    row.append(&image);
    row.append(&text);
    row.append(&trailing);
    (row, image, t, s, trailing)
}

/// A titled group of rows in a framed list, the Switchboard/System Settings look.
pub fn section(title: &str) -> (gtk::Box, gtk::ListBox) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    set_box_spacing_text_units(&group, 0.35);
    group.append(&granite::HeaderLabel::new(title));
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("rich-list");
    let frame = gtk::Frame::builder().child(&list).build();
    group.append(&frame);
    (group, list)
}

/// A list row: icon at the start, title and description, controls at the end.
pub fn row(icon: Option<&str>, title: &str, detail: &str) -> (gtk::ListBoxRow, gtk::Box, gtk::Label, gtk::Label) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.add_css_class("pantheon-row");
    if let Some(icon) = icon {
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(24);
        image.set_valign(gtk::Align::Center);
        image.set_accessible_role(gtk::AccessibleRole::Presentation);
        content.append(&image);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = label(title, &[]);
    let d = label(detail, &["dim-label", "pantheon-caption"]);
    d.set_visible(!detail.is_empty());
    text.append(&t);
    text.append(&d);
    content.append(&text);
    let trailing = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    trailing.set_valign(gtk::Align::Center);
    content.append(&trailing);
    let row = gtk::ListBoxRow::builder().child(&content).activatable(false).build();
    (row, trailing, t, d)
}

pub fn button(text: &str, classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.set_valign(gtk::Align::Center);
    for class in classes {
        b.add_css_class(class);
    }
    b
}

pub fn clear_list(list: &gtk::ListBox) {
    list.remove_all();
}

pub fn tilde(path: &Path) -> String {
    let home = crate::settings::home();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Open a folder or file with the desktop's default handler.
pub fn open_path(parent: &impl IsA<gtk::Window>, path: &Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(Some(parent), None::<&gio::Cancellable>, |_| {});
}

pub fn show_in_files(parent: &impl IsA<gtk::Window>, path: &Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.open_containing_folder(Some(parent), None::<&gio::Cancellable>, |_| {});
}

pub fn open_uri(parent: &impl IsA<gtk::Window>, uri: &str) {
    gtk::UriLauncher::new(uri).launch(Some(parent), None::<&gio::Cancellable>, |_| {});
}

/// "5 minutes ago" style age for a file time.
pub fn age(time: std::time::SystemTime) -> String {
    let secs = time.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => plural(secs / 60, "minute"),
        3600..=86_399 => plural(secs / 3600, "hour"),
        _ => {
            let dt = glib::DateTime::from_unix_local(
                time.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
            );
            dt.and_then(|d| d.format("%b %e, %H:%M"))
                .map(|s| s.to_string())
                .unwrap_or_else(|_| plural(secs / 86_400, "day"))
        }
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") }
}
