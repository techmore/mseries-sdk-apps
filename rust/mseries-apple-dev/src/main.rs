//! `mseries-apple-dev`: the Apple Dev window. `mseries-apple-dev [FOLDER…]` also adds project folders.
//!
//! Debug aids: `MSERIES_APPLE_DEV_SNAPSHOT=/path.png` renders the window to a PNG after
//! `MSERIES_APPLE_DEV_SNAPSHOT_DELAY` ms (default 2500) and quits; `MSERIES_APPLE_DEV_DARK=1|0`
//! forces the colour scheme; `MSERIES_APPLE_DEV_SNAPSHOT_ACTION=win.build` triggers a window action first; `MSERIES_APPLE_DEV_CONFIG`, `MSERIES_APPLE_DEV_FIXTURE_DEVICES`
//! and `MSERIES_APPLE_DEV_FIXTURE_TESTFLIGHT` point at fixture settings, devices and an App Store
//! Connect reply.

use granite::prelude::SettingsExt;
use gtk::{gio, glib, prelude::*};
use mseries_apple_dev::ui::window::{APP_ID, AppWindow};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    let window: Rc<RefCell<Option<Rc<AppWindow>>>> = Rc::new(RefCell::new(None));

    app.connect_startup(|app| {
        granite::init();
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
        follow_color_scheme();
        let quit = gio::SimpleAction::new("quit", None);
        let a = app.clone();
        quit.connect_activate(move |_, _| a.quit());
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        let about = gio::SimpleAction::new("about", None);
        let a = app.clone();
        about.connect_activate(move |_, _| show_about(&a));
        app.add_action(&about);
    });

    let w = window.clone();
    app.connect_activate(move |app| {
        present(app, &w);
    });
    let w = window.clone();
    app.connect_open(move |app, files, _| {
        let win = present(app, &w);
        for path in files.iter().filter_map(|f| f.path()) {
            win.add_project(&path, true);
        }
    });
    app.run()
}

fn present(app: &gtk::Application, slot: &Rc<RefCell<Option<Rc<AppWindow>>>>) -> Rc<AppWindow> {
    if let Some(existing) = slot.borrow().as_ref() {
        existing.window.present();
        return existing.clone();
    }
    let win = AppWindow::new(app);
    win.window.present();
    let slot_c = slot.clone();
    win.window.connect_destroy(move |_| {
        slot_c.borrow_mut().take();
    });
    *slot.borrow_mut() = Some(win.clone());
    maybe_snapshot(app, &win.window);
    win
}

#[allow(deprecated)] // Granite apps still drive GTK's prefer-dark setting.
fn follow_color_scheme() {
    let Some(gtk_settings) = gtk::Settings::default() else { return };
    if let Ok(forced) = std::env::var("MSERIES_APPLE_DEV_DARK") {
        gtk_settings.set_gtk_application_prefer_dark_theme(forced == "1");
        return;
    }
    let granite_settings = granite::Settings::default();
    let apply = move |s: &granite::Settings, g: &gtk::Settings| {
        g.set_gtk_application_prefer_dark_theme(s.prefers_color_scheme() == granite::SettingsColorScheme::Dark);
    };
    apply(&granite_settings, &gtk_settings);
    let g = gtk_settings.clone();
    granite_settings.connect_prefers_color_scheme_notify(move |s| apply(s, &g));
}

fn show_about(app: &gtk::Application) {
    let dialog = gtk::AboutDialog::builder()
        .program_name("Apple Dev")
        .logo_icon_name(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Build, run and ship iPhone apps from Linux with omarchy-apple-dev: no Mac and no Xcode install.")
        .website("https://github.com/joshuaswarren/omarchy-apple-dev")
        .website_label("omarchy-apple-dev on GitHub")
        .license_type(gtk::License::Gpl30Only)
        .modal(true)
        .build();
    if let Some(w) = app.active_window() {
        dialog.set_transient_for(Some(&w));
    }
    dialog.present();
}

fn maybe_snapshot(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let Ok(path) = std::env::var("MSERIES_APPLE_DEV_SNAPSHOT") else { return };
    let delay = std::env::var("MSERIES_APPLE_DEV_SNAPSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(2500);
    if let Ok(action) = std::env::var("MSERIES_APPLE_DEV_SNAPSHOT_ACTION") {
        let window = window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay / 3), move || {
            let name = action.trim_start_matches("win.");
            if let Some(a) = window.lookup_action(name) {
                a.activate(None);
            }
        });
    }
    let window = window.clone();
    let app = app.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        match (snapshot.to_node(), window.native().and_then(|n| n.renderer())) {
            (Some(node), Some(renderer)) => {
                let texture = renderer.render_texture(&node, None);
                match texture.save_to_png(&path) {
                    Ok(()) => eprintln!("snapshot written to {path}"),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
            _ => eprintln!("nothing to render for the snapshot"),
        }
        app.quit();
    });
}
