//! `claude-pantheon-rs`: Claude for elementary OS.
//!
//!   claude-pantheon-rs                      open the window
//!   claude-pantheon-rs --new-thread         open (or raise) the window on a new thread; also the .desktop action
//!   claude-pantheon-rs --usage              open on the Usage page
//!   claude-pantheon-rs --thread ID          open (or raise) the window on that thread
//!   claude-pantheon-rs --quit-after-frame   draw one frame, print the start-up time and exit (benchmarks)
//!   claude-pantheon-rs --index              no window: list the sessions the sidebar would show

use claude_pantheon_rs::{APP_ID, app, index};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
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
    if args.first().map(String::as_str) == Some("--t3-attach") {
        return claude_pantheon_rs::probe::attach(args.get(1).cloned().unwrap_or_default());
    }
    if args.first().map(String::as_str) == Some("--t3-call") {
        return claude_pantheon_rs::probe::call(args.get(1).cloned().unwrap_or_default(), args.get(2).cloned());
    }
    let quit_after_frame = args.iter().any(|a| a == "--quit-after-frame");
    // Desktop actions and a second launch reach the running instance through GApplication's command line.
    let mut forwarded: Vec<String> = args.iter().filter(|a| *a == "--new-thread" || *a == "--usage").cloned().collect();
    if let Some(i) = args.iter().position(|a| a == "--thread")
        && let Some(id) = args.get(i + 1)
    {
        forwarded.push("--thread".into());
        forwarded.push(id.clone());
    }

    // Mesa on Asahi loads LLVM and ~180 MB of shader heap for the GL renderers; cairo draws this window in
    // a third of the memory (see docs/PERFORMANCE.md). CLAUDE_PANTHEON_RENDERER=gl|ngl|vulkan switches back.
    if std::env::var_os("GSK_RENDERER").is_none() {
        let renderer = std::env::var("CLAUDE_PANTHEON_RENDERER")
            .unwrap_or_else(|_| claude_pantheon_rs::settings::Settings::load().renderer);
        // SAFETY: single-threaded here; GTK has not been initialised and no other thread reads the environment.
        unsafe { std::env::set_var("GSK_RENDERER", renderer) };
    }
    let application = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    application.connect_startup(|_| {
        granite::init();
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
        // GTK fades the text cursor with a frame-clock tick for ten seconds after every keystroke; with the
        // composer focused that kept the window repainting. A steady cursor costs nothing.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_cursor_blink(false);
        }
    });
    let current: Rc<RefCell<Option<Rc<app::App>>>> = Rc::new(RefCell::new(None));
    let slot = current.clone();
    application.connect_activate(move |application| {
        if let Some(existing) = application.active_window() {
            existing.present();
            return;
        }
        *slot.borrow_mut() = Some(app::App::build(application, started, quit_after_frame));
    });
    // app.new-thread and app.usage: the .desktop actions, also reachable with `gapplication action`.
    for (name, run) in [
        ("new-thread", Box::new(|a: &Rc<app::App>| a.new_chat()) as Box<dyn Fn(&Rc<app::App>)>),
        ("usage", Box::new(|a: &Rc<app::App>| a.show_usage())),
    ] {
        let action = gtk::gio::SimpleAction::new(name, None);
        let slot = current.clone();
        action.connect_activate(move |_, _| {
            if let Some(a) = slot.borrow().as_ref() {
                run(a);
            }
        });
        application.add_action(&action);
    }
    // app.open-thread '<id>': the Wingpanel indicator opens a thread (or an offline session) by id.
    let open = gtk::gio::SimpleAction::new("open-thread", Some(gtk::glib::VariantTy::STRING));
    let slot = current.clone();
    open.connect_activate(move |_, param| {
        if let Some(id) = param.and_then(|p| p.get::<String>())
            && let Some(a) = slot.borrow().as_ref()
        {
            a.open_by_id(&id);
        }
    });
    application.add_action(&open);
    application.connect_command_line(move |application, cmdline| {
        application.activate();
        let args = cmdline.arguments();
        for (i, arg) in args.iter().enumerate() {
            match arg.to_str() {
                Some("--new-thread") => application.activate_action("new-thread", None),
                Some("--usage") => application.activate_action("usage", None),
                Some("--thread") => {
                    if let Some(id) = args.get(i + 1).and_then(|s| s.to_str()) {
                        application.activate_action("open-thread", Some(&id.to_variant()));
                    }
                }
                _ => {}
            }
        }
        gtk::glib::ExitCode::SUCCESS
    });
    let argv: Vec<String> = std::iter::once(std::env::args().next().unwrap_or_default()).chain(forwarded).collect();
    application.run_with_args(&argv)
}
