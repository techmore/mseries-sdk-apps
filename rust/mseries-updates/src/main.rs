//! `mseries-updates`: the Software Updates window. Run it normally from the launcher.

use gtk::prelude::*;
use mseries_updates::backend::CliBackend;
use mseries_updates::ui::UpdatesWindow;
use std::sync::Arc;

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder().application_id("org.techmore.mseries.Updates").build();
    app.connect_startup(|_| granite::init());
    app.connect_activate(|app| {
        if let Some(existing) = app.active_window() {
            existing.present();
            return;
        }
        let window = UpdatesWindow::new(app, Arc::new(CliBackend::system()));
        window.window().present();
        window.refresh();
    });
    app.run()
}
