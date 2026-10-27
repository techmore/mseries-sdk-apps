//! Synthetic read-only preview for native screenshot review; no real accounts.
use gtk::gdk;
use gtk::prelude::*;
use msdk::usage::{
    Coverage, Observation, Quota, QuotaUnit, Scope, Snapshot, Source, State, Window,
};
use std::cell::Cell;
use std::rc::Rc;

fn main() {
    let output = std::env::args()
        .nth(1)
        .expect("usage: usage_preview OUTPUT.png");
    let app = gtk::Application::builder()
        .application_id("io.github.techmore.MsdkUsagePreview")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let failed = Rc::new(Cell::new(false));
    let outcome = failed.clone();
    app.connect_activate(move |app| {
        let base = Observation {
            scope: Scope {
                provider: "claude".into(),
                source: Source::LocalClient,
                account: Some("fixture-one".into()),
            },
            provider_label: "Claude".into(),
            source_label: "This device".into(),
            account_label: Some("Work account".into()),
            selected: true,
            state: State::Fresh,
            observed_at: Some(100),
            windows: vec![Window {
                key: "week".into(),
                label: "7 day".into(),
                quota: Some(Quota::new(2500, QuotaUnit::Used, 100, None).unwrap()),
            }],
            tokens: None,
        };
        let other = Observation {
            scope: Scope {
                account: Some("fixture-two".into()),
                ..base.scope.clone()
            },
            account_label: Some("Personal account with a longer label".into()),
            selected: false,
            state: State::Stale,
            ..base.clone()
        };
        let codex = Observation {
            scope: Scope {
                provider: "codex".into(),
                account: Some("fixture-three".into()),
                ..base.scope.clone()
            },
            provider_label: "Codex".into(),
            account_label: Some("Primary account".into()),
            ..base.clone()
        };
        let codex_other = Observation {
            scope: Scope {
                account: Some("fixture-four".into()),
                ..codex.scope.clone()
            },
            account_label: Some("Second account".into()),
            selected: false,
            ..codex.clone()
        };
        let grok = Observation {
            scope: Scope {
                provider: "grok".into(),
                account: Some("fixture-five".into()),
                ..base.scope.clone()
            },
            provider_label: "Grok".into(),
            account_label: Some("Grok account".into()),
            selected: false,
            state: State::Unknown,
            windows: vec![Window {
                key: "week".into(),
                label: "7 day".into(),
                quota: None,
            }],
            ..base.clone()
        };
        let opencode = Observation {
            scope: Scope {
                provider: "opencode".into(),
                account: Some("fixture-six".into()),
                ..base.scope.clone()
            },
            provider_label: "OpenCode".into(),
            account_label: Some("OpenCode account".into()),
            selected: false,
            state: State::Unavailable,
            windows: vec![],
            ..base.clone()
        };
        let snapshot = Snapshot {
            coverage: Coverage::Partial,
            observations: vec![base, other, codex, codex_other, grok, opencode],
        };
        let panel =
            msdk_gtk::usage::UsagePanel::new(&snapshot, 150).expect("synthetic snapshot is valid");
        let scroll = panel.widget().clone();
        scroll.set_margin_top(12);
        scroll.set_margin_bottom(12);
        scroll.set_margin_start(12);
        scroll.set_margin_end(12);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Usage preview")
            .default_width(480)
            .default_height(500)
            .child(&scroll)
            .build();
        window.present();
        if std::env::var_os("MSDK_USAGE_SCROLL_END").is_some() {
            let adjustment = scroll.vadjustment();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
            });
        }
        let app = app.clone();
        let path = output.clone();
        let failed = outcome.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            let result = (|| -> Result<(), String> {
                let (width, height) = (window.width(), window.height());
                if width <= 0 || height <= 0 {
                    return Err("window has no size".into());
                }
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(
                    snapshot.upcast_ref::<gdk::Snapshot>(),
                    width as f64,
                    height as f64,
                );
                let node = snapshot.to_node().ok_or("nothing to draw")?;
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
            app.quit();
        });
    });
    app.run_with_args::<&str>(&[]);
    if failed.get() {
        std::process::exit(1);
    }
}
