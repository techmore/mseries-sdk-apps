//! The terminal pane (INTEGRATIONS.md 4b): one VTE terminal per thread, attached over the server's
//! terminal stream (`terminal.attach`): the snapshot's retained history is fed first, then live
//! output chunks; keystrokes go back through `terminal.write`, size changes through
//! `terminal.resize`. No local PTY. The client keeps at most 512 KiB of history (the contract's
//! client cap); the server retains 5,000 lines / 8 MiB on its side.

use crate::t3::Rpc;
use futures_util::StreamExt;
use gtk::{gdk, glib, prelude::*};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use vte4::prelude::*;

pub const TERMINAL_ID: &str = "term-1";
pub const CLIENT_BUFFER_BYTES: usize = 512 * 1024;

pub struct TerminalPane {
    pub root: gtk::Box,
    term: vte4::Terminal,
    status: gtk::Label,
    thread_id: RefCell<Option<String>>,
    sub_id: RefCell<Option<String>>,
    rpc: RefCell<Option<Rpc>>,
    fed: Cell<usize>,
    cwd: RefCell<String>,
}

/// elementary Terminal's default palette (io.elementary.terminal, "Default" style).
fn palette() -> Vec<gdk::RGBA> {
    [
        "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#ec0048", "#2aa198", "#94a3a5", "#586e75", "#cb4b16",
        "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#6c71c4",
    ]
    .iter()
    .filter_map(|c| gdk::RGBA::parse(*c).ok())
    .collect()
}

impl TerminalPane {
    pub fn new() -> Rc<TerminalPane> {
        let term = vte4::Terminal::new();
        term.set_scrollback_lines(5000);
        term.set_scroll_on_output(false);
        term.set_scroll_on_keystroke(true);
        term.set_mouse_autohide(true);
        term.set_cursor_blink_mode(vte4::CursorBlinkMode::Off);
        term.set_font(Some(&pango::FontDescription::from_string("Monospace 10")));
        term.set_vexpand(true);
        term.set_hexpand(true);
        Self::apply_colors(&term);
        let scroll =
            gtk::ScrolledWindow::builder().child(&term).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let status = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label", "term-status"]).build();
        status.set_margin_start(8);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("terminal-pane");
        root.append(&status);
        root.append(&scroll);
        root.set_size_request(-1, 220);
        let pane = Rc::new(TerminalPane {
            root,
            term,
            status,
            thread_id: RefCell::new(None),
            sub_id: RefCell::new(None),
            rpc: RefCell::new(None),
            fed: Cell::new(0),
            cwd: RefCell::new(String::new()),
        });
        // Keystrokes go to the server's PTY.
        let p = pane.clone();
        pane.term.connect_commit(move |_, text, _| p.write(text));
        // Size follows the widget.
        let p = pane.clone();
        pane.term.connect_char_size_changed(move |_, _, _| p.resize());
        // The first real size arrives after mapping; send it once the widget is on screen.
        let p = pane.clone();
        pane.term.connect_map(move |_| p.resize());
        pane
    }

    fn apply_colors(term: &vte4::Terminal) {
        #[allow(deprecated)]
        let dark = gtk::Settings::default().map(|s| s.is_gtk_application_prefer_dark_theme()).unwrap_or(false);
        let (fg, bg) = if dark { ("#eee8d5", "#252e32") } else { ("#333333", "#fdf6e3") };
        let fg = gdk::RGBA::parse(fg).unwrap();
        let bg = gdk::RGBA::parse(bg).unwrap();
        let palette = palette();
        let refs: Vec<&gdk::RGBA> = palette.iter().collect();
        term.set_colors(Some(&fg), Some(&bg), &refs);
    }

    pub fn is_attached(&self) -> bool {
        self.sub_id.borrow().is_some()
    }

    /// Attaches to `thread_id`'s terminal, opening it in `cwd` when it does not exist yet.
    pub fn attach(self: &Rc<Self>, rpc: Rpc, thread_id: String, cwd: String) {
        self.detach();
        self.term.reset(true, true);
        self.fed.set(0);
        *self.rpc.borrow_mut() = Some(rpc.clone());
        *self.thread_id.borrow_mut() = Some(thread_id.clone());
        *self.cwd.borrow_mut() = cwd.clone();
        self.status.set_text(&format!("Attaching to the server's terminal in {}…", crate::look::tilde(&cwd)));
        let (cols, rows) = (self.term.column_count().max(20), self.term.row_count().max(5));
        let (id, mut stream) = rpc.subscribe(
            "terminal.attach",
            json!({"threadId": thread_id, "terminalId": TERMINAL_ID, "cwd": cwd, "cols": cols, "rows": rows, "restartIfNotRunning": true, "resume": true}),
        );
        *self.sub_id.borrow_mut() = Some(id);
        let pane = self.clone();
        glib::spawn_future_local(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(v) => pane.event(&v),
                    Err(e) => {
                        pane.status.set_text(&format!("Terminal stream ended: {}", e.message()));
                        break;
                    }
                }
            }
            *pane.sub_id.borrow_mut() = None;
        });
    }

    pub fn detach(&self) {
        if let (Some(id), Some(rpc)) = (self.sub_id.borrow_mut().take(), self.rpc.borrow().as_ref()) {
            rpc.interrupt(&id);
        }
    }

    fn event(&self, v: &Value) {
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "snapshot" | "started" | "restarted" => {
                let snap = &v["snapshot"];
                let status = snap.get("status").and_then(Value::as_str).unwrap_or("");
                let pid = snap.get("pid").and_then(Value::as_i64);
                self.status.set_text(&format!(
                    "{} · {}{}",
                    crate::look::tilde(snap.get("cwd").and_then(Value::as_str).unwrap_or("")),
                    status,
                    pid.map(|p| format!(" · pid {p}")).unwrap_or_default()
                ));
                if let Some(history) = snap.get("history").and_then(Value::as_str) {
                    self.term.reset(true, true);
                    self.fed.set(0);
                    self.feed(history);
                }
                self.resize();
            }
            "output" => {
                if let Some(data) = v.get("data").and_then(Value::as_str) {
                    self.feed(data);
                }
            }
            "exited" => {
                let code = v.get("exitCode").and_then(Value::as_i64);
                self.status
                    .set_text(&format!("The shell exited{}", code.map(|c| format!(" with {c}")).unwrap_or_default()));
            }
            "closed" => self.status.set_text("The terminal was closed on the server"),
            "cleared" => {
                self.term.reset(true, true);
                self.fed.set(0);
            }
            "error" => self
                .status
                .set_text(&format!("Terminal error: {}", v.get("message").and_then(Value::as_str).unwrap_or("?"))),
            _ => {}
        }
    }

    /// Feeds bytes to the terminal, dropping the oldest history once the client cap is exceeded.
    fn feed(&self, data: &str) {
        let bytes = data.as_bytes();
        let total = self.fed.get() + bytes.len();
        if total > CLIENT_BUFFER_BYTES {
            // VTE keeps its own ring of scrollback lines; the cap is enforced by resetting the
            // scrollback (not the screen) when the fed volume passes the limit.
            self.term.reset(true, false);
            self.fed.set(0);
        }
        self.term.feed(bytes);
        self.fed.set(self.fed.get() + bytes.len());
    }

    fn write(&self, text: &str) {
        let (Some(rpc), Some(thread_id)) = (self.rpc.borrow().clone(), self.thread_id.borrow().clone()) else { return };
        if text.is_empty() {
            return;
        }
        let payload = json!({"threadId": thread_id, "terminalId": TERMINAL_ID, "data": text});
        glib::spawn_future_local(async move {
            let _ = rpc.call("terminal.write", payload).await;
        });
    }

    fn resize(&self) {
        let (Some(rpc), Some(thread_id)) = (self.rpc.borrow().clone(), self.thread_id.borrow().clone()) else { return };
        let (cols, rows) = (self.term.column_count(), self.term.row_count());
        if cols < 2 || rows < 2 {
            return;
        }
        let payload = json!({"threadId": thread_id, "terminalId": TERMINAL_ID, "cols": cols, "rows": rows});
        glib::spawn_future_local(async move {
            let _ = rpc.call("terminal.resize", payload).await;
        });
    }

    pub fn grab_focus(&self) {
        self.term.grab_focus();
    }
}

use gtk::pango;
