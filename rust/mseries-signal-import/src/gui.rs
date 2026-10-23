//! A small window that shows the Signal history import while it runs: stage,
//! chats and messages done, elapsed time, a clear result, a finish notification
//! and a Cancel that is safe (the import is resumable and idempotent).
//!
//! Counts and stage names only: no message text, names or keys reach the window.

use gtk::{gio, glib, prelude::*};
use mseries_signal_import::{
    import::{self, Config, Observer, Progress, Report},
    keyring::SecretTool,
    progress,
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const APP_ID: &str = "io.github.sdolbec.SignalHistoryImport";

enum Message {
    Progress(Progress),
    Finished(Result<Report, String>),
}

struct Channel {
    sender: mpsc::Sender<Message>,
    cancel: Arc<AtomicBool>,
}

impl Observer for Channel {
    fn progress(&mut self, p: &Progress) {
        let _ = self.sender.send(Message::Progress(*p));
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

struct Ui {
    window: gtk::ApplicationWindow,
    stage: gtk::Label,
    bar: gtk::ProgressBar,
    values: Vec<gtk::Label>,
    result: gtk::Label,
    preview: gtk::Button,
    import: gtk::Button,
    cancel: gtk::Button,
    open: gtk::Button,
    running: bool,
    close_when_done: bool,
    started: Instant,
    dry_run: bool,
    cancel_flag: Arc<AtomicBool>,
}

fn friendly_error(message: &str) -> String {
    if message.contains("is running") {
        "Signal Pantheon is open. Close it, then press Import again. (It would overwrite the imported history from memory.)".into()
    } else if message.contains("keyring") || message.contains("Safe Storage") || message.contains("history key") {
        format!("The login keyring did not give up a key: {message}. Unlock the keyring and try again.")
    } else {
        format!("The import stopped: {message}. Nothing is half-written; you can press Import to try again.")
    }
}

fn done_text(report: &Report) -> String {
    let chats = report.conversations_created + report.conversations_merged;
    let verb = if report.dry_run { "Preview: would add" } else { "Done:" };
    let base = format!("{verb} {} messages in {chats} chats", report.messages_added);
    if report.cancelled {
        format!("Stopped. {base} so far. Press Import to continue where it left off.")
    } else if report.dry_run {
        format!("{base}. Nothing was written yet.")
    } else if report.messages_added == 0 {
        "Done: everything was already imported.".into()
    } else {
        format!("{base}. Open Signal Pantheon to see them.")
    }
}

fn start(ui: &Rc<RefCell<Ui>>, cfg_base: &Rc<dyn Fn() -> Config>, dry_run: bool, receiver: &Rc<RefCell<Option<mpsc::Receiver<Message>>>>) {
    {
        let mut u = ui.borrow_mut();
        if u.running {
            return;
        }
        u.running = true;
        u.dry_run = dry_run;
        u.started = Instant::now();
        u.cancel_flag.store(false, Ordering::Relaxed);
        u.result.set_text("");
        u.stage.set_text("Starting…");
        u.preview.set_sensitive(false);
        u.import.set_sensitive(false);
        u.cancel.set_sensitive(true);
        u.open.set_visible(false);
        for label in &u.values {
            label.set_text("–");
        }
    }
    let (sender, rx) = mpsc::channel();
    *receiver.borrow_mut() = Some(rx);
    let mut cfg = cfg_base();
    cfg.dry_run = dry_run;
    let cancel = ui.borrow().cancel_flag.clone();
    std::thread::spawn(move || {
        let mut channel = Channel {
            sender: sender.clone(),
            cancel,
        };
        let outcome = import::import_with(&cfg, &SecretTool, &mut channel);
        let _ = sender.send(Message::Finished(outcome));
    });
}

fn show_progress(u: &Ui, p: &Progress) {
    u.stage.set_text(&format!("Stage: {}", p.stage.label()));
    match progress::fraction(p) {
        Some(f) => u.bar.set_fraction(f),
        None => u.bar.pulse(),
    }
    let texts = [
        format!("{}", p.conversations_found),
        format!("{}", p.conversations_done),
        format!("{} of {}", p.messages_done, p.messages_total),
        format!("{}", p.messages_added),
        format!("{}", p.duplicates_skipped),
    ];
    for (label, text) in u.values.iter().zip(texts) {
        label.set_text(&text);
    }
}

fn build(app: &gtk::Application, cfg_base: Rc<dyn Fn() -> Config>, auto: Option<bool>) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Bring Signal history into Signal Pantheon")
        .default_width(460)
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for side in ["start", "end", "top", "bottom"] {
        match side {
            "start" => content.set_margin_start(20),
            "end" => content.set_margin_end(20),
            "top" => content.set_margin_top(20),
            _ => content.set_margin_bottom(20),
        }
    }
    let heading = gtk::Label::builder()
        .label("Copy your old Signal Beta chats into Signal Pantheon. Signal Beta's files are only read, never changed.")
        .wrap(true)
        .xalign(0.0)
        .build();
    let stage = gtk::Label::builder().label("Ready").xalign(0.0).css_classes(["heading"]).build();
    let bar = gtk::ProgressBar::new();
    let grid = gtk::Grid::builder().row_spacing(6).column_spacing(18).build();
    let names = ["Chats with messages", "Chats done", "Messages", "Added", "Already there"];
    let mut values = Vec::new();
    for (row, name) in names.iter().enumerate() {
        grid.attach(
            &gtk::Label::builder().label(*name).xalign(0.0).hexpand(true).build(),
            0,
            row as i32,
            1,
            1,
        );
        let value = gtk::Label::builder().label("–").xalign(1.0).build();
        value.add_css_class("numeric");
        grid.attach(&value, 1, row as i32, 1, 1);
        values.push(value);
    }
    let elapsed = gtk::Label::builder()
        .label("Elapsed 0:00")
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let result = gtk::Label::builder().label("").wrap(true).xalign(0.0).selectable(false).build();
    result.set_accessible_role(gtk::AccessibleRole::Status);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let preview = gtk::Button::with_label("Preview");
    preview.set_tooltip_text(Some("Count what would be imported without writing anything"));
    let import_button = gtk::Button::with_label("Import");
    import_button.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_sensitive(false);
    cancel.set_tooltip_text(Some("Stops after the current chat; run Import again to continue"));
    let open = gtk::Button::with_label("Open Signal Pantheon");
    open.set_visible(false);
    for b in [&open, &cancel, &preview, &import_button] {
        buttons.append(b);
    }
    for w in [
        heading.upcast_ref::<gtk::Widget>(),
        stage.upcast_ref(),
        bar.upcast_ref(),
        grid.upcast_ref(),
        elapsed.upcast_ref(),
        result.upcast_ref(),
        buttons.upcast_ref(),
    ] {
        content.append(w);
    }
    window.set_child(Some(&content));

    let ui = Rc::new(RefCell::new(Ui {
        window: window.clone(),
        stage,
        bar,
        values,
        result,
        preview: preview.clone(),
        import: import_button.clone(),
        cancel: cancel.clone(),
        open: open.clone(),
        running: false,
        close_when_done: false,
        started: Instant::now(),
        dry_run: false,
        cancel_flag: Arc::new(AtomicBool::new(false)),
    }));
    let receiver: Rc<RefCell<Option<mpsc::Receiver<Message>>>> = Rc::new(RefCell::new(None));

    {
        let (ui, cfg, rx) = (ui.clone(), cfg_base.clone(), receiver.clone());
        preview.connect_clicked(move |_| start(&ui, &cfg, true, &rx));
    }
    {
        let (ui, cfg, rx) = (ui.clone(), cfg_base.clone(), receiver.clone());
        import_button.connect_clicked(move |_| start(&ui, &cfg, false, &rx));
    }
    {
        let ui = ui.clone();
        cancel.connect_clicked(move |button| {
            ui.borrow().cancel_flag.store(true, Ordering::Relaxed);
            button.set_sensitive(false);
            ui.borrow().stage.set_text("Stopping after the current chat…");
        });
    }
    open.connect_clicked(|_| {
        let _ = std::process::Command::new("signal-pantheon").spawn();
    });
    {
        let ui = ui.clone();
        window.connect_close_request(move |_| {
            let mut u = ui.borrow_mut();
            if u.running {
                u.cancel_flag.store(true, Ordering::Relaxed);
                u.close_when_done = true;
                u.stage.set_text("Stopping after the current chat…");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    // Poll the worker; updates are cheap and arrive at most ten times a second.
    {
        let (ui, rx, app) = (ui.clone(), receiver.clone(), app.clone());
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let mut finished = None;
            {
                let guard = rx.borrow();
                if let Some(receiver) = guard.as_ref() {
                    let u = ui.borrow();
                    while let Ok(message) = receiver.try_recv() {
                        match message {
                            Message::Progress(p) => show_progress(&u, &p),
                            Message::Finished(outcome) => finished = Some(outcome),
                        }
                    }
                }
            }
            let mut u = ui.borrow_mut();
            if u.running {
                elapsed.set_text(&format!("Elapsed {}", progress::elapsed_text(u.started.elapsed())));
            }
            if let Some(outcome) = finished {
                u.running = false;
                u.preview.set_sensitive(true);
                u.import.set_sensitive(true);
                u.cancel.set_sensitive(false);
                let (summary, ok) = match &outcome {
                    Ok(report) => (done_text(report), true),
                    Err(message) => (friendly_error(message), false),
                };
                u.result.set_text(&summary);
                u.stage.set_text(if ok { "Finished" } else { "Stopped" });
                if ok {
                    u.bar.set_fraction(1.0);
                    if matches!(&outcome, Ok(r) if !r.dry_run && r.messages_added > 0) {
                        u.open.set_visible(true);
                    }
                }
                let note = gio::Notification::new(if ok {
                    "Signal history import finished"
                } else {
                    "Signal history import stopped"
                });
                note.set_body(Some(&summary));
                app.send_notification(Some("signal-history-import"), &note);
                if u.close_when_done {
                    u.window.close();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
    if let Some(dry_run) = auto {
        start(&ui, &cfg_base, dry_run, &receiver);
    }
}

fn main() -> glib::ExitCode {
    let mut auto = None;
    let mut home_override: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--start-preview" => auto = Some(true),
            "--start-import" => auto = Some(false),
            "--home" => home_override = args.next().map(PathBuf::from),
            _ => {
                eprintln!("usage: mseries-signal-import-gui [--start-preview|--start-import]");
                return glib::ExitCode::from(2);
            }
        }
    }
    let home = home_override
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .expect("HOME is set");
    let cfg_base: Rc<dyn Fn() -> Config> = Rc::new(move || Config::for_home(&home));
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build(app, cfg_base.clone(), auto));
    app.run_with_args::<&str>(&[])
}
