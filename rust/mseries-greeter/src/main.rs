//! The login screen: greetd on one side, the SDK renderer on the other.
//! Run under greetd inside a kiosk compositor, for example `cage -s -- mseries-greeter`.
//! The screen is `data/greeter.ui.json`, embedded in the binary; the logic is [`mseries_greeter::logic::GreeterLogic`].

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use futures_channel::mpsc::UnboundedSender;
use gtk::{gdk, prelude::*};
use msdk_gtk::host::{Event, Host, LogicFactory};
use mseries_greeter::greetd;
use mseries_greeter::logic::{GreeterLogic, Work};
use serde_json::json;

const SCREEN: &str = include_str!("../data/greeter.ui.json");
const CSS: &str = "
window.msdk-kiosk { background: linear-gradient(160deg, #0d1520, #172334); }
window.msdk-kiosk .msdk-card { background: alpha(#ffffff, 0.06); padding: 36px 40px; border-radius: 18px; }
";

/// Owns the greetd socket. Replies go back to the UI thread as `_greetd` events; a cancel's reply is dropped.
fn spawn_worker(mut sock: UnixStream, to_ui: UnboundedSender<Event>) -> mpsc::Sender<Work> {
    let (tx, rx) = mpsc::channel::<Work>();
    thread::spawn(move || {
        for work in rx {
            let (req, forward) = match work {
                Work::Send(r) => (r, true),
                Work::Cancel => (greetd::Request::CancelSession, false),
            };
            match greetd::write_request(&mut sock, &req)
                .and_then(|_| greetd::read_response(&mut sock))
            {
                Ok(reply) if forward => {
                    let _ =
                        to_ui.unbounded_send(("_greetd".into(), serde_json::to_value(reply).ok()));
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = to_ui.unbounded_send(("_lost".into(), Some(json!(e.to_string()))));
                }
            }
        }
    });
    tx
}

fn main() -> gtk::glib::ExitCode {
    let Ok(path) = std::env::var("GREETD_SOCK") else {
        eprintln!("mseries-greeter: GREETD_SOCK is not set; run this under greetd");
        return gtk::glib::ExitCode::FAILURE;
    };
    let sock = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mseries-greeter: cannot connect to {path}: {e}");
            return gtk::glib::ExitCode::FAILURE;
        }
    };
    let session_cmd: Vec<String> = std::env::var("MSERIES_GREETER_CMD")
        .unwrap_or_else(|_| "pantheon-rust-session".into())
        .split_whitespace()
        .map(String::from)
        .collect();
    let last_user_file = PathBuf::from(
        std::env::var("MSERIES_GREETER_STATE")
            .unwrap_or_else(|_| "/var/cache/mseries-greeter/last-user".into()),
    );
    let last_user = std::fs::read_to_string(&last_user_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let doc: serde_json::Value =
        serde_json::from_str(SCREEN).expect("embedded greeter.ui.json is valid JSON");

    let app = gtk::Application::builder()
        .application_id("io.github.techmore.Greeter")
        .build();
    let state = RefCell::new(Some((sock, last_user, last_user_file, session_cmd)));
    let keep = RefCell::new(None);
    app.connect_activate(move |app| {
        let Some((sock, last_user, last_user_file, session_cmd)) = state.borrow_mut().take() else {
            return;
        };
        if let Some(settings) = gtk::Settings::default() {
            #[allow(deprecated)]
            // GTK 4.20 deprecates the flag; there is no greeter-user colour-scheme setting to replace it
            settings.set_gtk_application_prefer_dark_theme(true);
        }
        let factory: LogicFactory = Box::new(move |events| {
            let work = spawn_worker(sock, events);
            Box::new(GreeterLogic::new(
                session_cmd,
                last_user,
                last_user_file,
                work,
            ))
        });
        match Host::open_doc(app, doc.clone(), None, "main", Some(factory), false) {
            Some(host) => {
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
