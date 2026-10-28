//! `mseries-updates`: the Software Updates window. Run it normally from the launcher.

use gtk::prelude::*;
use mseries_pantheon_kit::updates::CliBackend;
use mseries_updates::ui::UpdatesWindow;
use std::sync::Arc;

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.techmore.mseries.Updates")
        .build();
    app.connect_startup(|app| {
        granite::init();
        mseries_pantheon_kit::appearance::follow_color_scheme(None);
        mseries_pantheon_kit::chrome::install_app_actions(
            app,
            mseries_pantheon_kit::chrome::About {
                program_name: mseries_updates::strings::tr("Software Updates"),
                icon: "org.techmore.mseries.Updates".into(),
                comments: mseries_updates::strings::tr("Shows what the system updater has found and asks it to install, restore or switch channel."),
                menu_label: mseries_updates::strings::tr("About Software Updates"),
                website: None,
            },
        );
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
    });
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
