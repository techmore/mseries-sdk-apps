//! Private-session fixture for keyboard and accessibility tests. No real accounts.
use gtk::prelude::*;
use msdk::usage::{Coverage, Snapshot};

fn main() -> gtk::glib::ExitCode {
    let executable = std::env::args_os()
        .nth(1)
        .expect("synthetic executable path");
    let app = gtk::Application::builder()
        .application_id("io.github.techmore.MsdkUsageInteractive")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let quit = gtk::gio::SimpleAction::new("quit", None);
    let weak_app = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = weak_app.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Primary>q"]);
    app.connect_activate(move |app| {
        let snapshot = Snapshot {
            coverage: Coverage::Unknown,
            observations: vec![],
        };
        let control = msdk_gtk::usage_control::UsageControl::new(
            &snapshot,
            0,
            executable.clone().into(),
            u64::MAX,
        )
        .unwrap();
        control.widget().set_margin_top(12);
        control.widget().set_margin_bottom(12);
        control.widget().set_margin_start(12);
        control.widget().set_margin_end(12);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Usage control test")
            .default_width(480)
            .default_height(500)
            .child(control.widget())
            .build();
        let opened = control.open();
        window.connect_close_request(move |_| {
            control.close();
            gtk::glib::Propagation::Proceed
        });
        window.present();
        if let Err(error) = opened {
            eprintln!("synthetic fixture failed to start: {error}");
            app.quit();
        }
    });
    app.run_with_args::<&str>(&[])
}
