mod app;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    let application = gtk::Application::builder()
        .application_id("org.techmore.mseries.SystemMonitor")
        .build();
    application.connect_startup(|_| {
        granite::init();
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
    });
    application.connect_activate(app::build);
    application.run()
}
