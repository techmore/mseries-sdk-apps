//! Synthetic failure-state preview; the executable is supplied by the CI fixture.
use gtk::gdk;
use gtk::prelude::*;
use msdk::usage::{
    Coverage, Observation, Quota, QuotaUnit, Scope, Snapshot, Source, State, Window,
};
use std::cell::Cell;
use std::rc::Rc;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let output = std::path::PathBuf::from(args.next().expect("output PNG"));
    let executable = std::path::PathBuf::from(args.next().expect("synthetic executable"));
    let app = gtk::Application::builder()
        .application_id("io.github.techmore.MsdkUsageControlPreview")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let failed = Rc::new(Cell::new(false));
    let outcome = failed.clone();
    app.connect_activate(move |app| {
        let snapshot = Snapshot {
            coverage: Coverage::Partial,
            observations: vec![Observation {
                scope: Scope {
                    provider: "codex".into(),
                    source: Source::LocalClient,
                    account: None,
                },
                provider_label: "Codex".into(),
                source_label: "Synthetic local source".into(),
                account_label: None,
                selected: false,
                state: State::Fresh,
                observed_at: Some(100),
                windows: vec![Window {
                    key: "week".into(),
                    label: "7 day".into(),
                    quota: Some(Quota::new(2500, QuotaUnit::Used, 100, None).unwrap()),
                }],
                tokens: None,
            }],
        };
        let control =
            msdk_gtk::usage_control::UsageControl::new(&snapshot, 150, executable.clone(), 300)
                .unwrap();
        control.widget().set_margin_top(12);
        control.widget().set_margin_bottom(12);
        control.widget().set_margin_start(12);
        control.widget().set_margin_end(12);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Usage error preview")
            .default_width(480)
            .default_height(500)
            .child(control.widget())
            .build();
        window.present();
        if let Err(error) = control.open() {
            eprintln!("fixture could not start: {error}");
            outcome.set(true);
            app.quit();
            return;
        }
        let app = app.clone();
        let path = output.clone();
        let failed = outcome.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            if control.status_label().text() != "Retry when ready"
                && std::time::Instant::now() < deadline
            {
                return gtk::glib::ControlFlow::Continue;
            }
            let result = (|| -> Result<(), String> {
                if control.status_label().text() != "Retry when ready" {
                    return Err("fixture failure was not shown".into());
                }
                let (width, height) = (window.width(), window.height());
                if width <= 0 || height <= 0 {
                    return Err("window has no size".into());
                }
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let image = gtk::Snapshot::new();
                paintable.snapshot(
                    image.upcast_ref::<gdk::Snapshot>(),
                    width as f64,
                    height as f64,
                );
                let node = image.to_node().ok_or("nothing to draw")?;
                let renderer = window
                    .native()
                    .and_then(|native| native.renderer())
                    .ok_or("no renderer")?;
                renderer
                    .render_texture(&node, None)
                    .save_to_png(&path)
                    .map_err(|error| error.to_string())
            })();
            if let Err(error) = result {
                eprintln!("preview capture failed: {error}");
                failed.set(true);
            }
            control.close();
            app.quit();
            gtk::glib::ControlFlow::Break
        });
    });
    app.run_with_args::<&str>(&[]);
    if failed.get() {
        std::process::exit(1);
    }
}
