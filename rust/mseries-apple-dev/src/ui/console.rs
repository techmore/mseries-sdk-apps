//! The Activity pane: a VTE terminal that runs one toolkit command at a time.
//!
//! Commands run in a real PTY, so the installer's questions, sudo prompts and `xtool auth` work
//! here exactly as in a terminal. The pane keeps the scrollback of earlier commands.

use crate::commands::Job;
use gtk::{gdk, gio, glib, pango, prelude::*};
use mseries_pantheon_kit::{set_accessible_label, set_box_spacing_text_units, set_margins_text_units};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use vte4::prelude::*;

type Finished = Box<dyn Fn(&Job, Outcome)>;
type StateChanged = Box<dyn Fn(bool)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed(i32),
    Stopped,
}

pub struct Console {
    pub root: gtk::Box,
    terminal: vte4::Terminal,
    icon: gtk::Image,
    spinner: gtk::Spinner,
    title: gtk::Label,
    status: gtk::Label,
    stop: gtk::Button,
    pid: Cell<Option<glib::Pid>>,
    job: RefCell<Option<Job>>,
    started: Cell<Option<Instant>>,
    stop_requested: Cell<bool>,
    generation: Cell<u64>,
    on_finished: RefCell<Vec<Finished>>,
    on_state: RefCell<Vec<StateChanged>>,
}

impl Console {
    pub fn new() -> Rc<Console> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("apple-dev-console");

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("apple-dev-console-bar");
        set_box_spacing_text_units(&bar, 0.5);
        set_margins_text_units(&bar, 0.3, 0.6, 0.3, 0.9);
        let icon = gtk::Image::from_icon_name("utilities-terminal-symbolic");
        icon.set_accessible_role(gtk::AccessibleRole::Presentation);
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let title = gtk::Label::builder().label("Activity").xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
        title.add_css_class("heading");
        let status = gtk::Label::builder()
            .label("Commands you run appear here.")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        status.add_css_class("dim-label");
        let stop = gtk::Button::builder().icon_name("media-playback-stop-symbolic").sensitive(false).build();
        stop.add_css_class("flat");
        stop.set_tooltip_text(Some("Stop the running command (Ctrl+.)"));
        set_accessible_label(&stop, "Stop the running command");
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.add_css_class("flat");
        copy.set_tooltip_text(Some("Copy the selection, or everything when nothing is selected"));
        set_accessible_label(&copy, "Copy terminal output");
        let clear = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear.add_css_class("flat");
        clear.set_tooltip_text(Some("Clear the output"));
        set_accessible_label(&clear, "Clear the output");
        bar.append(&icon);
        bar.append(&spinner);
        bar.append(&title);
        bar.append(&status);
        bar.append(&stop);
        bar.append(&copy);
        bar.append(&clear);

        let terminal = vte4::Terminal::new();
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);
        terminal.set_scrollback_lines(20_000);
        terminal.set_scroll_on_output(true);
        terminal.set_scroll_on_keystroke(true);
        terminal.set_mouse_autohide(true);
        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::Off);
        terminal.set_size_request(-1, 120);
        set_accessible_label(&terminal, "Activity terminal");
        let scroll = gtk::ScrolledWindow::builder().child(&terminal).hexpand(true).vexpand(true).build();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);

        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&bar);
        root.append(&scroll);

        let console = Rc::new(Console {
            root,
            terminal,
            icon,
            spinner,
            title,
            status,
            stop,
            pid: Cell::new(None),
            job: RefCell::new(None),
            started: Cell::new(None),
            stop_requested: Cell::new(false),
            generation: Cell::new(0),
            on_finished: RefCell::new(Vec::new()),
            on_state: RefCell::new(Vec::new()),
        });

        apply_colors(&console.terminal);
        #[allow(deprecated)] // The Granite-driven app still sets prefer-dark (see main.rs).
        if let Some(settings) = gtk::Settings::default() {
            let term = console.terminal.clone();
            settings.connect_gtk_application_prefer_dark_theme_notify(move |_| apply_colors(&term));
        }
        let weak = Rc::downgrade(&console);
        console.stop.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.stop();
            }
        });
        let term = console.terminal.clone();
        copy.connect_clicked(move |button| {
            let text = if term.has_selection() {
                term.text_selected(vte4::Format::Text).map(|s| s.to_string())
            } else {
                term.text_format(vte4::Format::Text).map(|s| s.to_string())
            };
            if let Some(text) = text {
                button.clipboard().set_text(text.trim_end());
            }
        });
        let weak = Rc::downgrade(&console);
        clear.connect_clicked(move |_| {
            if let Some(c) = weak.upgrade() {
                c.terminal.reset(true, true);
            }
        });
        let weak = Rc::downgrade(&console);
        console.terminal.connect_child_exited(move |_, status| {
            if let Some(c) = weak.upgrade() {
                c.exited(status);
            }
        });
        console
    }

    pub fn is_running(&self) -> bool {
        self.pid.get().is_some() || self.job.borrow().is_some()
    }

    pub fn running_title(&self) -> Option<String> {
        self.job.borrow().as_ref().map(|j| j.title.clone())
    }

    pub fn connect_finished(&self, f: impl Fn(&Job, Outcome) + 'static) {
        self.on_finished.borrow_mut().push(Box::new(f));
    }

    pub fn connect_running_changed(&self, f: impl Fn(bool) + 'static) {
        self.on_state.borrow_mut().push(Box::new(f));
    }

    pub fn focus_terminal(&self) {
        self.terminal.grab_focus();
    }

    fn feed_line(&self, text: &str) {
        self.terminal.feed(text.replace('\n', "\r\n").as_bytes());
    }

    fn set_running(&self, running: bool) {
        self.spinner.set_visible(running);
        self.spinner.set_spinning(running);
        self.icon.set_visible(!running);
        self.stop.set_sensitive(running);
        for f in self.on_state.borrow().iter() {
            f(running);
        }
    }

    /// Start a job. Returns false (and does nothing) while another command is running.
    pub fn run(self: &Rc<Self>, job: Job) -> bool {
        if self.is_running() {
            return false;
        }
        let cwd = job.cwd.to_string_lossy().into_owned();
        self.feed_line(&format!("\n\x1b[1m▶ {}\x1b[0m  \x1b[2m{}\x1b[0m\n", job.title, cwd));
        self.title.set_text(&job.title);
        self.status.set_text("Starting…");
        self.icon.set_icon_name(Some("utilities-terminal-symbolic"));
        self.stop_requested.set(false);
        self.generation.set(self.generation.get() + 1);
        let env: Vec<String> = job.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let env_refs: Vec<&str> = env.iter().map(String::as_str).collect();
        let script = job.script.clone();
        *self.job.borrow_mut() = Some(job);
        self.started.set(Some(Instant::now()));
        self.set_running(true);

        let this = self.clone();
        self.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(&cwd),
            &["/bin/bash", "--login", "-c", &script],
            &env_refs,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(pid) => {
                    this.pid.set(Some(pid));
                    this.status.set_text("Running");
                    this.tick();
                }
                Err(error) => {
                    this.feed_line(&format!("\x1b[31mCould not start: {}\x1b[0m\n", error.message()));
                    this.finish(Outcome::Failed(127));
                }
            },
        );
        true
    }

    /// Update the elapsed time once a second while the command runs.
    fn tick(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(c) = weak.upgrade() else { return glib::ControlFlow::Break };
            if c.generation.get() != generation || c.pid.get().is_none() {
                return glib::ControlFlow::Break;
            }
            if let Some(start) = c.started.get() {
                let verb = if c.stop_requested.get() { "Stopping" } else { "Running" };
                c.status.set_text(&format!("{verb} · {}", elapsed(start.elapsed())));
            }
            glib::ControlFlow::Continue
        });
    }

    /// Interrupt like Ctrl+C, then escalate if the command ignores it.
    pub fn stop(self: &Rc<Self>) {
        let Some(pid) = self.pid.get() else { return };
        self.stop_requested.set(true);
        self.status.set_text("Stopping…");
        // VTE starts the child as a session leader, so the negative pid reaches its whole group.
        unsafe { libc::kill(-pid.0, libc::SIGINT) };
        let generation = self.generation.get();
        for (delay, signal) in [(4, libc::SIGTERM), (10, libc::SIGKILL)] {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_secs(delay), move || {
                if let Some(c) = weak.upgrade()
                    && c.generation.get() == generation
                    && let Some(pid) = c.pid.get()
                {
                    unsafe { libc::kill(-pid.0, signal) };
                }
            });
        }
    }

    fn exited(&self, status: i32) {
        let outcome = if self.stop_requested.get() {
            Outcome::Stopped
        } else if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
            Outcome::Succeeded
        } else if libc::WIFEXITED(status) {
            Outcome::Failed(libc::WEXITSTATUS(status))
        } else {
            Outcome::Failed(128 + libc::WTERMSIG(status))
        };
        self.finish(outcome);
    }

    fn finish(&self, outcome: Outcome) {
        self.pid.set(None);
        let took = self.started.take().map(|s| elapsed(s.elapsed())).unwrap_or_default();
        let (line, status, icon) = match outcome {
            Outcome::Succeeded => {
                (format!("\x1b[32m✓ Finished in {took}\x1b[0m\n"), format!("Finished · {took}"), "emblem-ok-symbolic")
            }
            Outcome::Failed(code) => (
                format!("\x1b[31m✗ Failed with exit code {code} after {took}\x1b[0m\n"),
                format!("Failed (exit {code}) · {took}"),
                "dialog-error-symbolic",
            ),
            Outcome::Stopped => {
                ("\x1b[33m■ Stopped\x1b[0m\n".to_string(), format!("Stopped · {took}"), "media-playback-stop-symbolic")
            }
        };
        self.feed_line(&line);
        self.status.set_text(&status);
        self.icon.set_icon_name(Some(icon));
        let job = self.job.borrow_mut().take();
        self.set_running(false);
        if let Some(job) = job {
            for f in self.on_finished.borrow().iter() {
                f(&job, outcome);
            }
        }
    }
}

fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 { format!("{s} s") } else { format!("{} min {:02} s", s / 60, s % 60) }
}

#[allow(deprecated)]
fn apply_colors(term: &vte4::Terminal) {
    let dark = gtk::Settings::default().map(|s| s.is_gtk_application_prefer_dark_theme()).unwrap_or(false);
    let (fg, bg) = if dark { ("#e8e8e8", "#1f2326") } else { ("#2c2f33", "#fbfbfc") };
    let palette: Vec<gdk::RGBA> = [
        "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf", "#555753", "#ef2929",
        "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
    ]
    .iter()
    .map(|c| gdk::RGBA::parse(*c).expect("palette colour"))
    .collect();
    let refs: Vec<&gdk::RGBA> = palette.iter().collect();
    term.set_colors(gdk::RGBA::parse(fg).ok().as_ref(), gdk::RGBA::parse(bg).ok().as_ref(), &refs);
}
