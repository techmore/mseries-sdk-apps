//! Renders the Software Updates window for every status the updater can report, drives its buttons
//! against a fake updater, and optionally writes PNGs of each state. Needs a display:
//!
//!   dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
//!     xvfb-run -a cargo test -p mseries-updates --test render
//!
//! Set MSERIES_UPDATES_SHOTS=<dir> to keep the screenshots (light and dark). Plain `main`, not libtest:
//! GTK must stay on one thread.

use gtk::prelude::*;
use gtk::{gio, glib};
use mseries_update_core::command::{Channel, Command};
use mseries_update_core::model::Status;
use mseries_updates::backend::{Backend, Failure};
use mseries_updates::ui::{Confirmation, UpdatesWindow};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

// 2026-10-21T20:07:11Z plus five minutes.
const NOW: i64 = 1_792_613_531;

const NEVER_CHECKED: &str = r#"{"schema":1,"channel":"stable","auto_stage":false,"state":"idle","staged":null,
 "pending_confirmation":null,"last_applied":null,"last_check":null,"skipped":{},"last_result":null,
 "power":{"source":"ac","percent":null}}"#;
const UP_TO_DATE: &str = r#"{"schema":1,"channel":"stable","state":"idle","last_check":{"time":"2026-10-21T20:07:11Z","channel":"stable","updates":{}},
 "last_applied":{"id":"a","from":{"mseries-update":"0.1.0-0mseries2"},"to":{"mseries-update":"0.1.0-0mseries3"},"confirmed":"2026-10-20T10:00:00Z","rolled_back":null},
 "last_result":{"time":"2026-10-20T10:00:00Z","outcome":"confirmed","detail":""}}"#;
const AVAILABLE: &str = r#"{"schema":1,"channel":"stable","state":"idle","last_check":{"time":"2026-10-21T20:07:11Z","channel":"stable","updates":{
 "mseries-quick-settings":{"from":"1.4.0-0mseries3","to":"1.4.0-0mseries4"},
 "mseries-wingpanel-network":{"from":"8.0.1-0mseries3","to":"8.0.1-0mseries4"}}}}"#;
const STAGED: &str = r#"{"schema":1,"channel":"stable","state":"staged",
 "staged":{"id":"s","created":"2026-10-21T20:08:00Z","channel":"stable","target":{"mseries-quick-settings":"1.4.0-0mseries4"},"from":{"mseries-quick-settings":"1.4.0-0mseries3"}},
 "last_check":{"time":"2026-10-21T20:07:11Z","updates":{"mseries-quick-settings":{"from":"1.4.0-0mseries3","to":"1.4.0-0mseries4"}}}}"#;
const AWAITING: &str = r#"{"schema":1,"channel":"dev","state":"awaiting-confirmation","pending_confirmation":{"id":"p","unconfirmed_boots":0},
 "last_applied":{"id":"p","from":{"mseries-quick-settings":"1.4.0-0mseries3"},"to":{"mseries-quick-settings":"1.4.0-0mseries3~dev20261021200711.abc1234"},"confirmed":null,"rolled_back":null}}"#;
const RECOVERING: &str = r#"{"schema":1,"channel":"stable","state":"recovering",
 "last_applied":{"id":"p","from":{"a":"1"},"to":{"a":"2"},"confirmed":null,"rolled_back":null}}"#;
const ROLLED_BACK_BY_UPDATER: &str = r#"{"schema":1,"channel":"stable","state":"idle","skipped":{"mseries-quick-settings":["1.4.0-0mseries4"]},
 "last_check":{"time":"2026-10-21T20:07:11Z","updates":{"mseries-quick-settings":{"from":"1.4.0-0mseries3","to":"1.4.0-0mseries4"}}},
 "last_applied":{"id":"p","from":{"mseries-quick-settings":"1.4.0-0mseries3"},"to":{"mseries-quick-settings":"1.4.0-0mseries4"},"rolled_back":"2026-10-21T20:00:00Z"},
 "last_result":{"time":"2026-10-21T20:00:00Z","outcome":"rolled-back","detail":"no healthy login after 2 restarts"}}"#;
const ROLLBACK_FAILED: &str = r#"{"schema":1,"channel":"stable","state":"idle",
 "last_result":{"time":"2026-10-21T20:00:00Z","outcome":"rollback-failed","detail":"stored copy of mseries-update is missing"}}"#;

/// Stand-in for the updater. `after` may change the status when a command succeeds, the way the
/// real updater's state moves; `gate` holds a command open so the busy state can be observed.
struct Fake {
    status: Mutex<Result<String, Failure>>,
    commands: Mutex<Vec<Command>>,
    result: Mutex<Result<(), Failure>>,
    after: Mutex<Option<String>>,
    gate: (Mutex<bool>, Condvar),
}

impl Fake {
    fn new(status: &str) -> Arc<Fake> {
        Arc::new(Fake {
            status: Mutex::new(Ok(status.to_string())),
            commands: Mutex::new(vec![]),
            result: Mutex::new(Ok(())),
            after: Mutex::new(None),
            gate: (Mutex::new(true), Condvar::new()),
        })
    }
    fn commands(&self) -> Vec<Command> {
        self.commands.lock().unwrap().clone()
    }
    fn hold(&self) {
        *self.gate.0.lock().unwrap() = false;
    }
    fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}

impl Backend for Fake {
    fn status(&self) -> Result<Status, Failure> {
        match &*self.status.lock().unwrap() {
            Ok(json) => Status::parse(json).map_err(|e| Failure::Failed(e.to_string())),
            Err(failure) => Err(failure.clone()),
        }
    }
    fn run(&self, command: Command) -> Result<(), Failure> {
        let (lock, cv) = &self.gate;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = cv.wait(open).unwrap();
        }
        self.commands.lock().unwrap().push(command);
        let result = self.result.lock().unwrap().clone();
        if result.is_ok() {
            if let Some(next) = self.after.lock().unwrap().take() {
                *self.status.lock().unwrap() = Ok(next);
            }
        }
        result
    }
}

fn pump_until(what: &str, mut done: impl FnMut() -> bool) {
    let ctx = glib::MainContext::default();
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for: {what}");
        while ctx.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    while ctx.iteration(false) {}
}

fn walk(widget: &gtk::Widget, visit: &mut dyn FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(c) = child {
        walk(&c, visit);
        child = c.next_sibling();
    }
}

/// Text of every label and button that is on screen right now.
fn shown_text(window: &UpdatesWindow) -> Vec<String> {
    let mut out = vec![];
    walk(window.window().upcast_ref(), &mut |w| {
        if !w.is_mapped() {
            return;
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>() {
            // Button labels are reported with their button below.
            if w.parent().is_some_and(|p| p.is::<gtk::Button>()) {
                return;
            }
            out.push(l.label().to_string());
        } else if let Some(b) = w.downcast_ref::<gtk::Button>() {
            if let Some(text) = b.label() {
                out.push(format!("[{text}]"));
            }
        }
    });
    out
}

fn has(window: &UpdatesWindow, needle: &str) -> bool {
    shown_text(window).iter().any(|t| t.contains(needle))
}

fn button(window: &UpdatesWindow, label: &str) -> gtk::Button {
    let mut found = None;
    walk(window.window().upcast_ref(), &mut |w| {
        if let Some(b) = w.downcast_ref::<gtk::Button>() {
            // A plain label, or a button built from child widgets (Granite's placeholder actions).
            let mut inner = b.label().map(|l| l.to_string()).into_iter().collect::<Vec<_>>();
            walk(b.upcast_ref(), &mut |c| {
                if let Some(l) = c.downcast_ref::<gtk::Label>() {
                    inner.push(l.label().to_string());
                }
            });
            if inner.iter().any(|l| l == label) {
                found = Some(b.clone());
            }
        }
    });
    found.unwrap_or_else(|| panic!("no button '{label}'"))
}

fn expand_advanced(window: &UpdatesWindow) {
    walk(window.window().upcast_ref(), &mut |w| {
        if let Some(e) = w.downcast_ref::<gtk::Expander>() {
            e.set_expanded(true);
        }
    });
    pump_until("advanced expanded", || has(window, "Update channel"));
}

/// The channel selector lives under "Advanced", which is collapsed until the user opens it.
fn dropdown(window: &UpdatesWindow) -> gtk::DropDown {
    expand_advanced(window);
    let mut found = None;
    walk(window.window().upcast_ref(), &mut |w| {
        if let Some(d) = w.downcast_ref::<gtk::DropDown>() {
            found = Some(d.clone());
        }
    });
    found.expect("channel dropdown")
}

struct Case {
    app: gtk::Application,
    fake: Arc<Fake>,
    win: UpdatesWindow,
    asked: Rc<RefCell<Vec<Confirmation>>>,
    answer: Rc<RefCell<bool>>,
}

fn open(app: &gtk::Application, status: &str) -> Case {
    let fake = Fake::new(status);
    let win = UpdatesWindow::new(app, fake.clone());
    win.set_clock(|| NOW);
    let asked = Rc::new(RefCell::new(vec![]));
    let answer = Rc::new(RefCell::new(true));
    {
        let (asked, answer) = (asked.clone(), answer.clone());
        win.set_confirmer(Box::new(move |_, q| {
            asked.borrow_mut().push(q);
            let yes = *answer.borrow();
            Box::pin(async move { yes })
        }));
    }
    win.window().present();
    win.refresh();
    pump_until("window mapped and status shown", || win.window().is_mapped() && shown_text(&win).len() > 2);
    Case { app: app.clone(), fake, win, asked, answer }
}

impl Case {
    fn settle(&self) {
        pump_until("idle", || button(&self.win, "Check for Updates").is_sensitive() || has(&self.win, "[Check Again]"));
    }
    fn close(self) {
        self.win.window().destroy();
        let _ = self.app;
    }
}

fn save_png(win: &UpdatesWindow, path: &std::path::Path) {
    let window = win.window();
    // The first frame may not be painted yet: let the main loop run until the window has content.
    let mut node = None;
    for _ in 0..100 {
        let ctx = glib::MainContext::default();
        let until = Instant::now() + Duration::from_millis(50);
        while Instant::now() < until {
            while ctx.iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
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
    renderer.render_texture(&node, None).save_to_png(path).expect("write png");
}

fn main() {
    if gtk::init().is_err() {
        eprintln!("FAIL: no display for GTK (run under xvfb-run)");
        std::process::exit(1);
    }
    granite::init();
    let app = gtk::Application::builder().application_id("org.techmore.mseries.Updates.RenderTest").build();
    app.register(gio::Cancellable::NONE).expect("register");

    let shots = std::env::var_os("MSERIES_UPDATES_SHOTS").map(std::path::PathBuf::from);
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).unwrap();
    }

    // -- every state the updater can report -------------------------------------------------
    let states: &[(&str, &str, &[&str], &[&str])] = &[
        ("never-checked", NEVER_CHECKED, &["Updates haven't been checked yet", "[Check for Updates]"], &["[Install at Next Restart]", "Previous Version"]),
        ("up-to-date", UP_TO_DATE, &["The M-series software is up to date", "Last checked 5 minutes ago", "Channel: Stable", "Previous Version"], &["[Install at Next Restart]"]),
        ("available", AVAILABLE, &["2 updates available", "mseries-quick-settings", "1.4.0-0mseries3 → 1.4.0-0mseries4", "[Install at Next Restart]"], &["Previous Version"]),
        ("staged", STAGED, &["Ready to install at your next restart", "Ready to install", "[Check Again]"], &["[Install at Next Restart]"]),
        ("awaiting-confirmation", AWAITING, &["Updates installed", "Previous Version", "[Roll Back…]"], &["[Install at Next Restart]"]),
        ("recovering", RECOVERING, &["Restoring the previous versions"], &["[Install at Next Restart]", "Previous Version"]),
        ("rolled-back-by-updater", ROLLED_BACK_BY_UPDATER, &["The last update was rolled back automatically", "no healthy login after 2 restarts", "The M-series software is up to date"], &["[Install at Next Restart]", "1 update available"]),
        ("rollback-failed", ROLLBACK_FAILED, &["Restoring the previous versions failed. The software needs attention:", "stored copy of mseries-update is missing"], &[]),
    ];
    for dark in [false, true] {
        // Granite's stylesheet follows this older setting; gtk-interface-color-scheme does not switch it.
        #[allow(deprecated)]
        gtk::Settings::default().expect("settings").set_gtk_application_prefer_dark_theme(dark);
        for (name, json, expected, absent) in states {
            let case = open(&app, json);
            for text in *expected {
                assert!(has(&case.win, text), "{name}: expected '{text}' in {:?}", shown_text(&case.win));
            }
            for text in *absent {
                assert!(!has(&case.win, text), "{name}: '{text}' should not be shown: {:?}", shown_text(&case.win));
            }
            if let Some(dir) = &shots {
                save_png(&case.win, &dir.join(format!("{name}-{}.png", if dark { "dark" } else { "light" })));
            }
            case.close();
        }
        println!("ok: {} states rendered ({})", states.len(), if dark { "dark" } else { "light" });
    }
    #[allow(deprecated)]
    gtk::Settings::default().unwrap().set_gtk_application_prefer_dark_theme(false);

    // -- a narrow window with long text still shows everything and every control is reachable ----
    {
        let case = open(&app, AVAILABLE);
        case.win.window().set_default_size(360, 640);
        pump_until("narrow layout", || case.win.window().width() <= 400);
        for text in ["2 updates available", "[Install at Next Restart]", "[Check for Updates]"] {
            assert!(has(&case.win, text), "narrow: '{text}' missing in {:?}", shown_text(&case.win));
        }
        let mut unfocusable = vec![];
        walk(case.win.window().upcast_ref(), &mut |w| {
            if let Some(b) = w.downcast_ref::<gtk::Button>() {
                if w.is_mapped() && !b.is_focusable() {
                    unfocusable.push(b.label().map(|l| l.to_string()).unwrap_or_default());
                }
            }
        });
        assert!(unfocusable.is_empty(), "buttons the keyboard cannot reach: {unfocusable:?}");
        if let Some(dir) = &shots {
            save_png(&case.win, &dir.join("available-narrow-light.png"));
        }
        case.close();
        println!("ok: narrow window keeps every control; buttons are keyboard focusable");
    }

    // -- unavailable and unreadable ---------------------------------------------------------
    {
        let fake = Fake::new(NEVER_CHECKED);
        *fake.status.lock().unwrap() = Err(Failure::NotInstalled);
        let win = UpdatesWindow::new(&app, fake.clone());
        win.window().present();
        win.refresh();
        pump_until("unavailable page", || has(&win, "Updates aren't available"));
        assert!(has(&win, "Try Again"), "{:?}", shown_text(&win));
        *fake.status.lock().unwrap() = Ok(AVAILABLE.to_string());
        button(&win, "Try Again").emit_clicked();
        pump_until("recovered after Try Again", || has(&win, "2 updates available"));
        *fake.status.lock().unwrap() = Ok(r#"{"schema":2,"channel":"stable","state":"idle"}"#.to_string());
        win.refresh();
        pump_until("newer schema reported", || has(&win, "newer than this app understands"));
        win.window().destroy();
        println!("ok: not installed / newer schema / Try Again");
    }

    // -- buttons send exactly one command each ---------------------------------------------
    {
        let case = open(&app, AVAILABLE);
        *case.fake.after.lock().unwrap() = Some(STAGED.to_string());
        button(&case.win, "Install at Next Restart").emit_clicked();
        pump_until("staged page", || has(&case.win, "Ready to install at your next restart"));
        assert_eq!(case.fake.commands(), vec![Command::Stage]);
        assert!(!has(&case.win, "[Install at Next Restart]"));
        button(&case.win, "Check Again").emit_clicked();
        pump_until("check ran", || case.fake.commands().len() == 2);
        assert_eq!(case.fake.commands(), vec![Command::Stage, Command::Check]);
        case.close();
        println!("ok: Install at Next Restart -> stage; Check -> check");
    }
    {
        let case = open(&app, AWAITING);
        assert!(!dropdown(&case.win).is_sensitive(), "channel switch must be off while an update awaits confirmation");
        pump_until("advanced shown", || has(&case.win, "Finish or roll back the current update before switching channel."));
        *case.answer.borrow_mut() = false;
        button(&case.win, "Roll Back…").emit_clicked();
        pump_until("question asked", || case.asked.borrow().len() == 1);
        pump_until("settled", || true);
        assert!(case.fake.commands().is_empty(), "declining must not roll back");
        *case.answer.borrow_mut() = true;
        *case.fake.after.lock().unwrap() = Some(UP_TO_DATE.to_string());
        button(&case.win, "Roll Back…").emit_clicked();
        pump_until("rollback ran", || case.fake.commands() == vec![Command::Rollback]);
        let q = case.asked.borrow()[1].clone();
        assert!(q.detail.contains("next time you log in") && q.accept == "Roll Back", "{q:?}");
        case.close();
        println!("ok: Roll Back asks first, sends rollback only when confirmed");
    }
    {
        let case = open(&app, UP_TO_DATE);
        let d = dropdown(&case.win);
        assert_eq!(d.selected(), 0);
        *case.answer.borrow_mut() = false;
        d.set_selected(1);
        pump_until("question asked", || case.asked.borrow().len() == 1);
        pump_until("dropdown reverted", || d.selected() == 0);
        assert!(case.fake.commands().is_empty());
        *case.answer.borrow_mut() = true;
        *case.fake.after.lock().unwrap() = Some(UP_TO_DATE.replace("\"stable\"", "\"dev\""));
        d.set_selected(1);
        pump_until("switch ran", || case.fake.commands() == vec![Command::SwitchChannel(Channel::Dev)]);
        pump_until("status reread", || has(&case.win, "Channel: Development"));
        assert_eq!(d.selected(), 1);
        let q = case.asked.borrow()[1].clone();
        assert!(q.title.starts_with("Switch to Development") && q.detail.contains("less tested"), "{q:?}");
        case.close();
        println!("ok: channel switch asks first, reverts when declined, follows the status");
    }

    // -- busy, cancelled, failed ------------------------------------------------------------
    {
        let case = open(&app, AVAILABLE);
        case.fake.hold();
        button(&case.win, "Check for Updates").emit_clicked();
        pump_until("busy text", || has(&case.win, "Checking for updates…"));
        for label in ["Check for Updates", "Install at Next Restart"] {
            assert!(!button(&case.win, label).is_sensitive(), "'{label}' must be off while a task runs");
        }
        assert!(!dropdown(&case.win).is_sensitive());
        case.fake.release();
        case.settle();
        assert_eq!(case.fake.commands(), vec![Command::Check]);
        case.close();
        println!("ok: controls are off while a task runs");
    }
    for (failure, shown, banner) in [
        (Failure::Cancelled, "2 updates available", false),
        (Failure::NotAuthorized, "You don't have permission to change updates.", true),
        (Failure::Busy, "Another update task is running", true),
        (Failure::Failed("update server unreachable".into()), "update server unreachable", true),
        (Failure::RollbackFailed("copy missing".into()), "Restoring the previous versions failed. The software needs attention: copy missing", true),
    ] {
        let case = open(&app, AVAILABLE);
        *case.fake.result.lock().unwrap() = Err(failure.clone());
        button(&case.win, "Install at Next Restart").emit_clicked();
        pump_until("command attempted", || case.fake.commands().len() == 1);
        case.settle();
        assert!(has(&case.win, shown), "{failure:?}: expected '{shown}' in {:?}", shown_text(&case.win));
        if !banner {
            assert!(!has(&case.win, "permission") && !has(&case.win, "unreachable"), "a dismissed prompt must stay silent");
        }
        assert!(has(&case.win, "[Install at Next Restart]"), "the user can try again after {failure:?}");
        case.close();
    }
    println!("ok: dismissed prompt is silent; refusals and failures are explained and retryable");

    println!("PASS: Software Updates renders every state and sends only the commands it should");
}
