//! `claude-pantheon-rs`: Claude for elementary OS.
//!
//!   claude-pantheon-rs                      open the window
//!   claude-pantheon-rs --quit-after-frame   draw one frame, print the start-up time and exit (benchmarks)
//!   claude-pantheon-rs --index              no window: list the sessions the sidebar would show

use claude_pantheon_rs::{APP_ID, app, index};
use gtk::prelude::*;
use std::time::Instant;

fn main() -> gtk::glib::ExitCode {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--index") {
        let sessions = index::index_all();
        for s in &sessions {
            println!("{}  {:<28} {}", s.id, s.project(), s.title);
        }
        println!("{} sessions in {:.3}s", sessions.len(), started.elapsed().as_secs_f64());
        return gtk::glib::ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) == Some("--t3-dump") {
        return claude_pantheon_rs::probe::dump(args.get(1).cloned(), args.get(2).cloned());
    }
    if args.first().map(String::as_str) == Some("--t3-call") {
        return claude_pantheon_rs::probe::call(args.get(1).cloned().unwrap_or_default(), args.get(2).cloned());
    }
    let quit_after_frame = args.iter().any(|a| a == "--quit-after-frame");

    // Mesa on Asahi loads LLVM and ~180 MB of shader heap for the GL renderers; cairo draws this window in
    // a third of the memory (see docs/PERFORMANCE.md). CLAUDE_PANTHEON_RENDERER=gl|ngl|vulkan switches back.
    if std::env::var_os("GSK_RENDERER").is_none() {
        let renderer = std::env::var("CLAUDE_PANTHEON_RENDERER").unwrap_or_else(|_| "cairo".into());
        // SAFETY: single-threaded here; GTK has not been initialised and no other thread reads the environment.
        unsafe { std::env::set_var("GSK_RENDERER", renderer) };
    }
    let application = gtk::Application::builder().application_id(APP_ID).build();
    application.connect_startup(|_| {
        granite::init();
        // GTK fades the text cursor with a frame-clock tick for ten seconds after every keystroke; with the
        // composer focused that kept the window repainting. A steady cursor costs nothing.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_cursor_blink(false);
        }
    });
    application.connect_activate(move |application| {
        if let Some(existing) = application.active_window() {
            existing.present();
            return;
        }
        app::App::build(application, started, quit_after_frame);
    });
    application.run_with_args::<&str>(&[])
}
