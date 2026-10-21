//! `mseries-diskspace`: a disk space analyzer. Run it from the launcher, or:
//!
//!   mseries-diskspace [FOLDER]        open the window on FOLDER (default: your home folder)
//!   mseries-diskspace --scan FOLDER   no window: print the biggest items, for checking against `du`

use gtk::prelude::*;
use mseries_diskspace::{app, scan};
use std::path::PathBuf;

fn main() -> gtk::glib::ExitCode {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--scan") {
        return scan_report(args.next().map(PathBuf::from));
    }
    let start = first
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"));

    let application = gtk::Application::builder().application_id("org.techmore.mseries.DiskSpace").build();
    application.connect_startup(|_| granite::init());
    application.connect_activate(move |application| {
        if let Some(existing) = application.active_window() {
            existing.present();
            return;
        }
        app::App::build(application, start.clone());
    });
    // Our own argument is the folder, so don't hand it to GApplication as a file to open.
    application.run_with_args::<&str>(&[])
}

fn scan_report(path: Option<PathBuf>) -> gtk::glib::ExitCode {
    let Some(path) = path else {
        eprintln!("usage: mseries-diskspace --scan FOLDER");
        return gtk::glib::ExitCode::FAILURE;
    };
    let progress = scan::Progress::default();
    let started = std::time::Instant::now();
    let Some(tree) = scan::scan_blocking(&path, false, &progress) else {
        eprintln!("cannot scan {}", path.display());
        return gtk::glib::ExitCode::FAILURE;
    };
    let root = tree.node(tree.root);
    println!(
        "{}: {} bytes on disk, {} bytes apparent, {} items, {} unreadable folders, {:.2}s",
        path.display(),
        root.size,
        root.apparent,
        root.items,
        tree.errors,
        started.elapsed().as_secs_f64()
    );
    for &child in root.children.iter().take(15) {
        let n = tree.node(child);
        println!("{:>16}  {}", n.size, n.name);
    }
    gtk::glib::ExitCode::SUCCESS
}
