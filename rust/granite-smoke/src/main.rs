//! Drives the system Granite 7 from Rust through granite-rs and exits.
//! Builds a few Granite widgets in an unmapped window, so it needs a display
//! (run it under Xvfb or a nested compositor, never on the owner's screen).

use granite::prelude::PlaceholderExt;
use gtk::prelude::*;

fn main() -> std::process::ExitCode {
    if gtk::init().is_err() {
        eprintln!("FAIL: no display for GTK");
        return std::process::ExitCode::FAILURE;
    }
    granite::init();

    let placeholder = granite::Placeholder::new("Rust drives Granite");
    placeholder.set_description("granite-rs bindings, generated from Granite-7.0.gir");
    let toast = granite::Toast::new("hello from Rust");
    let header = granite::HeaderLabel::new("Pantheon");

    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.append(&header);
    column.append(&placeholder);
    column.append(&toast);
    let window = gtk::Window::new();
    window.set_child(Some(&column));

    let style = granite::StyleManager::default();
    let format = granite::date_time_get_default_time_format(false, false);
    println!(
        "PASS: Granite widgets built from Rust (title={:?}, style manager={}, time format={format})",
        placeholder.title(),
        style.type_().name(),
    );
    std::process::ExitCode::SUCCESS
}
