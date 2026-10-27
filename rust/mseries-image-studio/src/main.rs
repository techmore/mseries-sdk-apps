use gtk::prelude::*;
use mseries_image_studio::{APP_ID, ui::ImageStudioWindow};
use std::{cell::RefCell, rc::Rc};

fn main() -> gtk::glib::ExitCode {
    if std::env::var_os("GSK_RENDERER").is_none() {
        // GTK reads this during startup. Keep the lightweight cairo renderer as the safe default on Asahi.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    // M_SERIES_IMAGE_STUDIO_DEV lets a development build run beside the installed app.
    let flags = if std::env::var_os("M_SERIES_IMAGE_STUDIO_DEV").is_some() {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::FLAGS_NONE
    };
    let application = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    application.connect_startup(|app| {
        granite::init();
        gtk::Window::set_default_icon_name(APP_ID);
        mseries_image_studio::host::recover_abandoned_workflows(
            &gtk::glib::user_data_dir().join("mseries-image-studio"),
        );
        // Our Pantheon compositor may not expose XSettings. Read the desktop's
        // own GTK theme and color preference instead of falling back to Adwaita.
        if std::env::var_os("GTK_THEME").is_none() {
            if let Some(schema) = gtk::gio::SettingsSchemaSource::default()
                .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
            {
                let desktop =
                    gtk::gio::Settings::new_full(&schema, None::<&gtk::gio::SettingsBackend>, None);
                if let Some(settings) = gtk::Settings::default() {
                    let refresh = move || {
                        let theme = desktop.string("gtk-theme");
                        if settings.gtk_theme_name().as_deref() != Some(theme.as_str()) {
                            settings.set_gtk_theme_name(Some(&theme));
                        }
                        let font = desktop.string("font-name");
                        if settings.gtk_font_name().as_deref() != Some(font.as_str()) {
                            settings.set_gtk_font_name(Some(&font));
                        }
                        let scheme = match desktop.string("color-scheme").as_str() {
                            "prefer-dark" => gtk::InterfaceColorScheme::Dark,
                            "prefer-light" => gtk::InterfaceColorScheme::Light,
                            _ => gtk::InterfaceColorScheme::Default,
                        };
                        if settings.gtk_interface_color_scheme() != scheme {
                            settings.set_gtk_interface_color_scheme(scheme);
                        }
                    };
                    refresh();
                    gtk::glib::timeout_add_seconds_local(5, move || {
                        refresh();
                        gtk::glib::ControlFlow::Continue
                    });
                }
            }
        }
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
        let quit = gtk::gio::SimpleAction::new("quit", None);
        let app_weak = app.downgrade();
        quit.connect_activate(move |_, _| {
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        });
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        let about = gtk::gio::SimpleAction::new("about", None);
        let app_weak = app.downgrade();
        about.connect_activate(move |_, _| {
            if let Some(app) = app_weak.upgrade() {
                let dialog = gtk::AboutDialog::builder()
                    .application(&app)
                    .program_name("Image Studio")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("A native Pantheon image creation workspace for M-Series.")
                    .logo_icon_name(APP_ID)
                    .license_type(gtk::License::Gpl30Only)
                    .build();
                dialog.present();
            }
        });
        app.add_action(&about);
    });
    let current_window = Rc::new(RefCell::new(None::<ImageStudioWindow>));
    let window_slot = current_window.clone();
    application.connect_activate(move |app| {
        if let Some(window) = window_slot.borrow().as_ref() {
            window.window().present();
            return;
        }
        let window = ImageStudioWindow::build(app);
        window.window().present();
        *window_slot.borrow_mut() = Some(window);
    });
    application.run()
}
