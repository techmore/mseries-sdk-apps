//! The dock: shows the `panel` surface of `msdk/examples/dock.ui.json` with [`DockLogic`] behind it.

use std::cell::RefCell;

use gtk::{gdk, prelude::*};
use msdk_gtk::host::{Host, LogicFactory};
use mseries_dock::logic::DockLogic;

const SCREEN: &str = include_str!("../../msdk/examples/dock.ui.json");
const CSS: &str = "window.msdk-dock { background: alpha(@theme_bg_color, 0.85); border-radius: 18px; padding: 6px 10px; }";

fn main() -> gtk::glib::ExitCode {
    let doc: serde_json::Value =
        serde_json::from_str(SCREEN).expect("embedded dock.ui.json is valid JSON");
    let app = gtk::Application::builder()
        .application_id("io.github.techmore.Dock")
        .build();
    let keep = RefCell::new(None);
    app.connect_activate(move |app| {
        let factory: LogicFactory = Box::new(|_events| Box::new(DockLogic::from_system()));
        match Host::open_doc(app, doc.clone(), None, "dock", Some(factory), false) {
            Some(host) => {
                host.window().add_css_class("msdk-dock");
                if let Some(display) = gdk::Display::default() {
                    let provider = gtk::CssProvider::new();
                    provider.load_from_string(CSS);
                    gtk::style_context_add_provider_for_display(
                        &display,
                        &provider,
                        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                    );
                }
                host.post("_start", None);
                *keep.borrow_mut() = Some(host);
            }
            None => app.quit(),
        }
    });
    app.run()
}
