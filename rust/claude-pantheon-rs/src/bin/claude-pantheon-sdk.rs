//! Claude for elementary OS written in the SDK's own language (`sdk/t3.msdk`), with the real T3 server behind it.
//! The screen, behaviour and gates are the description; the server access is `claude_pantheon_rs::sdk_services`.

use std::cell::RefCell;
use std::rc::Rc;

use claude_pantheon_rs::sdk_services::T3Services;
use gtk::prelude::*;
use msdk_gtk::host::{Host, Services};

fn main() -> gtk::glib::ExitCode {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../sdk/t3.services.json")).expect("embedded manifest is valid JSON");
    msdk::vocab::load_manifest(&manifest).expect("embedded manifest registers");
    let doc = msdk::text::parse(include_str!("../../sdk/t3.msdk"))
        .unwrap_or_else(|e| panic!("embedded t3.msdk does not parse: {e}"))
        .doc;
    let app = gtk::Application::builder().application_id("io.github.techmore.ClaudePantheonSdk").build();
    let keep = RefCell::new(None);
    app.connect_activate(move |app| {
        let services: Rc<dyn Services> = Rc::new(T3Services::new());
        match Host::open_with(app, doc.clone(), None, "main", None, Some(services), false) {
            Some(h) => {
                // CLAUDE_PANTHEON_SDK_SHOT=out.png draws the window once the first data has arrived, then exits.
                if let Some(out) = std::env::var_os("CLAUDE_PANTHEON_SDK_SHOT") {
                    let (h2, app2) = (h.clone(), app.clone());
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(4500), move || {
                        match h2.screenshot(std::path::Path::new(&out)) {
                            Ok(()) => println!("wrote {}", std::path::Path::new(&out).display()),
                            Err(e) => eprintln!("screenshot: {e}"),
                        }
                        app2.quit();
                    });
                }
                *keep.borrow_mut() = Some(h);
            }
            None => app.quit(),
        }
    });
    app.run()
}
