//! `mseries-tailscale`: the Tailscale window.
//!
//! Debug aids: `MSERIES_TAILSCALE_SNAPSHOT=/path.png` renders the window to a PNG after
//! `MSERIES_TAILSCALE_SNAPSHOT_DELAY` ms (default 2500) and quits; `MSERIES_TAILSCALE_DARK=1|0` forces the colour
//! scheme; `MSERIES_TAILSCALE_FIXTURES=DIR` reads status and prefs from DIR (see `cli.rs`).

use gtk::{glib, prelude::*};
use mseries_pantheon_kit::chrome;
use mseries_tailscale::strings::tr;
use mseries_tailscale::ui::window::{APP_ID, AppWindow};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    let window: Rc<RefCell<Option<Rc<AppWindow>>>> = Rc::new(RefCell::new(None));

    app.connect_startup(|app| {
        granite::init();
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
        mseries_pantheon_kit::appearance::follow_color_scheme(Some("MSERIES_TAILSCALE_DARK"));
        chrome::install_app_actions(
            app,
            chrome::About {
                program_name: tr("Tailscale"),
                icon: APP_ID.into(),
                comments: tr("Connect to your tailnet, pick an exit node, and copy any device's IP or ssh command."),
                menu_label: tr("About Tailscale"),
                website: Some("https://github.com/techmore/mseries-sdk-apps".into()),
            },
        );
    });

    let slot = window.clone();
    app.connect_activate(move |app| {
        if let Some(existing) = slot.borrow().as_ref() {
            existing.window.present();
            return;
        }
        let win = AppWindow::new(app);
        win.window.present();
        let s = slot.clone();
        win.window.connect_destroy(move |_| {
            s.borrow_mut().take();
        });
        maybe_snapshot(app, &win.window);
        *slot.borrow_mut() = Some(win);
    });
    app.run()
}

fn maybe_snapshot(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let Ok(path) = std::env::var("MSERIES_TAILSCALE_SNAPSHOT") else { return };
    let delay = std::env::var("MSERIES_TAILSCALE_SNAPSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(2500);
    // MSERIES_TAILSCALE_SNAPSHOT_TOGGLE=1 flips the connect switch first (a refused change in fixture mode).
    if std::env::var("MSERIES_TAILSCALE_SNAPSHOT_TOGGLE").as_deref() == Ok("1") {
        let window = window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay / 3), move || {
            if let Some(switch) = find_switch(window.upcast_ref()) {
                switch.set_active(!switch.is_active());
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
            (Some(node), Some(renderer)) => match renderer.render_texture(&node, None).save_to_png(&path) {
                Ok(()) => eprintln!("snapshot written to {path}"),
                Err(e) => eprintln!("snapshot failed: {e}"),
            },
            _ => eprintln!("nothing to render for the snapshot"),
        }
        app.quit();
    });
}

fn find_switch(widget: &gtk::Widget) -> Option<gtk::Switch> {
    if let Ok(switch) = widget.clone().downcast::<gtk::Switch>() {
        return Some(switch);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_switch(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}
