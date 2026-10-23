//! GTK4 front end for the greeter. Meant to run under greetd inside a kiosk compositor, for example
//! `cage -s -- mseries-greeter`. The screen follows `data/greeter.ui.json`; the logic is `state::update`.
#![allow(deprecated)]

use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gtk::{gdk, glib, prelude::*};
use mseries_greeter::greetd::{self, Request, Response};
use mseries_greeter::state::{Cmd, Msg, Phase, State, update};

const CSS: &str = "
window.greeter { background: linear-gradient(160deg, #0d1520, #172334); }
.login-card { background: alpha(#ffffff, 0.06); border-radius: 18px; padding: 36px 40px; min-width: 340px; }
.login-title { font-size: 24px; font-weight: 600; color: #e8eef6; }
.login-prompt { color: #9fb0c4; }
.login-notice { color: #9fb0c4; }
.login-notice.error { color: #ff9a8f; }
.login-card entry { min-height: 40px; }
.login-card button { min-height: 40px; }
";

enum Work {
    Send(Request),
    Cancel,
}

/// Owns the greetd socket; replies go back to the UI thread as messages. A cancel's reply is not forwarded.
fn spawn_worker(mut sock: UnixStream, to_ui: UnboundedSender<Msg>) -> mpsc::Sender<Work> {
    let (tx, rx) = mpsc::channel::<Work>();
    thread::spawn(move || {
        for work in rx {
            let (req, forward) = match work {
                Work::Send(r) => (r, true),
                Work::Cancel => (Request::CancelSession, false),
            };
            match greetd::write_request(&mut sock, &req)
                .and_then(|_| greetd::read_response(&mut sock))
            {
                Ok(reply) if forward => {
                    let _ = to_ui.unbounded_send(Msg::Reply(reply));
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = to_ui.unbounded_send(Msg::ConnectionLost(e.to_string()));
                }
            }
        }
    });
    tx
}

struct Ui {
    title: gtk::Label,
    prompt: gtk::Label,
    notice: gtk::Label,
    user: gtk::Entry,
    answer: gtk::Entry,
    cont: gtk::Button,
    sign_in: gtk::Button,
    not_you: gtk::Button,
}

struct App {
    state: RefCell<State>,
    ui: Ui,
    work: mpsc::Sender<Work>,
    last_user_file: PathBuf,
    gtk_app: gtk::Application,
}

impl App {
    fn dispatch(self: &Rc<Self>, msg: Msg) {
        if matches!(msg, Msg::Reply(Response::Success)) && self.state.borrow().phase == Phase::Done
        {
            if let Some(dir) = self.last_user_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(
                &self.last_user_file,
                self.state.borrow().username.as_bytes(),
            );
            self.gtk_app.quit();
            return;
        }
        let cmd = update(&mut self.state.borrow_mut(), msg);
        match cmd {
            Cmd::Send(r) => {
                let _ = self.work.send(Work::Send(r));
            }
            Cmd::Cancel => {
                let _ = self.work.send(Work::Cancel);
            }
            Cmd::None => {}
        }
        self.render();
    }

    fn render(&self) {
        let s = self.state.borrow();
        let ui = &self.ui;
        let user_step = s.phase == Phase::User;
        let secret = matches!(s.phase, Phase::Prompt { secret: true });
        let prompt_step = matches!(s.phase, Phase::Prompt { .. });
        let busy = s.phase == Phase::Busy;
        for w in [ui.user.upcast_ref::<gtk::Widget>(), ui.cont.upcast_ref()] {
            w.set_visible(user_step);
        }
        for w in [
            ui.answer.upcast_ref::<gtk::Widget>(),
            ui.sign_in.upcast_ref(),
            ui.not_you.upcast_ref(),
            ui.prompt.upcast_ref(),
        ] {
            w.set_visible(prompt_step);
        }
        ui.title.set_text(if !user_step && !s.username.is_empty() {
            &s.username
        } else {
            "Welcome"
        });
        ui.prompt
            .set_text(s.prompt.trim().trim_end_matches(':').trim());
        ui.answer.set_visibility(prompt_step && !secret);
        match &s.notice {
            Some(n) => {
                ui.notice.set_text(&n.text);
                ui.notice.set_visible(true);
                if n.is_error {
                    ui.notice.add_css_class("error");
                } else {
                    ui.notice.remove_css_class("error");
                }
            }
            None => ui.notice.set_visible(false),
        }
        for w in [
            ui.user.upcast_ref::<gtk::Widget>(),
            ui.answer.upcast_ref(),
            ui.cont.upcast_ref(),
            ui.sign_in.upcast_ref(),
            ui.not_you.upcast_ref(),
        ] {
            w.set_sensitive(!busy);
        }
        if user_step {
            ui.user.set_text(&s.username);
            ui.user.grab_focus();
        } else if prompt_step {
            ui.answer.set_text("");
            ui.answer.grab_focus();
        }
    }
}

fn button(label: &str, css: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class(css);
    b
}

fn label(css: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.add_css_class(css);
    l.set_wrap(true);
    l
}

fn build(
    gtk_app: &gtk::Application,
    sock: UnixStream,
    last_user: Option<String>,
    last_user_file: PathBuf,
    cmd: Vec<String>,
) {
    if let (Some(display), provider) = (gdk::Display::default(), gtk::CssProvider::new()) {
        provider.load_from_data(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    let ui = Ui {
        title: label("login-title"),
        prompt: label("login-prompt"),
        notice: label("login-notice"),
        user: gtk::Entry::builder().placeholder_text("User name").build(),
        answer: gtk::Entry::builder()
            .placeholder_text("Password")
            .input_purpose(gtk::InputPurpose::Password)
            .build(),
        cont: button("Continue", "suggested-action"),
        sign_in: button("Sign in", "suggested-action"),
        not_you: button("Not you?", "flat"),
    };
    let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
    card.add_css_class("login-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    for w in [
        ui.title.upcast_ref::<gtk::Widget>(),
        ui.user.upcast_ref(),
        ui.cont.upcast_ref(),
        ui.prompt.upcast_ref(),
        ui.answer.upcast_ref(),
        ui.sign_in.upcast_ref(),
        ui.not_you.upcast_ref(),
        ui.notice.upcast_ref(),
    ] {
        card.append(w);
    }
    let restart = button("Restart", "flat");
    let shutdown = button("Shut down", "flat");
    let power = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    power.set_halign(gtk::Align::End);
    power.set_valign(gtk::Align::End);
    power.set_margin_end(24);
    power.set_margin_bottom(24);
    power.append(&restart);
    power.append(&shutdown);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&card));
    overlay.add_overlay(&power);
    let window = gtk::ApplicationWindow::builder()
        .application(gtk_app)
        .child(&overlay)
        .build();
    window.add_css_class("greeter");
    window.fullscreen();

    let (tx, mut rx) = unbounded::<Msg>();
    let app = Rc::new(App {
        state: RefCell::new(State::new(last_user.clone(), cmd)),
        ui,
        work: spawn_worker(sock, tx),
        last_user_file,
        gtk_app: gtk_app.clone(),
    });

    let a = app.clone();
    app.ui
        .user
        .connect_activate(move |e| a.dispatch(Msg::SubmitUser(e.text().to_string())));
    let a = app.clone();
    app.ui
        .cont
        .connect_clicked(move |_| a.dispatch(Msg::SubmitUser(a.ui.user.text().to_string())));
    let a = app.clone();
    app.ui
        .answer
        .connect_activate(move |e| a.dispatch(Msg::SubmitAnswer(Some(e.text().to_string()))));
    let a = app.clone();
    app.ui.sign_in.connect_clicked(move |_| {
        a.dispatch(Msg::SubmitAnswer(Some(a.ui.answer.text().to_string())))
    });
    let a = app.clone();
    app.ui
        .not_you
        .connect_clicked(move |_| a.dispatch(Msg::ChangeUser));
    restart.connect_clicked(|_| {
        let _ = std::process::Command::new("systemctl")
            .arg("reboot")
            .spawn();
    });
    shutdown.connect_clicked(|_| {
        let _ = std::process::Command::new("systemctl")
            .arg("poweroff")
            .spawn();
    });

    let a = app.clone();
    glib::spawn_future_local(async move {
        while let Some(msg) = rx.next().await {
            a.dispatch(msg);
        }
    });

    app.render();
    window.present();
    // The last user goes straight to the password step; "Not you?" returns to the user name.
    if let Some(user) = last_user.filter(|u| !u.is_empty()) {
        app.dispatch(Msg::SubmitUser(user));
    }
}

fn main() -> glib::ExitCode {
    let Ok(path) = std::env::var("GREETD_SOCK") else {
        eprintln!("mseries-greeter: GREETD_SOCK is not set; run this under greetd");
        return glib::ExitCode::FAILURE;
    };
    let sock = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mseries-greeter: cannot connect to {path}: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    let cmd: Vec<String> = std::env::var("MSERIES_GREETER_CMD")
        .unwrap_or_else(|_| "pantheon-rust-session".into())
        .split_whitespace()
        .map(String::from)
        .collect();
    let last_user_file = PathBuf::from(
        std::env::var("MSERIES_GREETER_STATE")
            .unwrap_or_else(|_| "/var/cache/mseries-greeter/last-user".into()),
    );
    let last_user = std::fs::read_to_string(&last_user_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let gtk_app = gtk::Application::builder()
        .application_id("io.github.techmore.Greeter")
        .build();
    let sock = RefCell::new(Some(sock));
    gtk_app.connect_activate(move |app| {
        if let Some(sock) = sock.borrow_mut().take() {
            build(
                app,
                sock,
                last_user.clone(),
                last_user_file.clone(),
                cmd.clone(),
            );
        }
    });
    gtk_app.run()
}
