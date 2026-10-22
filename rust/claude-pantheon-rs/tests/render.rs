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
    // Fenced code blocks render in GtkSourceView (the transcript has plenty).
    fn has_type(widget: &gtk::Widget, name: &str) -> bool {
        if widget.type_().name() == name {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if has_type(&c, name) {
                return true;
            }
            child = c.next_sibling();
        }
        false
    }
    assert!(has_type(app.window().upcast_ref(), "GtkSourceView"), "a code block became a GtkSourceView");
    app.set_search("zzzz-nothing-matches");
    pump(100);
    app.set_search("");
    pump(100);
    if let Some(path) = &shot {
        save_png(app.window(), path);
    }

    // ---- permission card (synthetic pending approval) -----------------------------------------------
    {
        use claude_pantheon_rs::t3::model::TurnItem;
        let item: TurnItem = serde_json::from_value(serde_json::json!({
            "id": "item-1", "type": "approval_request", "status": "waiting", "requestId": "req-1",
            "requestKind": "command", "prompt": "rm -rf build/\nand then rebuild", "reason": "The agent wants to run a command"
        })).unwrap();
        let answered = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = answered.clone();
        let widget = claude_pantheon_rs::chat::item_widget(
            &item,
            Some(std::rc::Rc::new(move |id: &str, decision: &str| {
                seen.borrow_mut().push(format!("{id}:{decision}"));
            })),
        )
        .expect("a card");
        let mut t = Vec::new();
        texts(&widget, &mut t);
        for expect in ["Run a command?", "rm -rf build/", "Allow Once", "Allow for This Thread", "Deny"] {
            assert!(t.iter().any(|x| x.contains(expect)), "card shows '{expect}': {t:?}");
        }
        fn first_button(w: &gtk::Widget, label_text: &str) -> Option<gtk::Button> {
            if let Some(b) = w.downcast_ref::<gtk::Button>()
                && b.label().is_some_and(|l| l == label_text)
            {
                return Some(b.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                if let Some(b) = first_button(&c, label_text) {
                    return Some(b);
                }
                child = c.next_sibling();
            }
            None
        }
        first_button(&widget, "Allow for This Thread").expect("button").emit_clicked();
        assert_eq!(answered.borrow().as_slice(), ["req-1:acceptForSession"]);
        println!("permission card ok");
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
        // ---- sidebar: collapsible sections, context-menu actions -----------------------------------------
        let (rows_before, headers_before) = app.row_count();
        app.set_section_collapsed("offline", true);
        pump(100);
        let (rows_after, headers_after) = app.row_count();
        assert!(
            rows_after < rows_before,
            "collapsing the offline section hides its rows ({rows_before} -> {rows_after})"
        );
        assert!(headers_after < headers_before, "collapsed sections hide their sub-headers");
        app.set_section_collapsed("offline", false);
        pump(100);
        assert_eq!(app.row_count().0, rows_before, "expanding brings the rows back");
        for action in ["thread-pin", "thread-rename", "thread-handoff", "thread-copy-link", "thread-archive"] {
            assert!(app.window().lookup_action(action).is_some(), "window action {action} exists");
        }
        println!("sidebar ok");

        // ---- composer: drafts per thread, attachment chips ---------------------------------------------
        let first = pick.id.clone();
        app.set_entry("half-written thought");
        pump(1300); // the draft saves a second after the last change
        let other = threads.iter().find(|t| t.id != first).cloned().expect("a second thread");
        assert!(app.select_id(&other.id));
        assert!(pump_until("the other thread", 60, || app.live_item_count() > 0 || app.page() == "chat"));
        pump(300);
        assert_eq!(app.composer().text(), "", "the other thread starts with an empty draft");
        assert!(app.select_id(&first));
        assert!(pump_until("the first thread again", 60, || app.live_item_count() > 0));
        pump(300);
        assert_eq!(app.composer().text(), "half-written thought", "the draft came back with its thread");
        assert_eq!(app.composer().draft_for(&first).as_deref(), Some("half-written thought"));
        app.set_entry("");
        pump(1300);
        println!("composer drafts ok");

        // ---- terminal pane (the server's PTY) --------------------------------------------------------
        app.toggle_terminal();
        assert!(
            pump_until("the terminal to attach", 60, || shown(&app)
                .iter()
                .any(|x| x.contains("pid ") || x.contains("running"))),
            "terminal status: {:?}",
            shown(&app).iter().filter(|x| x.contains("~/") || x.contains("Terminal")).collect::<Vec<_>>()
        );
        assert!(app.terminal_pane().is_attached());
        pump(300);
        app.toggle_terminal();
        pump(200);
        assert!(!app.terminal_pane().is_attached(), "hiding the pane detaches");
        println!("terminal ok");

        // ---- changes page ------------------------------------------------------------------------------
        app.show_changes();
        assert!(pump_until("the diff", 60, || shown(&app)
            .iter()
            .any(|x| x.contains(" files · +") || x.contains("No file changes") || x.starts_with("Diff failed"))));
        pump(200);
        println!("changes page: {} files", app.changes_page().file_count());
        app.select_id(&pick.id);
        assert!(pump_until("the thread again", 60, || app.live_item_count() > 0));
        pump(200);

        // ---- settings window -----------------------------------------------------------------------
        WidgetExt::activate_action(app.window(), "win.settings", None).expect("settings action");
        pump(400);
        let settings_window = gtk::Window::list_toplevels()
            .into_iter()
            .find(|w| w.downcast_ref::<gtk::Window>().is_some_and(|w| w.title().is_some_and(|t| t == "Settings")));
        let settings_window = settings_window.expect("a Settings window").downcast::<gtk::Window>().unwrap();
        let mut t = Vec::new();
        texts(settings_window.upcast_ref(), &mut t);
        for expect in ["Default permission mode", "Pairing", "Renderer", "Turn finished"] {
            assert!(t.iter().any(|x| x.contains(expect)), "settings shows '{expect}'");
        }
        settings_window.close();
        pump(200);
        println!("settings ok");

        // ---- command palette (Ctrl+K) --------------------------------------------------------------
        let palette = app.palette();
        pump(200);
        palette.set_query("open usage");
        pump(100);
        let titles = palette.shown_titles();
        assert_eq!(titles.first().map(String::as_str), Some("Open Usage"), "palette ranks the page first: {titles:?}");
        palette.run(0);
        pump(300);
        assert_eq!(app.page(), "usage", "the palette opened the Usage page");
        let palette = app.palette();
        palette.set_query("hdmi");
        pump(100);
        assert!(palette.shown_titles().iter().any(|t| t.contains("HDMI")), "threads are in the palette");
        palette.window().close();
        pump(100);
        println!("palette ok");

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
