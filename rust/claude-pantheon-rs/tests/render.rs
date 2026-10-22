//! Opens the real Claude window, waits for the offline index and (when a credential is stored) the T3
//! server, drives it, checks what is shown and optionally writes PNGs. Needs a display:
//!
//!   dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
//!     xvfb-run -a -s "-screen 0 1400x900x24" cargo test --release --test render
//!
//! Environment:
//!   CLAUDE_PANTHEON_SHOT=<file.png>      offline transcript view (milestone 1)
//!   CLAUDE_PANTHEON_SHOT_M2=<file.png>   live thread view, needs the server (milestone 2)
//!   CLAUDE_PANTHEON_SHOT_M3=<file.png>   usage page (milestone 3)
//!   CLAUDE_PANTHEON_SHOT_DIAG=<file.png> diagnostics page (milestone 4g)
//!   CLAUDE_PANTHEON_SHOT_REPOS=<file.png> repos and issues (milestone 5), pinned repos from workflow.json
//!   CLAUDE_PANTHEON_LIVE_SEND=1          start a real thread and wait for the answer (costs tokens)
//!   CLAUDE_PANTHEON_DARK=1               dark style
//! Plain `main`, not libtest: GTK must stay on one thread.

use claude_pantheon_rs::app::App;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn pump(ms: u64) {
    let ctx = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        while ctx.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pump_until(what: &str, secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let limit = Instant::now() + Duration::from_secs(secs);
    while !done() {
        if Instant::now() > limit {
            eprintln!("timed out waiting for {what}");
            return false;
        }
        pump(20);
    }
    true
}

fn texts(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        texts(&c, out);
        child = c.next_sibling();
    }
}

fn shown(app: &App) -> Vec<String> {
    let mut out = Vec::new();
    texts(app.window().upcast_ref(), &mut out);
    out
}

fn save_png(window: &gtk::ApplicationWindow, path: &Path) {
    let mut node = None;
    for _ in 0..100 {
        pump(50);
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
        node = snapshot.to_node();
        if node.is_some() {
            break;
        }
    }
    let node = node.expect("the window never painted");
    let renderer = window.native().and_then(|n| n.renderer()).expect("renderer");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    renderer.render_texture(&node, None).save_to_png(path).expect("write png");
    println!("wrote {}", path.display());
}

fn main() {
    if gtk::init().is_err() {
        eprintln!("FAIL: no display for GTK (run under xvfb-run)");
        std::process::exit(1);
    }
    granite::init();
    let application =
        gtk::Application::builder().application_id("io.github.techmore.ClaudePantheonRs.RenderTest").build();
    application.register(gio::Cancellable::NONE).expect("register");
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let shot = env("CLAUDE_PANTHEON_SHOT");
    let shot_m2 = env("CLAUDE_PANTHEON_SHOT_M2");
    let shot_m3 = env("CLAUDE_PANTHEON_SHOT_M3");
    let live_send = std::env::var_os("CLAUDE_PANTHEON_LIVE_SEND").is_some();
    #[allow(deprecated)]
    gtk::Settings::default()
        .expect("settings")
        .set_gtk_application_prefer_dark_theme(env("CLAUDE_PANTHEON_DARK").is_some());

    let app = App::build(&application, Instant::now(), false);
    assert!(pump_until("the index and the connection attempt", 60, || !app.is_busy()));
    pump(200);
    let sessions = app.sessions();
    assert!(!sessions.is_empty(), "no sessions were indexed");
    println!("indexed {} sessions; connected: {}", sessions.len(), app.is_connected());

    // ---- milestone 1: offline transcript ------------------------------------------------------
    let newest_project = sessions[0].project();
    let pick = sessions
        .iter()
        .filter(|s| s.project() == newest_project)
        .max_by_key(|s| s.size)
        .unwrap_or(&sessions[0])
        .clone();
    assert!(app.select_id(&pick.id), "the row for {} exists", pick.title);
    assert!(pump_until("the transcript", 60, || !app.is_busy()));
    pump(300);
    let t = shown(&app);
    assert!(t.iter().any(|x| x == &pick.title), "the title shows: {:?}", &t[..t.len().min(20)]);
    assert!(
        t.iter().any(|x| x.starts_with("Ran ") || x.starts_with("Edited ") || x.starts_with("Read ")),
        "tool rows show"
    );
    println!("opened offline '{}' ({} bytes)", pick.title, pick.size);
    app.set_search("zzzz-nothing-matches");
    pump(100);
    app.set_search("");
    pump(100);
    if let Some(path) = &shot {
        save_png(app.window(), path);
    }

    // ---- milestone 2: live threads ------------------------------------------------------------
    if app.is_connected() {
        assert!(pump_until("the thread list", 30, || !app.threads().is_empty()), "the server sent threads");
        pump(300);
        let providers = app.providers();
        println!(
            "providers: {:?}",
            providers.iter().map(|p| format!("{} {}", p.name(), p.ready())).collect::<Vec<_>>()
        );
        assert!(providers.len() >= 2, "at least two providers are listed");
        let threads = app.threads();
        let pick =
            threads.iter().find(|t| t.id.starts_with("import:claudeAgent:")).or(threads.first()).unwrap().clone();
        assert!(app.select_id(&pick.id), "thread row exists");
        assert!(pump_until("the thread snapshot", 60, || app.live_item_count() > 0));
        pump(400);
        let t = shown(&app);
        assert!(
            t.iter().any(|x| x.contains("Claude ·") || x.contains("Codex ·")),
            "provider picker shows: {:?}",
            &t[t.len().saturating_sub(12)..]
        );
        println!("opened thread '{}' with {} items", pick.title, app.live_item_count());
        if live_send {
            app.new_chat();
            pump(200);
            app.set_entry("Reply with exactly the single word: pong");
            app.send();
            assert!(pump_until("the live answer", 180, || shown(&app)
                .iter()
                .any(|x| x.to_lowercase().contains("pong") && !x.contains("exactly"))));
            pump(500);
            println!("live answer arrived");
        }
        if let Some(path) = &shot_m2 {
            save_png(app.window(), path);
        }
        // ---- milestone 3: usage ----------------------------------------------------------------
        app.show_usage();
        assert!(pump_until("usage", 90, || app.usage_page().summary().is_some()));
        pump(400);
        let t = shown(&app);
        assert!(t.iter().any(|x| x.contains("API-equivalent cost")), "usage totals show");
        if let Some(path) = &shot_m3 {
            save_png(app.window(), path);
        }
        // ---- milestone 4g: diagnostics, 5a/5b: repos and issues -----------------------------------
        app.show_diagnostics();
        assert!(pump_until("telemetry", 60, || app.diagnostics_page().samples() >= 2));
        pump(300);
        let t = shown(&app);
        assert!(t.iter().any(|x| x.contains("Server") || x.contains("All of T3")), "telemetry groups show");
        if let Some(path) = env("CLAUDE_PANTHEON_SHOT_DIAG") {
            save_png(app.window(), &path);
        }
        app.show_repos();
        assert!(pump_until("repos", 90, || shown(&app)
            .iter()
            .any(|x| x.contains("GitHub repos known") || x.contains("No GitHub checkouts"))));
        pump(500);
        let pinned = app.repos_page().config().repos;
        if let Some(first) = pinned.first().cloned() {
            app.repos_page().show_issues(&app, first.clone());
            assert!(pump_until("issues", 90, || app.repos_page().issue_count() > 0
                || shown(&app).iter().any(|x| x.contains("No issues match") || x.starts_with("GitHub:"))));
            pump(400);
            println!("issues for {}: {}", first.repo, app.repos_page().issue_count());
        }
        if let Some(path) = env("CLAUDE_PANTHEON_SHOT_REPOS") {
            save_png(app.window(), &path);
        }
    } else {
        println!("not connected to a T3 server; live checks skipped");
    }
    app.window().destroy();
    println!("PASS: Claude render test");
}
