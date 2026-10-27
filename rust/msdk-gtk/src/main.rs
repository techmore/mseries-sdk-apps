//! `msdk-run <app.ui.json> [--surface NAME] [--watch]`: render an app description live.

use std::cell::RefCell;
use std::path::PathBuf;

use gtk::prelude::*;
use std::rc::Rc;

use msdk_gtk::fixture::FixtureServices;
use msdk_gtk::host::{Host, Services};

fn main() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.iter().find(|a| {
        !a.starts_with("--")
            && args
                .iter()
                .position(|x| x == *a)
                .is_some_and(|i| i == 0 || args[i - 1] != "--surface")
    }) else {
        eprintln!(
            "usage: msdk-run <app.ui.json|app.msdk> [--surface NAME] [--watch] [--services manifest.json] [--fixture data.json] [--screenshot out.png]"
        );
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
    let id = msdk::text::load(&path)
        .ok()
        .and_then(|(d, _)| d["app"]["id"].as_str().map(String::from))
        .unwrap_or_else(|| "io.github.techmore.MsdkRun".into());
    let app = gtk::Application::builder()
        .application_id(format!("{id}.Preview"))
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let opt = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // Plug-in service manifests: every *.services.json next to the app, plus --services.
    let mut manifests: Vec<PathBuf> = Vec::new();
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            dir
        };
        let entries = match std::fs::read_dir(dir)
            .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
        {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("{}: {error}", dir.display());
                return gtk::glib::ExitCode::FAILURE;
            }
        };
        manifests.extend(
            entries
                .into_iter()
                .map(|entry| entry.path())
                .filter(|path| path.to_string_lossy().ends_with(".services.json")),
        );
    }
    for (index, argument) in args.iter().enumerate() {
        if argument == "--services" {
            let Some(path) = args.get(index + 1).filter(|path| !path.starts_with("--")) else {
                eprintln!("--services requires a manifest path");
                return gtk::glib::ExitCode::FAILURE;
            };
            manifests.push(path.into());
        }
    }
    manifests.sort();
    manifests.dedup();
    for m in manifests {
        if let Err(e) = std::fs::read_to_string(&m)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
            .and_then(|v| msdk::vocab::load_manifest(&v))
        {
            eprintln!("{}: {e}", m.display());
            return gtk::glib::ExitCode::FAILURE;
        }
    }
    let services: Option<Rc<dyn Services>> = match opt("--fixture") {
        Some(f) => match FixtureServices::load(std::path::Path::new(&f)) {
            Ok(s) => Some(Rc::new(s)),
            Err(e) => {
                eprintln!("{f}: {e}");
                return gtk::glib::ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let shot = opt("--screenshot");
    let keep = RefCell::new(None);
    let failed = Rc::new(std::cell::Cell::new(false));
    let activation_failed = failed.clone();
    app.connect_activate(move |app| {
        let doc = match msdk::text::load(&path) {
            Ok((d, _)) => d,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                activation_failed.set(true);
                return app.quit();
            }
        };
        match Host::open_with(
            app,
            doc,
            Some(&path),
            &surface,
            None,
            services.clone(),
            watch,
        ) {
            Some(h) => {
                if let Some(out) = shot.clone() {
                    let (h2, app2) = (h.clone(), app.clone());
                    let screenshot_failed = activation_failed.clone();
                    gtk::glib::timeout_add_local_once(
                        std::time::Duration::from_millis(1800),
                        move || {
                            match h2.screenshot(std::path::Path::new(&out)) {
                                Ok(()) => println!("wrote {out}"),
                                Err(e) => {
                                    screenshot_failed.set(true);
                                    eprintln!("screenshot: {e}");
                                }
                            }
                            app2.quit();
                        },
                    );
                }
                *keep.borrow_mut() = Some(h);
            }
            None => {
                activation_failed.set(true);
                app.quit();
            }
        }
    });
    let status = app.run_with_args::<&str>(&[]);
    if failed.get() {
        gtk::glib::ExitCode::FAILURE
    } else {
        status
    }
}
