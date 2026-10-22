//! Opens the real Claude window, waits for the offline index and (when a credential is stored) the T3
//! server, drives it, checks what is shown and optionally writes PNGs. Needs a display:
//!
//!   rust/claude-pantheon-rs/tests/run-render-sandbox.sh
//!
//! Environment:
//!   CLAUDE_PANTHEON_SHOT=<file.png>      offline transcript view (milestone 1)
//!   CLAUDE_PANTHEON_SHOT_M2=<file.png>   live thread view, needs the server (milestone 2)
//!   CLAUDE_PANTHEON_SHOT_M3=<file.png>   usage page (milestone 3)
//!   CLAUDE_PANTHEON_SHOT_DIAG=<file.png> diagnostics page (milestone 4g)
//!   CLAUDE_PANTHEON_SHOT_REPOS=<file.png> repos and issues (milestone 5), pinned repos from workflow.json
//!   CLAUDE_PANTHEON_LIVE_RENDER=1        opt in to the real home, keyring, T3 server and transcripts
//!   CLAUDE_PANTHEON_LIVE_SEND=1           with LIVE_RENDER, send a real prompt (costs tokens)
//!   CLAUDE_PANTHEON_LIVE_ROTATE=1         with LIVE_RENDER, may move a live runner task
//!   CLAUDE_PANTHEON_TEST_MICROPHONE=1     start microphone capture during the voice probe
//!   CLAUDE_PANTHEON_DARK=1               dark style
//! Plain `main`, not libtest: GTK must stay on one thread. The default run uses a temporary home,
//! one checked-in transcript, and offline mode; live profile/server access needs explicit opt-in.

use claude_pantheon_rs::app::App;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct TestHome(PathBuf);

impl TestHome {
    fn prepare() -> Self {
        let home = std::env::temp_dir().join(format!("claude-pantheon-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let projects = home.join(".claude/projects/render-fixture");
        std::fs::create_dir_all(&projects).expect("create temporary Claude project");
        std::fs::write(
            projects.join("offline-render-fixture.jsonl"),
            include_str!("fixtures/offline-render-session.jsonl"),
        )
        .expect("write offline transcript fixture");
        for name in ["config", "cache", "data", "state", "runtime"] {
            std::fs::create_dir_all(home.join(name)).expect("create isolated XDG directory");
        }
        #[cfg(unix)]
        std::fs::set_permissions(home.join("runtime"), std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("restrict temporary runtime directory");
        // SAFETY: this single-threaded integration harness sets process paths before GTK, GLib, Rayon,
        // or any application worker is initialized. The process exits after the test.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CONFIG_HOME", home.join("config"));
            std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
            std::env::set_var("XDG_DATA_HOME", home.join("data"));
            std::env::set_var("XDG_STATE_HOME", home.join("state"));
            std::env::set_var("XDG_RUNTIME_DIR", home.join("runtime"));
            std::env::set_var("GSETTINGS_BACKEND", "memory");
            std::env::set_var("GTK_USE_PORTAL", "0");
            std::env::set_var("CLAUDE_PANTHEON_OFFLINE", "1");
        }
        Self(home)
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
    if std::env::var("CLAUDE_PANTHEON_RENDER_TEST_SANDBOX").as_deref() != Ok("1") {
        println!("SKIP: run tests/run-render-sandbox.sh for an isolated offline UI render");
        return;
    }
    let live_render = std::env::var("CLAUDE_PANTHEON_LIVE_RENDER").as_deref() == Ok("1");
    let _test_home = if live_render {
        // SAFETY: see TestHome::prepare; this runs before GTK or application workers are initialized.
        unsafe { std::env::remove_var("CLAUDE_PANTHEON_OFFLINE") };
        None
    } else {
        Some(TestHome::prepare())
    };
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
    let live_send = live_render && std::env::var("CLAUDE_PANTHEON_LIVE_SEND").as_deref() == Ok("1");
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
    // GtkSearchEntry delays search-changed by 150 ms.
    assert!(
        pump_until("the search to apply", 3, || shown(&app).iter().any(|x| x == "Nothing matches")),
        "the sidebar placeholder names the situation"
    );
    assert_eq!(app.side_placeholder_name(), "empty", "an empty search result shows the hint, not a blank list");
    app.set_search("");
    pump(100);
    assert!(!has_type(app.window().upcast_ref(), "GtkSpinner"), "loading states are skeletons, not spinners");

    // ---- window state, zoom, HiDPI, a11y ------------------------------------------------------
    {
        let scale = app.window().scale_factor();
        println!("scale factor: {scale}");
        if std::env::var("GDK_SCALE").as_deref() == Ok("2") {
            assert_eq!(scale, 2, "GDK_SCALE=2 gives a 2x window");
        }
        assert!(app.sidebar_width() >= 200);
        app.zoom_to(1.2);
        pump(100);
        assert!((app.text_zoom() - 1.2).abs() < 1e-9);
        let dpi = gtk::Settings::default().unwrap().gtk_xft_dpi();
        assert!(dpi >= 96 * 1024 * 11 / 10, "xft dpi scaled: {dpi}");
        app.zoom_to(1.0);
        pump(100);
        let state = claude_pantheon_rs::winstate::WindowState::capture(
            app.window(),
            &gtk::Paned::new(gtk::Orientation::Horizontal),
        );
        assert!(state.width >= 640 && state.height >= 420);
        // Icon-only buttons: each one needs a tooltip (shown) and a label (read); the ones without a
        // tooltip are listed so a new one stands out in the log.
        fn icon_buttons_without_tooltip(w: &gtk::Widget, out: &mut Vec<String>) {
            if let Some(b) = w.downcast_ref::<gtk::Button>()
                && b.label().is_none()
                && b.child().is_some_and(|c| c.is::<gtk::Image>())
                && b.tooltip_text().is_none()
            {
                out.push(b.icon_name().map(|n| n.to_string()).unwrap_or_default());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                icon_buttons_without_tooltip(&c, out);
                child = c.next_sibling();
            }
        }
        let mut bare = Vec::new();
        icon_buttons_without_tooltip(app.window().upcast_ref(), &mut bare);
        println!("icon buttons without a tooltip: {bare:?}");
        use claude_pantheon_rs::t3::model::TurnItem;
        let item: TurnItem = serde_json::from_value(
            serde_json::json!({"id": "a", "type": "assistant_message", "text": "Hello  there\nfriend"}),
        )
        .unwrap();
        assert_eq!(claude_pantheon_rs::chat::accessible_name(&item), "Assistant: Hello there friend");
        println!("window state, zoom and a11y ok");
    }

    // ---- voice input: never sends; a transcript lands in the draft it was recorded for ----------
    {
        let avail = claude_pantheon_rs::voice::Availability::probe(None);
        println!("voice: recorder {:?}, whisper {:?}, model {:?}", avail.recorder, avail.whisper, avail.model);
        app.composer().set_text("note:");
        app.insert_transcript("hello from the mic", app.composer().draft_key());
        assert_eq!(app.composer().text(), "note: hello from the mic");
        assert!(!app.is_recording());
        app.insert_transcript("late words", Some("another-thread".into()));
        assert_eq!(app.composer().text(), "note: hello from the mic", "a transcript for another draft is dropped");
        app.composer().set_text("");
        if std::env::var("CLAUDE_PANTHEON_TEST_MICROPHONE").as_deref() == Ok("1") {
            app.toggle_voice();
            pump(1200);
            if app.is_recording() {
                app.toggle_voice();
                pump(1500);
            }
            assert!(!app.is_recording());
        }
        println!("voice ok");
    }

    // ---- a transient error toast carries Retry ------------------------------------------------
    {
        let hit = std::rc::Rc::new(std::cell::Cell::new(false));
        let h = hit.clone();
        app.toast_retry("probe failed", std::rc::Rc::new(move |_| h.set(true)));
        app.toast_widget().emit_by_name::<()>("default-action", &[]);
        pump(50);
        assert!(hit.get(), "the toast's Retry runs the retry closure");
        app.toast("plain");
        app.toast_widget().emit_by_name::<()>("default-action", &[]);
        pump(50);
        println!("retry toast ok");
    }
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

    // ---- visual reply card (no WebKit view until the owner asks) ------------------------------------
    {
        use claude_pantheon_rs::t3::model::TurnItem;
        let item: TurnItem = serde_json::from_value(serde_json::json!({
            "id": "item-2", "type": "tool_call", "status": "completed", "toolName": "mcp__t3__html_render",
            "output": "{\"htmlRender\": {\"attachmentId\": \"att-does-not-exist\", \"title\": \"Burn-down chart\", \"height\": 320}}"
        }))
        .unwrap();
        let widget = claude_pantheon_rs::chat::item_widget(&item, None).expect("a page card");
        let mut t = Vec::new();
        texts(&widget, &mut t);
        for expect in ["Burn-down chart", "Show page"] {
            assert!(t.iter().any(|x| x.contains(expect)), "page card shows '{expect}': {t:?}");
        }
        assert!(!has_type(&widget, "WebKitWebView"), "no web view before Show page");
        println!("page card ok");
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

        // ---- composer: draft slots, attachment chips -------------------------------------------
        // Use private keys so a live-render pass never overwrites or clears an owner's draft.
        let first = format!("render-test-{}-first", std::process::id());
        let other = format!("render-test-{}-other", std::process::id());
        let composer = app.composer();
        composer.set_draft_key(Some(first.clone()));
        app.set_entry("half-written thought");
        pump(1300); // the draft saves a second after the last change
        composer.set_draft_key(Some(other));
        assert_eq!(composer.text(), "", "a fresh draft slot starts empty");
        composer.set_draft_key(Some(first.clone()));
        assert_eq!(composer.text(), "half-written thought", "the saved draft comes back with its key");
        assert_eq!(composer.draft_for(&first).as_deref(), Some("half-written thought"));
        app.set_entry("");
        pump(1300);
        composer.set_draft_key(Some(pick.id.clone()));
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
        if live_render && std::env::var("CLAUDE_PANTHEON_LIVE_ROTATE").as_deref() == Ok("1") {
            app.rotate_runners_now();
            pump(1500);
            println!("rotation pass ok");
        }
    } else {
        println!("not connected to a T3 server; live checks skipped");
    }
    // ---- environments switcher: an unreachable environment, then back to this computer ----------
    {
        use claude_pantheon_rs::environments::{Environment, Route, State};
        let was_connected = app.is_connected();
        assert_eq!(app.current_environment_id().as_deref(), Some("local"));
        app.save_environment(
            Environment {
                id: "probe".into(),
                name: "Nowhere".into(),
                route: Route::Direct { url: "http://127.0.0.1:1".into() },
            },
            true,
        );
        assert!(pump_until("the probe environment to settle", 30, || matches!(
            app.environment_state("probe"),
            State::Offline | State::Idle
        ) && app.page() == "pair"));
        assert_eq!(app.current_environment_id().as_deref(), Some("probe"));
        assert!(shown(&app).iter().any(|x| x.contains("Nowhere")), "the switcher shows the current environment");
        assert_eq!(app.current_server(), "http://127.0.0.1:1");
        app.switch_environment("local");
        if was_connected {
            assert!(pump_until("reconnect to this computer", 60, || app.is_connected()));
        }
        app.remove_environment("probe");
        assert!(!app.environments().iter().any(|e| e.id == "probe"));
        assert_eq!(app.current_environment_id().as_deref(), Some("local"));
        println!("environments ok");
    }
    app.window().destroy();
    println!("PASS: Claude render test");
}
