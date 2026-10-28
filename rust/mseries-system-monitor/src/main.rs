mod app;
mod strings;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    let application = gtk::Application::builder()
        .application_id("org.techmore.mseries.SystemMonitor")
        .build();
    application.connect_startup(|application| {
        granite::init();
        mseries_pantheon_kit::appearance::follow_color_scheme(None);
        mseries_pantheon_kit::chrome::install_app_actions(
            application,
            mseries_pantheon_kit::chrome::About {
                program_name: strings::tr("System Monitor"),
                icon: "org.techmore.mseries.SystemMonitor".into(),
                comments: strings::tr("Live CPU and memory use, with btop for process detail."),
                menu_label: strings::tr("About System Monitor"),
                website: None,
            },
        );
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
    });
    application.connect_activate(app::build);
    application.run()
}
