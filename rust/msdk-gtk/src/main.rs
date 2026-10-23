//! `msdk-run <app.ui.json> [--surface NAME] [--watch]`: render an app description live.

use std::cell::RefCell;
use std::path::PathBuf;

use gtk::prelude::*;
use msdk_gtk::host::Host;

fn main() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.iter().find(|a| {
        !a.starts_with("--")
            && args
                .iter()
                .position(|x| x == *a)
                .is_some_and(|i| i == 0 || args[i - 1] != "--surface")
    }) else {
        eprintln!("usage: msdk-run <app.ui.json> [--surface NAME] [--watch]");
        return gtk::glib::ExitCode::from(2);
    };
    let surface = args
        .iter()
        .position(|a| a == "--surface")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "main".into());
    let watch = args.iter().any(|a| a == "--watch");
    let path = PathBuf::from(file);
    let id = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|d| d["app"]["id"].as_str().map(String::from))
        .unwrap_or_else(|| "io.github.techmore.MsdkRun".into());
    let app = gtk::Application::builder()
        .application_id(format!("{id}.Preview"))
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let keep = RefCell::new(None);
    app.connect_activate(
        move |app| match Host::open(app, &path, &surface, None, watch) {
            Some(h) => *keep.borrow_mut() = Some(h),
            None => app.quit(),
        },
    );
    app.run_with_args::<&str>(&[])
}
