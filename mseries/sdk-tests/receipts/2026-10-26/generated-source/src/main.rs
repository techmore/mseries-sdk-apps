use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

fn main() -> gtk::glib::ExitCode {
    // Screenshot mode is an explicit developer check. Normal activation keeps
    // the stable app ID and presents the existing window.
    let args: Vec<_> = std::env::args().skip(1).collect();
    let shot = match args.as_slice() {
        [] => None,
        [flag, path] if flag == "--screenshot" => Some(std::path::PathBuf::from(path)),
        _ => {
            eprintln!("usage: application [--screenshot output.png]");
            return gtk::glib::ExitCode::from(2);
        }
    };
    let app = gtk::Application::builder()
        .application_id("io.github.techmore.SdkSettings")
        .build();
    let quit = gtk::gio::SimpleAction::new("quit", None);
    let weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            for window in app.windows() {
                window.close();
            }
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Primary>q"]);
    let keep = RefCell::new(None::<Rc<msdk_gtk::host::Host>>);
    let failed = Rc::new(Cell::new(false));
    let failure = failed.clone();
    app.connect_activate(move |app| {
        if let Some(host) = keep.borrow().as_ref() {
            host.window().present();
            return;
        }
        let Some(host) =
            msdk_gtk::host::Host::open_doc(app, app_model::document(), None, "main", None, false)
        else {
            failure.set(true);
            app.quit();
            return;
        };
        if let Some(path) = shot.clone() {
            let (host, app, failure) = (host.clone(), app.clone(), failure.clone());
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                if let Err(error) = host.screenshot(&path) {
                    eprintln!("screenshot failed: {error}");
                    failure.set(true);
                }
                host.window().close();
                app.quit();
            });
        }
        *keep.borrow_mut() = Some(host);
    });
    let status = app.run_with_args::<&str>(&[]);
    if failed.get() {
        gtk::glib::ExitCode::FAILURE
    } else {
        status
    }
}
