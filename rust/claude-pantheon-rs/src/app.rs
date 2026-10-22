//! The Claude window: projects and threads from the T3 server (plus the offline JSONL transcripts) on
//! the left, the selected thread as a chat on the right with a composer underneath. Everything that
//! talks to the server runs as futures on the GLib main loop; parsing transcripts runs on gio's pool.

use crate::chat::{self, label};
use crate::index::{self, Session};
use crate::look::{now_unix, tilde};
use crate::t3::attach;
use crate::t3::model::Project;
use crate::t3::model::{Provider, ServerConfig, Shell, ThreadShell, ThreadView, TurnItem};
use crate::t3::{self, Rpc, auth, ws::WsError};
use crate::transcript::{self, Transcript};
use crate::workflow::{self, Issue, PinnedRepo};
use crate::{changes, composer, diagnostics, handoff, notify, palette, repos, sidebar, terminal, usage};
use futures_util::StreamExt;
use granite::prelude::{PlaceholderExt, ToastExt};
use gtk::{gdk, gio, glib, pango, prelude::*};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

/// Transcripts longer than this start folded; a button at the top reveals the rest.
const TAIL_TURNS: usize = 200;
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:3773";

const RUNTIME_MODES: [(&str, &str); 4] = [
    ("approval-required", "Ask before actions"),
    ("auto-accept-edits", "Auto-accept edits"),
    ("auto", "Auto"),
    ("full-access", "Full access"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Thread(usize),
    Session(usize),
}

/// Server state while connected.
pub struct Conn {
    pub rpc: Rpc,
    pub cred: auth::Credential,
    pub config: ServerConfig,
    pub shell: Shell,
    shell_sub: Option<String>,
}

/// The open live thread.
struct Live {
    thread_id: String,
    project_id: String,
    sub_id: String,
    view: ThreadView,
    /// Item id → widget currently in the chat column.
    widgets: HashMap<String, gtk::Widget>,
    running: bool,
}

/// A new thread being composed before its first message.
#[derive(Clone)]
struct Draft {
    project_id: String,
}

pub struct App {
    window: gtk::ApplicationWindow,
    // sidebar
    list: gtk::ListBox,
    rows: RefCell<Vec<Option<Row>>>,
    search: gtk::SearchEntry,
    status: gtk::Label,
    status_box: gtk::Box,
    // main
    stack: gtk::Stack,
    chat: gtk::Box,
    chat_scroll: gtk::ScrolledWindow,
    title: gtk::Label,
    subtitle: gtk::Label,
    toast: granite::Toast,
    // composer
    composer: gtk::Box,
    provider_dd: gtk::DropDown,
    model_dd: gtk::DropDown,
    mode_dd: gtk::DropDown,
    entry: Rc<composer::Composer>,
    send_btn: gtk::Button,
    stop_btn: gtk::Button,
    composer_note: gtk::Label,
    // pairing
    pair_entry: gtk::Entry,
    pair_server: gtk::Entry,
    pair_status: gtk::Label,
    pair_button: gtk::Button,
    // pages
    usage_page: usage::UsagePage,
    repos_page: Rc<repos::ReposPage>,
    diagnostics_page: Rc<diagnostics::DiagnosticsPage>,
    changes_page: Rc<changes::ChangesPage>,
    terminal_pane: Rc<terminal::TerminalPane>,
    terminal_revealer: gtk::Revealer,
    // state
    conn: RefCell<Option<Conn>>,
    sessions: RefCell<Vec<Session>>,
    current_session: RefCell<Option<Session>>,
    transcript: RefCell<Option<Rc<Transcript>>>,
    live: RefCell<Option<Live>>,
    draft: RefCell<Option<Draft>>,
    providers: RefCell<Vec<Provider>>,
    handoff: RefCell<Option<handoff::Pending>>,
    generation: Cell<u64>,
    syncing: Cell<bool>,
    indexing: Cell<bool>,
    loading: Cell<bool>,
    connecting: Cell<bool>,
    list_dirty: Cell<bool>,
    reconnect_attempt: Cell<u32>,
    auto_opened: Cell<bool>,
    palette: RefCell<Option<Rc<palette::Palette>>>,
    sidebar_cfg: RefCell<sidebar::SidebarConfig>,
    started: Instant,
}

fn header_bar() -> gtk::HeaderBar {
    let bar = gtk::HeaderBar::builder().show_title_buttons(false).build();
    bar.add_css_class(granite::STYLE_CLASS_FLAT);
    bar
}

pub fn new_id() -> String {
    glib::uuid_string_random().to_string()
}

impl App {
    pub fn build(application: &gtk::Application, started: Instant, quit_after_frame: bool) -> Rc<App> {
        load_css();

        // ---- sidebar -------------------------------------------------------------------------
        let side_header = header_bar();
        side_header.pack_start(&gtk::WindowControls::new(gtk::PackType::Start));
        side_header.set_title_widget(Some(&label("", &[])));
        let search = gtk::SearchEntry::builder().placeholder_text("Search chats").hexpand(true).build();
        search.set_margin_start(12);
        search.set_margin_end(12);
        search.set_margin_bottom(6);
        let list = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        let list_scroll =
            gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let status = label("Not connected", &["caption", "dim-label"]);
        status.set_hexpand(true);
        let status_icon = gtk::Image::from_icon_name("network-server-symbolic");
        status_icon.add_css_class("dim-label");
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status_box.add_css_class("sb-status");
        status_box.append(&status_icon);
        status_box.append(&status);
        let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side.add_css_class(granite::STYLE_CLASS_SIDEBAR);
        side.set_width_request(260);
        side.append(&side_header);
        side.append(&search);
        side.append(&list_scroll);
        side.append(&status_box);

        // ---- main header ---------------------------------------------------------------------
        let main_header = header_bar();
        let title = label("Claude", &["title"]);
        let subtitle = label("", &["subtitle", "dim-label"]);
        subtitle.set_visible(false);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.set_valign(gtk::Align::Center);
        titles.append(&title);
        titles.append(&subtitle);
        main_header.set_title_widget(Some(&titles));
        let new_chat = gtk::Button::from_icon_name("list-add-symbolic");
        new_chat.set_tooltip_text(Some("New Thread (Ctrl+N)"));
        main_header.pack_start(&new_chat);
        // The app menu (elementary HIG: a MenuButton in the header, sections, accelerator labels drawn by
        // GTK from the `win.*` accelerators registered in `wire`).
        let menu_model = gio::Menu::new();
        let section = |items: &[(&str, &str)]| {
            let m = gio::Menu::new();
            for (label_text, action) in items {
                m.append(Some(label_text), Some(action));
            }
            m
        };
        menu_model.append_section(
            None,
            &section(&[
                ("New Thread", "win.new-thread"),
                ("Command Palette", "win.palette"),
                ("Search Threads", "win.search"),
            ]),
        );
        menu_model.append_section(
            None,
            &section(&[
                ("Usage", "win.usage"),
                ("Repos and Issues", "win.repos"),
                ("Changes in This Thread", "win.changes"),
                ("Diagnostics", "win.diagnostics"),
            ]),
        );
        menu_model.append_section(
            None,
            &section(&[
                ("Import Sessions from Claude Code and Codex", "win.import"),
                ("Hand Off to Repo", "win.handoff"),
                ("Open Offline Twin", "win.offline"),
                ("Close Thread", "win.close-thread"),
                ("Terminal", "win.terminal"),
            ]),
        );
        menu_model.append_section(
            None,
            &section(&[("Settings", "win.settings"), ("Reconnect", "win.reconnect"), ("Forget Pairing", "win.forget")]),
        );
        menu_model.append_section(None, &section(&[("About Claude", "win.about"), ("Quit", "win.quit")]));
        let menu =
            gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu_model).primary(true).build();
        menu.set_tooltip_text(Some("Menu"));
        main_header.pack_end(&gtk::WindowControls::new(gtk::PackType::End));
        main_header.pack_end(&menu);

        // ---- chat ----------------------------------------------------------------------------
        let chat = gtk::Box::new(gtk::Orientation::Vertical, 14);
        chat.add_css_class("chat");
        chat.set_margin_top(18);
        chat.set_margin_bottom(18);
        chat.set_margin_start(24);
        chat.set_margin_end(24);
        let chat_scroll = gtk::ScrolledWindow::builder()
            .child(&chat)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();

        // ---- composer ------------------------------------------------------------------------
        let provider_dd = gtk::DropDown::from_strings(&[]);
        provider_dd.set_tooltip_text(Some("Provider"));
        let model_dd = gtk::DropDown::from_strings(&[]);
        model_dd.set_tooltip_text(Some("Model"));
        let modes: Vec<&str> = RUNTIME_MODES.iter().map(|(_, l)| *l).collect();
        let mode_dd = gtk::DropDown::from_strings(&modes);
        mode_dd.set_selected(0); // supervised by default; full access is an explicit per-thread choice
        mode_dd.set_tooltip_text(Some("Permission mode"));
        let pickers = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        pickers.append(&provider_dd);
        pickers.append(&model_dd);
        pickers.append(&mode_dd);
        let composer_note = label("", &["caption", "dim-label"]);
        composer_note.set_hexpand(true);
        composer_note.set_halign(gtk::Align::End);
        pickers.append(&composer_note);
        let entry = composer::Composer::new();
        let send_btn = gtk::Button::from_icon_name("mail-send-symbolic");
        send_btn.add_css_class("suggested-action");
        send_btn.set_tooltip_text(Some("Send (Enter; Shift+Enter for a new line)"));
        send_btn.set_valign(gtk::Align::End);
        let stop_btn = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop_btn.add_css_class("destructive-action");
        stop_btn.set_tooltip_text(Some("Stop"));
        stop_btn.set_valign(gtk::Align::End);
        stop_btn.set_visible(false);
        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        entry_row.append(&entry.root);
        entry_row.append(&stop_btn);
        entry_row.append(&send_btn);
        let composer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        composer.add_css_class("composer");
        composer.append(&pickers);
        composer.append(&entry_row);

        let terminal_pane = terminal::TerminalPane::new();
        let terminal_revealer = gtk::Revealer::builder()
            .child(&terminal_pane.root)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .reveal_child(false)
            .build();
        let chat_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chat_page.append(&chat_scroll);
        chat_page.append(&terminal_revealer);
        chat_page.append(&composer);

        // ---- other pages ---------------------------------------------------------------------
        let empty = granite::Placeholder::new("Claude for elementary OS");
        empty.set_description("Pick a thread on the left, or start a new one.");
        empty.set_icon(&gio::ThemedIcon::new("mail-read-symbolic"));
        let busy = gtk::Spinner::builder().spinning(true).width_request(32).height_request(32).build();
        busy.set_valign(gtk::Align::Center);
        busy.set_halign(gtk::Align::Center);

        let pair_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        pair_box.set_valign(gtk::Align::Center);
        pair_box.set_halign(gtk::Align::Center);
        pair_box.set_width_request(420);
        let pair_icon = gtk::Image::from_icon_name("network-server-symbolic");
        pair_icon.set_pixel_size(64);
        pair_icon.add_css_class("dim-label");
        pair_box.append(&pair_icon);
        let pair_title = label("Connect to T3 Code", &["title-2"]);
        pair_title.set_xalign(0.5);
        pair_box.append(&pair_title);
        let pair_text = label(
            "Run `t3 pair` on the server and paste the token or the pairing URL. The session credential goes to your keyring.",
            &["dim-label"],
        );
        pair_text.set_wrap(true);
        pair_text.set_ellipsize(pango::EllipsizeMode::None);
        pair_text.set_justify(gtk::Justification::Center);
        pair_text.set_xalign(0.5);
        pair_box.append(&pair_text);
        let pair_server = gtk::Entry::builder().text(DEFAULT_SERVER).placeholder_text("Server URL").build();
        let pair_entry = gtk::Entry::builder().placeholder_text("Pairing token or URL").build();
        let pair_button = gtk::Button::with_label("Pair");
        pair_button.add_css_class("suggested-action");
        pair_button.set_halign(gtk::Align::End);
        let pair_status = label("", &["caption"]);
        pair_status.set_wrap(true);
        pair_status.set_ellipsize(pango::EllipsizeMode::None);
        pair_box.append(&pair_server);
        pair_box.append(&pair_entry);
        pair_box.append(&pair_button);
        pair_box.append(&pair_status);

        let usage_page = usage::UsagePage::new();
        let repos_page = Rc::new(repos::ReposPage::new());
        let diagnostics_page = Rc::new(diagnostics::DiagnosticsPage::new());
        let changes_page = changes::ChangesPage::new();

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&busy, Some("busy"));
        stack.add_named(&pair_box, Some("pair"));
        stack.add_named(&chat_page, Some("chat"));
        stack.add_named(usage_page.widget(), Some("usage"));
        stack.add_named(repos_page.widget(), Some("repos"));
        stack.add_named(diagnostics_page.widget(), Some("diagnostics"));
        stack.add_named(&changes_page.root, Some("changes"));
        stack.set_vexpand(true);

        let toast = granite::Toast::new("");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        overlay.add_overlay(&toast);

        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.append(&main_header);
        main.append(&overlay);

        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&side)
            .end_child(&main)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(280)
            .build();

        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Claude")
            .default_width(1180)
            .default_height(760)
            .icon_name("io.github.techmore.ClaudePantheonRs")
            .child(&paned)
            .build();
        // Both header bars carry the window controls, so the real title bar stays hidden (elementary style).
        window.set_titlebar(Some(&gtk::Grid::builder().visible(false).build()));

        let app = Rc::new(App {
            window,
            list,
            rows: RefCell::new(Vec::new()),
            search,
            status,
            status_box,
            stack,
            chat,
            chat_scroll,
            title,
            subtitle,
            toast,
            composer,
            provider_dd,
            model_dd,
            mode_dd,
            entry,
            send_btn,
            stop_btn,
            composer_note,
            pair_entry,
            pair_server,
            pair_status,
            pair_button,
            usage_page,
            repos_page,
            diagnostics_page,
            changes_page,
            terminal_pane,
            terminal_revealer,
            conn: RefCell::new(None),
            sessions: RefCell::new(Vec::new()),
            current_session: RefCell::new(None),
            transcript: RefCell::new(None),
            live: RefCell::new(None),
            draft: RefCell::new(None),
            providers: RefCell::new(Vec::new()),
            handoff: RefCell::new(None),
            generation: Cell::new(0),
            syncing: Cell::new(false),
            indexing: Cell::new(false),
            loading: Cell::new(false),
            connecting: Cell::new(false),
            list_dirty: Cell::new(false),
            reconnect_attempt: Cell::new(0),
            auto_opened: Cell::new(false),
            palette: RefCell::new(None),
            sidebar_cfg: RefCell::new(sidebar::SidebarConfig::load()),
            started,
        });

        // First frame: report start-up time when asked, and quit when benchmarking.
        if std::env::var_os("CLAUDE_PANTHEON_TIMING").is_some() || quit_after_frame {
            let app2 = app.clone();
            let once = Cell::new(false);
            app.window.add_tick_callback(move |_, _| {
                if !once.replace(true) {
                    eprintln!("first frame: {} ms after main()", app2.started.elapsed().as_millis());
                    if quit_after_frame {
                        app2.window.close();
                    }
                }
                glib::ControlFlow::Break
            });
        }

        app.wire(&new_chat);
        app.wire_composer();
        app.window.present();
        app.reindex();
        if std::env::var_os("CLAUDE_PANTHEON_OFFLINE").is_none() {
            app.connect_stored();
        } else {
            app.set_status("Offline mode", false);
        }
        app
    }

    fn wire(self: &Rc<Self>, new_chat: &gtk::Button) {
        let a = self.clone();
        self.list.connect_row_selected(move |_, row| {
            if a.syncing.get() {
                return;
            }
            let Some(row) = row else { return };
            let picked = a.rows.borrow().get(row.index() as usize).copied().flatten();
            match picked {
                Some(Row::Thread(i)) => {
                    let thread = a.conn.borrow().as_ref().and_then(|c| c.shell.threads.get(i).cloned());
                    if let Some(t) = thread {
                        a.open_thread(t);
                    }
                }
                Some(Row::Session(i)) => {
                    let s = a.sessions.borrow().get(i).cloned();
                    if let Some(s) = s {
                        a.open_session(s);
                    }
                }
                None => {}
            }
        });
        let a = self.clone();
        self.search.connect_search_changed(move |_| a.rebuild_list());
        let a = self.clone();
        new_chat.connect_clicked(move |_| a.new_chat());
        let a = self.clone();
        self.send_btn.connect_clicked(move |_| a.send());
        let a = self.clone();
        self.stop_btn.connect_clicked(move |_| a.stop());
        let a = self.clone();
        self.pair_button.connect_clicked(move |_| a.pair());
        let a = self.clone();
        self.pair_entry.connect_activate(move |_| a.pair());
        let a = self.clone();
        self.provider_dd.connect_selected_notify(move |_| a.fill_models());

        // Window actions with their HIG accelerators; the menu shows the labels automatically.
        type Action = Box<dyn Fn(&Rc<App>)>;
        let actions: Vec<(&str, &[&str], Action)> = vec![
            ("new-thread", &["<Control>n"], Box::new(|a| a.new_chat())),
            (
                "palette",
                &["<Control>k"],
                Box::new(|a| {
                    a.palette();
                }),
            ),
            ("search", &["<Control>f"], Box::new(|a| a.focus_search())),
            ("send", &["<Control>Return", "<Control>KP_Enter"], Box::new(|a| a.send())),
            ("usage", &["<Control><Shift>u"], Box::new(|a| a.show_usage())),
            ("repos", &[], Box::new(|a| a.show_repos())),
            ("changes", &[], Box::new(|a| a.show_changes())),
            ("diagnostics", &[], Box::new(|a| a.show_diagnostics())),
            ("import", &[], Box::new(|a| a.import_sessions())),
            ("handoff", &[], Box::new(|a| a.handoff_to_repo())),
            ("offline", &[], Box::new(|a| a.open_offline_twin())),
            ("close-thread", &["<Control>w"], Box::new(|a| a.close_thread())),
            ("terminal", &["<Control><Shift>t"], Box::new(|a| a.toggle_terminal())),
            ("settings", &["<Control>comma"], Box::new(|a| a.settings())),
            ("reconnect", &[], Box::new(|a| a.connect_stored())),
            ("forget", &[], Box::new(|a| a.forget())),
            ("about", &[], Box::new(|a| a.about())),
            ("quit", &["<Control>q"], Box::new(|a| a.window.close())),
        ];
        for (name, what) in [
            ("thread-pin", "pin"),
            ("thread-rename", "rename"),
            ("thread-handoff", "handoff"),
            ("thread-copy-link", "copy-link"),
            ("thread-archive", "archive"),
        ] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let a = self.clone();
            action.connect_activate(move |_, param| {
                if let Some(id) = param.and_then(|p| p.get::<String>()) {
                    a.thread_command(&id, what);
                }
            });
            self.window.add_action(&action);
        }
        // Y and N answer the oldest pending permission prompt when nothing is being typed.
        let keys = gtk::EventControllerKey::new();
        let a = self.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            if state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            let typing = GtkWindowExt::focus(&a.window).is_some_and(|w| {
                w.is::<gtk::Text>() || w.is::<gtk::TextView>() || w.is::<gtk::Entry>() || w.is::<gtk::SearchEntry>()
            });
            if typing {
                return glib::Propagation::Proceed;
            }
            let decision = match key {
                gdk::Key::y | gdk::Key::Y => "accept",
                gdk::Key::n | gdk::Key::N => "decline",
                _ => return glib::Propagation::Proceed,
            };
            match a.oldest_pending_request() {
                Some(id) => {
                    a.answer_request(&id, decision);
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);
        let application = self.window.application();
        for (name, accels, f) in actions {
            let action = gio::SimpleAction::new(name, None);
            let a = self.clone();
            action.connect_activate(move |_, _| f(&a));
            self.window.add_action(&action);
            if !accels.is_empty()
                && let Some(app) = &application
            {
                app.set_accels_for_action(&format!("win.{name}"), accels);
            }
        }
    }

    fn focus_search(&self) {
        self.search.grab_focus();
        self.search.select_region(0, -1);
    }

    /// Ctrl+K: the command palette over threads, transcripts, projects, repos, issues and actions.
    pub fn palette(self: &Rc<Self>) -> Rc<palette::Palette> {
        let catalog = self.palette_catalog();
        let app = self.clone();
        let p = palette::Palette::open(
            self.window.upcast_ref(),
            catalog,
            Rc::new(move |action| app.run_palette_action(action)),
        );
        *self.palette.borrow_mut() = Some(p.clone());
        p
    }

    fn palette_catalog(&self) -> Vec<palette::Entry> {
        use palette::{Action, Entry};
        let mut out = Vec::new();
        let providers = self.providers.borrow();
        for p in providers.iter().filter(|p| p.ready()) {
            out.push(Entry {
                title: format!("New thread on {}", p.name()),
                kind: "Action".into(),
                action: Action::NewThread { provider: Some(p.instance_id.clone()) },
            });
        }
        for p in providers.iter().filter(|p| p.ready()) {
            for m in &p.models {
                out.push(Entry {
                    title: format!("Switch model to {} ({})", m.display(), p.name()),
                    kind: "Model".into(),
                    action: Action::SetModel { provider: p.instance_id.clone(), model: m.key().to_string() },
                });
            }
        }
        for (title, page) in [
            ("Open Usage", "usage"),
            ("Open Repos and Issues", "repos"),
            ("Open Diagnostics", "diagnostics"),
            ("Changes in this thread", "changes"),
        ] {
            out.push(Entry { title: title.into(), kind: "Page".into(), action: Action::Page(page.into()) });
        }
        for (title, action) in [
            ("Pair with a T3 environment", "pair"),
            ("Import sessions from Claude Code and Codex", "import"),
            ("Hand off to repo", "handoff"),
            ("Open offline twin", "offline"),
            ("Reconnect", "reconnect"),
            ("Close thread", "close-thread"),
            ("About Claude", "about"),
        ] {
            out.push(Entry { title: title.into(), kind: "Action".into(), action: Action::WindowAction(action.into()) });
        }
        if let Some(c) = self.conn.borrow().as_ref() {
            for p in &c.shell.projects {
                out.push(Entry {
                    title: format!("New thread in {}", p.title),
                    kind: "Project".into(),
                    action: Action::NewThreadIn(p.id.clone()),
                });
            }
            for t in c.shell.threads.iter().filter(|t| t.archived_at.is_none()) {
                let provider = providers
                    .iter()
                    .find(|p| p.instance_id == t.provider_instance_id)
                    .map(Provider::name)
                    .unwrap_or_default();
                let title = crate::transcript::title_from_prompt(&t.title).unwrap_or_else(|| "Untitled thread".into());
                out.push(Entry { title, kind: format!("Thread · {provider}"), action: Action::OpenId(t.id.clone()) });
            }
        }
        for s in self.sessions.borrow().iter() {
            out.push(Entry {
                title: s.title.clone(),
                kind: format!("Transcript · {}", s.project()),
                action: Action::OpenId(s.id.clone()),
            });
        }
        let config = self.repos_page.config();
        for r in &config.repos {
            out.push(Entry {
                title: format!("Issues in {}", r.repo),
                kind: "Repo".into(),
                action: Action::Repo(r.repo.clone()),
            });
        }
        for (repo, issue) in self.repos_page.cached_issues() {
            out.push(Entry {
                title: format!("#{} {}", issue.number, issue.title),
                kind: format!("Issue · {repo}"),
                action: Action::StartIssue { repo, number: issue.number },
            });
        }
        out
    }

    fn run_palette_action(self: &Rc<Self>, action: palette::Action) {
        use palette::Action;
        match action {
            Action::OpenId(id) => self.open_by_id(&id),
            Action::NewThread { provider } => {
                if let Some(p) = provider {
                    self.select_provider(&p, None);
                }
                self.new_chat();
            }
            Action::NewThreadIn(project_id) => self.new_chat_in(Some(project_id)),
            Action::SetModel { provider, model } => {
                self.select_provider(&provider, Some(&model));
                self.toast(&format!("Next message goes to {model}"));
            }
            Action::Page(page) => match page.as_str() {
                "usage" => self.show_usage(),
                "repos" => self.show_repos(),
                "diagnostics" => self.show_diagnostics(),
                "changes" => self.show_changes(),
                _ => {}
            },
            Action::Repo(repo) => {
                self.show_repos();
                let pinned = self.repos_page.config().repos.into_iter().find(|r| r.repo == repo);
                if let Some(r) = pinned {
                    self.repos_page.show_issues(self, r);
                }
            }
            Action::StartIssue { repo, number } => {
                let pinned = self.repos_page.config().repos.into_iter().find(|r| r.repo == repo);
                let issue = self
                    .repos_page
                    .cached_issues()
                    .into_iter()
                    .find(|(r, i)| *r == repo && i.number == number)
                    .map(|(_, i)| i);
                match (pinned, issue) {
                    (Some(r), Some(i)) => self.start_issue(r, i),
                    _ => self.toast("Open the repo's issues first."),
                }
            }
            Action::WindowAction(name) => {
                if name == "pair" {
                    self.show_page("pair");
                } else {
                    WidgetExt::activate_action(&self.window, &format!("win.{name}"), None).ok();
                }
            }
        }
    }

    /// Ctrl+W: leaves the open thread or transcript and shows the empty page.
    pub fn close_thread(self: &Rc<Self>) {
        self.leave_live();
        *self.current_session.borrow_mut() = None;
        *self.transcript.borrow_mut() = None;
        self.list.unselect_all();
        self.title.set_text("Claude");
        self.window.set_title(Some("Claude"));
        self.subtitle.set_visible(false);
        self.show_page("empty");
    }

    /// Ctrl+,: the settings window lands with its own issue; until then say so.
    fn settings(self: &Rc<Self>) {
        self.toast("Settings arrive with the settings-window issue; pairing lives in the menu for now.");
    }

    /// About: version, server and provider state, in a Granite-styled dialog.
    fn about(self: &Rc<Self>) {
        let (server, version, environment) = match self.conn.borrow().as_ref() {
            Some(c) => (
                format!("{} ({})", c.cred.label, c.cred.server),
                c.config.environment.get("serverVersion").and_then(Value::as_str).unwrap_or("?").to_string(),
                c.config
                    .environment
                    .get("platform")
                    .map(|p| {
                        format!(
                            "{} {}",
                            p.get("os").and_then(Value::as_str).unwrap_or(""),
                            p.get("arch").and_then(Value::as_str).unwrap_or("")
                        )
                    })
                    .unwrap_or_default(),
            ),
            None => ("not connected".to_string(), "–".to_string(), String::new()),
        };
        let ready: Vec<String> = self.providers.borrow().iter().filter(|p| p.ready()).map(Provider::name).collect();
        let dialog = gtk::AboutDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .program_name("Claude")
            .logo_icon_name("io.github.techmore.ClaudePantheonRs")
            .version(env!("CARGO_PKG_VERSION"))
            .comments(format!(
                "A native elementary OS client of the T3 Code server.\n\nServer: {server}\nServer version: {version} {environment}\nProviders ready: {}",
                if ready.is_empty() { "none".to_string() } else { ready.join(", ") }
            ))
            .website("https://github.com/techmore/ElementaryOS_MSeries")
            .website_label("techmore/ElementaryOS_MSeries")
            .license_type(gtk::License::Gpl30Only)
            .build();
        dialog.add_css_class("about-dialog");
        dialog.present();
    }

    /// Enter sends; pasted or dropped files upload to the server; `@` searches the project's files.
    fn wire_composer(self: &Rc<Self>) {
        let a = self.clone();
        self.entry.on_send(Rc::new(move || a.send()));
        let a = self.clone();
        self.entry.on_attach(Rc::new(move |bytes, name| a.upload_attachment(bytes, name)));
        let a = self.clone();
        self.entry.on_search(Rc::new(move |query, reply| a.search_project_files(query, reply)));
    }

    fn upload_attachment(self: &Rc<Self>, bytes: Vec<u8>, name: String) {
        let (rpc, server) = match self.conn.borrow().as_ref() {
            Some(c) => (c.rpc.clone(), c.cred.server.clone()),
            None => {
                self.entry.set_status("Connect to the server to attach files.");
                return;
            }
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            match attach::upload(&server, &rpc, &name, bytes).await {
                Ok(a) => app.entry.add_attachment(a),
                Err(e) => app.entry.set_status(&format!("Could not attach {name}: {e}")),
            }
        });
    }

    /// The project folder the composer writes into: the open thread's, the draft's, else none.
    fn composer_cwd(&self) -> Option<String> {
        let conn = self.conn.borrow();
        let c = conn.as_ref()?;
        let project_id = self
            .live
            .borrow()
            .as_ref()
            .map(|l| l.project_id.clone())
            .or_else(|| self.draft.borrow().as_ref().map(|d| d.project_id.clone()))?;
        c.shell.projects.iter().find(|p| p.id == project_id).map(|p| p.workspace_root.clone())
    }

    fn search_project_files(self: &Rc<Self>, query: String, reply: composer::SearchReply) {
        let (Some(rpc), Some(cwd)) = (self.conn.borrow().as_ref().map(|c| c.rpc.clone()), self.composer_cwd()) else {
            reply(Vec::new());
            return;
        };
        glib::spawn_future_local(async move {
            let payload = json!({"cwd": cwd, "query": query, "limit": 40});
            let paths = match rpc.call("projects.searchEntries", payload).await {
                Ok(v) => v
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter().filter_map(|e| e.get("path").and_then(Value::as_str).map(str::to_string)).collect()
                    })
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            reply(paths);
        });
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    pub fn is_busy(&self) -> bool {
        self.indexing.get() || self.loading.get() || self.connecting.get()
    }

    pub fn is_connected(&self) -> bool {
        self.conn.borrow().is_some()
    }

    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.borrow().clone()
    }

    pub fn threads(&self) -> Vec<ThreadShell> {
        self.conn.borrow().as_ref().map(|c| c.shell.threads.clone()).unwrap_or_default()
    }

    pub fn providers(&self) -> Vec<Provider> {
        self.providers.borrow().clone()
    }

    pub fn live_item_count(&self) -> usize {
        self.live.borrow().as_ref().map(|l| l.view.items.len()).unwrap_or(0)
    }

    pub fn set_search(&self, text: &str) {
        self.search.set_text(text);
    }

    pub fn composer(&self) -> &Rc<composer::Composer> {
        &self.entry
    }

    pub fn set_entry(&self, text: &str) {
        self.entry.set_text(text);
    }

    pub fn toast(&self, text: &str) {
        self.toast.set_title(text);
        self.toast.send_notification();
    }

    pub fn show_page(&self, name: &str) {
        if self.page() == "diagnostics"
            && name != "diagnostics"
            && let Some(c) = self.conn.borrow().as_ref()
        {
            self.diagnostics_page.stop(&c.rpc); // the server samples only while someone watches
        }
        self.stack.set_visible_child_name(name);
    }

    pub fn projects(&self) -> Vec<Project> {
        self.conn.borrow().as_ref().map(|c| c.shell.projects.clone()).unwrap_or_default()
    }

    pub fn repos_page(&self) -> &repos::ReposPage {
        &self.repos_page
    }

    pub fn repos_page_rc(&self) -> Rc<repos::ReposPage> {
        self.repos_page.clone()
    }

    pub fn diagnostics_page(&self) -> &diagnostics::DiagnosticsPage {
        &self.diagnostics_page
    }

    pub fn page(&self) -> String {
        self.stack.visible_child_name().map(|s| s.to_string()).unwrap_or_default()
    }

    // ---- connection ------------------------------------------------------------------------

    fn set_status(&self, text: &str, connected: bool) {
        self.status.set_text(text);
        if connected {
            self.status_box.add_css_class("connected");
        } else {
            self.status_box.remove_css_class("connected");
        }
    }

    /// After a dropped connection: retry with a growing delay (2, 4, 8, 16, 32 s, then every minute),
    /// and reopen the thread that was on screen once the server is back.
    fn schedule_reconnect(self: &Rc<Self>) {
        let attempt = self.reconnect_attempt.get();
        self.reconnect_attempt.set(attempt + 1);
        let delay = std::time::Duration::from_secs(2u64.pow(attempt.min(5)).max(2));
        self.set_status(&format!("Disconnected; retrying in {} s", delay.as_secs()), false);
        let app = self.clone();
        glib::spawn_future_local(async move {
            glib::timeout_future(delay).await;
            if app.conn.borrow().is_some() {
                return;
            }
            let reopen = app.live.borrow().as_ref().map(|l| l.thread_id.clone());
            let Some(cred) = auth::load().await else { return };
            if app.connecting.replace(true) {
                return;
            }
            app.connect_with(cred).await;
            if app.conn.borrow().is_none() {
                app.schedule_reconnect();
                return;
            }
            if let Some(id) = reopen {
                let shell = {
                    let conn = app.conn.borrow();
                    conn.as_ref().and_then(|c| c.shell.threads.iter().find(|t| t.id == id).cloned())
                };
                match shell {
                    Some(t) => app.open_thread(t),
                    None => {
                        // The thread list has not arrived yet; the thread stays open and its stream is
                        // re-established when the user selects it again.
                        app.toast("Reconnected.");
                    }
                }
            }
        });
    }

    /// Connects with the stored credential, or shows the pairing page.
    pub fn connect_stored(self: &Rc<Self>) {
        if self.connecting.replace(true) {
            return;
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            match auth::load().await {
                Some(cred) => app.connect_with(cred).await,
                None => {
                    app.connecting.set(false);
                    app.set_status("Not paired", false);
                    app.show_page("pair");
                }
            }
        });
    }

    fn pair(self: &Rc<Self>) {
        let input = self.pair_entry.text().to_string();
        if input.trim().is_empty() {
            self.pair_status.set_text("Paste the token from `t3 pair`.");
            return;
        }
        let (server, token) = auth::parse_pairing(&input, &self.pair_server.text());
        self.pair_status.set_text("Pairing…");
        self.pair_button.set_sensitive(false);
        let app = self.clone();
        glib::spawn_future_local(async move {
            match auth::pair(&server, &token).await {
                Ok(cred) => {
                    let at = auth::store(&cred).await;
                    app.pair_status.set_text(&format!("Paired with {}. Credential stored in the {at}.", cred.label));
                    app.pair_entry.set_text("");
                    app.connecting.set(true);
                    app.connect_with(cred).await;
                }
                Err(e) => app.pair_status.set_text(&format!("Pairing failed: {e}")),
            }
            app.pair_button.set_sensitive(true);
        });
    }

    async fn connect_with(self: &Rc<Self>, cred: auth::Credential) {
        self.set_status(&format!("Connecting to {}…", cred.server), false);
        let rpc = match t3::connect(&cred).await {
            Ok(r) => r,
            Err(e) => {
                self.connecting.set(false);
                let text = match &e {
                    WsError::Refused { status: 426, body } => format!("Server needs a newer client: {body}"),
                    WsError::Refused { status: 401, .. } | WsError::Refused { status: 403, .. } => {
                        "The stored pairing was rejected; pair again.".to_string()
                    }
                    other => format!("Could not connect: {other}"),
                };
                self.set_status("Not connected", false);
                self.pair_status.set_text(&text);
                self.show_page("pair");
                self.rebuild_list();
                return;
            }
        };
        let config: ServerConfig = match rpc.call("server.getConfig", json!({})).await {
            Ok(v) => serde_json::from_value(v).unwrap_or_default(),
            Err(e) => {
                self.connecting.set(false);
                self.set_status(&format!("Config failed: {e}"), false);
                return;
            }
        };
        let app = self.clone();
        rpc.on_close(move |reason| {
            *app.conn.borrow_mut() = None;
            app.set_status(&format!("Disconnected: {reason}"), false);
            app.rebuild_list();
            app.write_status_feed();
            app.schedule_reconnect();
        });
        self.reconnect_attempt.set(0);
        *self.providers.borrow_mut() = config.providers.clone();
        *self.conn.borrow_mut() =
            Some(Conn { rpc: rpc.clone(), cred: cred.clone(), config, shell: Shell::default(), shell_sub: None });
        self.connecting.set(false);
        self.refresh_status();
        self.fill_providers();
        if self.page() == "pair" {
            self.show_page("empty");
        }
        self.offer_local_claude();
        self.subscribe_shell();
        // Plan windows move; re-read the provider snapshot every five minutes (one small RPC).
        let app = self.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(300), move || {
            if app.conn.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            let app2 = app.clone();
            glib::spawn_future_local(async move {
                app2.reload_config().await;
                if app2.page() == "usage" {
                    app2.usage_page.render_windows(&app2.providers());
                }
            });
            glib::ControlFlow::Continue
        });
    }

    /// Writes the indicator feed (status.json) from what the window knows now.
    pub fn write_status_feed(&self) {
        let (connected, label_text, threads) = match self.conn.borrow().as_ref() {
            Some(c) => (true, c.cred.label.clone(), c.shell.threads.clone()),
            None => (false, String::new(), Vec::new()),
        };
        crate::status::write(&crate::status::build(connected, &label_text, &self.providers.borrow(), &threads));
    }

    fn refresh_status(&self) {
        self.write_status_feed();
        let label_text = self.conn.borrow().as_ref().map(|c| c.cred.label.clone()).unwrap_or_default();
        let ready: Vec<String> = self.providers.borrow().iter().filter(|p| p.ready()).map(Provider::name).collect();
        self.set_status(
            &format!("{label_text} · {}", if ready.is_empty() { "no provider ready".into() } else { ready.join(", ") }),
            true,
        );
    }

    /// When the server runs on this machine and cannot find `claude`, point it at the Claude Code
    /// binary bundled with the desktop app.
    fn offer_local_claude(self: &Rc<Self>) {
        let (server, needs) = {
            let conn = self.conn.borrow();
            let Some(c) = conn.as_ref() else { return };
            let needs = c.config.providers.iter().any(|p| p.driver == "claudeAgent" && p.enabled && !p.installed);
            (c.cred.server.clone(), needs)
        };
        if !needs || !(server.contains("127.0.0.1") || server.contains("localhost")) {
            return;
        }
        let Some(cli) = crate::claude_cli::find() else { return };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let rpc = match app.conn.borrow().as_ref() {
                Some(c) => c.rpc.clone(),
                None => return,
            };
            let payload = json!({
                "patch": {},
                "providerInstanceMutation": {
                    "operation": "upsert", "instanceId": "claudeAgent",
                    "instance": {"driver": "claudeAgent", "enabled": true, "config": {"binaryPath": cli.to_string_lossy()}}
                }
            });
            if rpc.call("server.updateSettings", payload).await.is_ok() {
                let _ = rpc.call("server.refreshProviders", json!({})).await;
                glib::timeout_future(std::time::Duration::from_secs(2)).await;
                app.reload_config().await;
                app.toast(&format!("Pointed the server at Claude Code at {}", tilde(&cli.to_string_lossy())));
            }
        });
    }

    async fn reload_config(&self) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => return,
        };
        if let Ok(v) = rpc.call("server.getConfig", json!({})).await
            && let Ok(config) = serde_json::from_value::<ServerConfig>(v)
        {
            *self.providers.borrow_mut() = config.providers.clone();
            if let Some(c) = self.conn.borrow_mut().as_mut() {
                c.config = config;
            }
            self.fill_providers();
            self.refresh_status();
        }
    }

    fn subscribe_shell(self: &Rc<Self>) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => return,
        };
        let (id, mut stream) = rpc.subscribe("orchestration.subscribeShell", json!({"requestCompletionMarker": true}));
        if let Some(c) = self.conn.borrow_mut().as_mut() {
            c.shell_sub = Some(id);
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            while let Some(item) = stream.next().await {
                match item {
                    Ok(v) => app.shell_event(&v),
                    Err(e) => {
                        app.set_status(&format!("Thread list stopped: {e}"), false);
                        break;
                    }
                }
            }
        });
    }

    fn shell_event(self: &Rc<Self>, v: &Value) {
        let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
        {
            let mut conn = self.conn.borrow_mut();
            let Some(c) = conn.as_mut() else { return };
            match kind {
                "snapshot" => {
                    // A snapshot carrying `resolvedRepositoryIdentityRoots` is a metadata-only enrichment
                    // (project identities); the thread list it carries is empty and must be kept.
                    let fresh = Shell::from_snapshot(&v["snapshot"]);
                    if v.get("resolvedRepositoryIdentityRoots").is_some() && !c.shell.threads.is_empty() {
                        for p in fresh.projects {
                            c.shell.upsert_project(p);
                        }
                    } else {
                        c.shell = fresh;
                    }
                }
                "thread.updated" => {
                    if let Ok(t) = serde_json::from_value::<ThreadShell>(v["thread"].clone()) {
                        let open = self.live.borrow().as_ref().is_some_and(|l| l.thread_id == t.id);
                        if open {
                            self.title.set_text(&t.title);
                            self.window.set_title(Some(&format!("{} - Claude", t.title)));
                        }
                        if v.get("location").and_then(Value::as_str) == Some("archive") {
                            c.shell.remove_thread(&t.id);
                        } else {
                            c.shell.upsert_thread(t);
                        }
                    }
                }
                "thread.removed" => {
                    if let Some(id) = v.get("threadId").and_then(Value::as_str) {
                        c.shell.remove_thread(id);
                    }
                }
                "project.updated" => {
                    if let Ok(p) = serde_json::from_value(v["project"].clone()) {
                        c.shell.upsert_project(p);
                    }
                }
                "project.removed" => {
                    if let Some(id) = v.get("projectId").and_then(Value::as_str) {
                        c.shell.projects.retain(|p| p.id != id);
                    }
                }
                _ => return,
            }
        }
        self.schedule_rebuild();
        self.write_status_feed();
        self.refresh_launcher();
    }

    /// The dock badge counts running threads; the bar shows while any turn runs (the server reports
    /// no fraction, so it sits at the half mark as a "working" indicator).
    fn refresh_launcher(&self) {
        let running =
            self.conn.borrow().as_ref().map(|c| c.shell.threads.iter().filter(|t| t.is_running()).count()).unwrap_or(0);
        notify::launcher_entry(running as i64, (running > 0).then_some(0.5));
    }

    /// Notifies about a thread only while the window is not the active one.
    fn notify_background(&self, kind: &str, thread_id: &str, title: &str, body: &str, answer: bool) {
        if self.window.is_active() {
            return;
        }
        let Some(application) = self.window.application() else { return };
        notify::notify(&application, &format!("{kind}:{thread_id}"), title, body, thread_id, answer);
    }

    /// Coalesces bursts of shell events into one list rebuild.
    fn schedule_rebuild(self: &Rc<Self>) {
        if self.list_dirty.replace(true) {
            return;
        }
        let app = self.clone();
        glib::idle_add_local_once(move || {
            app.list_dirty.set(false);
            app.rebuild_list();
        });
    }

    fn forget(self: &Rc<Self>) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            let taken = app.conn.borrow_mut().take();
            if let Some(c) = taken {
                c.rpc.close().await;
            }
            auth::forget().await;
            app.set_status("Not paired", false);
            app.show_page("pair");
            app.rebuild_list();
        });
    }

    // ---- sidebar ---------------------------------------------------------------------------

    pub fn reindex(self: &Rc<Self>) {
        if self.indexing.replace(true) {
            return;
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            let sessions = gio::spawn_blocking(index::index_all).await.unwrap_or_default();
            app.indexing.set(false);
            *app.sessions.borrow_mut() = sessions;
            app.rebuild_list();
        });
    }

    /// Session uuids that are already threads on the server (`import:claudeAgent:<uuid>`).
    fn imported_uuids(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        if let Some(c) = self.conn.borrow().as_ref() {
            for t in &c.shell.threads {
                if let Some(uuid) = t.id.strip_prefix("import:claudeAgent:") {
                    out.insert(uuid.to_string(), t.id.clone());
                }
            }
        }
        out
    }

    pub fn rebuild_list(self: &Rc<Self>) {
        self.syncing.set(true);
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let needle = self.search.text().to_lowercase();
        let now = now_unix();
        let mut rows: Vec<Option<Row>> = Vec::new();
        let mut select_at = None;
        let selected_thread = self.live.borrow().as_ref().map(|l| l.thread_id.clone());
        let selected_session = self.current_session.borrow().as_ref().map(|s| s.id.clone());
        let matches = |title: &str, group: &str| {
            needle.is_empty() || title.to_lowercase().contains(&needle) || group.to_lowercase().contains(&needle)
        };
        let cfg = self.sidebar_cfg.borrow().clone();
        let searching = !needle.is_empty();

        // Server threads grouped by project: the owner's order first, then newest project first;
        // pinned threads lead each section.
        if let Some(c) = self.conn.borrow().as_ref() {
            let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
            let mut order: Vec<usize> = (0..c.shell.threads.len()).collect();
            order.sort_by(|&a, &b| {
                let (ta, tb) = (&c.shell.threads[a], &c.shell.threads[b]);
                sidebar::is_pinned(tb).cmp(&sidebar::is_pinned(ta)).then_with(|| tb.updated_at.cmp(&ta.updated_at))
            });
            for i in order {
                let t = &c.shell.threads[i];
                if t.archived_at.is_some() {
                    continue;
                }
                let name = c
                    .shell
                    .projects
                    .iter()
                    .find(|p| p.id == t.project_id)
                    .map(|p| p.title.clone())
                    .unwrap_or_else(|| "Project".into());
                if !matches(&t.title, &name) {
                    continue;
                }
                match groups.iter_mut().find(|(id, _, _)| *id == t.project_id) {
                    Some((_, _, v)) => v.push(i),
                    None => groups.push((t.project_id.clone(), name, vec![i])),
                }
            }
            let ordered = cfg.order_projects(groups.iter().map(|(id, _, _)| id.clone()).collect());
            groups.sort_by_key(|(id, _, _)| ordered.iter().position(|o| o == id).unwrap_or(usize::MAX));
            let providers = self.providers.borrow();
            for (pid, name, members) in &groups {
                let root = c.shell.projects.iter().find(|p| p.id == *pid).map(|p| tilde(&p.workspace_root));
                let collapsed = cfg.collapsed.contains(pid) && !searching;
                let app = self.clone();
                let toggle_id = pid.clone();
                let header = sidebar::project_header(name, root.as_deref(), collapsed, members.len(), move || {
                    app.toggle_section(&toggle_id);
                });
                let app = self.clone();
                sidebar::make_header_draggable(&header, pid, move |moved, before| app.reorder_project(&moved, &before));
                self.list.append(&header);
                rows.push(None);
                if collapsed {
                    continue;
                }
                for &i in members {
                    let t = &c.shell.threads[i];
                    let provider = providers.iter().find(|p| p.instance_id == t.provider_instance_id);
                    let driver = provider.map(|p| p.driver.clone()).unwrap_or_else(|| t.provider_instance_id.clone());
                    let accent = sidebar::accent_for(provider, &driver);
                    let clean = crate::transcript::title_from_prompt(&t.title);
                    let title = clean.as_deref().unwrap_or("Untitled thread");
                    let row = sidebar::thread_row(t, title, &accent, now);
                    sidebar::attach_menu(&row, sidebar::thread_menu(&t.id, sidebar::is_pinned(t)));
                    self.list.append(&row);
                    if selected_thread.as_deref() == Some(t.id.as_str()) {
                        select_at = Some(rows.len());
                    }
                    rows.push(Some(Row::Thread(i)));
                }
            }
        }

        // Offline transcripts: everything Claude Code wrote, grouped by project folder.
        let imported = self.imported_uuids();
        {
            let sessions = self.sessions.borrow();
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            for (i, s) in sessions.iter().enumerate() {
                let project = s.project();
                if !matches(&s.title, &project) {
                    continue;
                }
                match groups.iter_mut().find(|(p, _)| *p == project) {
                    Some((_, v)) => v.push(i),
                    None => groups.push((project, vec![i])),
                }
            }
            if !groups.is_empty() {
                let collapsed = cfg.collapsed.contains("offline") && !searching;
                let app = self.clone();
                let header = sidebar::project_header(
                    "Offline transcripts",
                    Some(&tilde(&index::projects_dir().to_string_lossy())),
                    collapsed,
                    groups.iter().map(|(_, m)| m.len()).sum(),
                    move || app.toggle_section("offline"),
                );
                self.list.append(&header);
                rows.push(None);
                if collapsed {
                    groups.clear();
                }
            }
            for (project, members) in &groups {
                let key = format!("offline:{project}");
                let root = sessions.get(members[0]).map(|s| tilde(&s.cwd));
                let collapsed = cfg.collapsed.contains(&key) && !searching;
                let app = self.clone();
                let toggle_key = key.clone();
                let header = sidebar::project_header(
                    &format!("  {project}"),
                    root.as_deref(),
                    collapsed,
                    members.len(),
                    move || {
                        app.toggle_section(&toggle_key);
                    },
                );
                self.list.append(&header);
                rows.push(None);
                if collapsed {
                    continue;
                }
                for &i in members {
                    let s = &sessions[i];
                    let row = sidebar::session_row(&s.title, s.mtime, imported.contains_key(&s.id), &s.id, now);
                    self.list.append(&row);
                    if selected_session.as_deref() == Some(s.id.as_str()) && selected_thread.is_none() {
                        select_at = Some(rows.len());
                    }
                    rows.push(Some(Row::Session(i)));
                }
            }
        }
        *self.rows.borrow_mut() = rows;
        match select_at.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
        // CLAUDE_PANTHEON_OPEN=<thread or session id> opens that chat as soon as its row exists (benchmarks).
        if !self.auto_opened.get()
            && let Ok(id) = std::env::var("CLAUDE_PANTHEON_OPEN")
            && self.select_id(&id)
        {
            self.auto_opened.set(true);
        }
    }

    fn toggle_section(self: &Rc<Self>, key: &str) {
        {
            let mut cfg = self.sidebar_cfg.borrow_mut();
            if !cfg.collapsed.remove(key) {
                cfg.collapsed.insert(key.to_string());
            }
            cfg.save();
        }
        self.rebuild_list();
    }

    fn reorder_project(self: &Rc<Self>, moved: &str, before: &str) {
        {
            let current: Vec<String> = self
                .conn
                .borrow()
                .as_ref()
                .map(|c| c.shell.projects.iter().map(|p| p.id.clone()).collect())
                .unwrap_or_default();
            let mut cfg = self.sidebar_cfg.borrow_mut();
            let ordered = cfg.order_projects(current);
            cfg.move_project(ordered, moved, Some(before));
            cfg.save();
        }
        self.rebuild_list();
    }

    /// The context-menu commands on a thread row.
    fn thread_command(self: &Rc<Self>, thread_id: &str, what: &str) {
        let (rpc, thread) = {
            let conn = self.conn.borrow();
            match conn.as_ref() {
                Some(c) => (c.rpc.clone(), c.shell.threads.iter().find(|t| t.id == thread_id).cloned()),
                None => return,
            }
        };
        let Some(thread) = thread else { return };
        let app = self.clone();
        let id = thread_id.to_string();
        match what {
            "pin" => {
                let kind = if sidebar::is_pinned(&thread) { "thread.unpin" } else { "thread.pin" };
                glib::spawn_future_local(async move {
                    let payload = json!({"type": kind, "commandId": new_id(), "threadId": id});
                    if let Err(e) = rpc.call("orchestration.dispatchCommand", payload).await {
                        app.toast(&format!("Could not pin: {}", e.message()));
                    }
                });
            }
            "archive" => {
                glib::spawn_future_local(async move {
                    let payload = json!({"type": "thread.archive", "commandId": new_id(), "threadId": id});
                    match rpc.call("orchestration.dispatchCommand", payload).await {
                        Ok(_) => app.toast("Archived."),
                        Err(e) => app.toast(&format!("Could not archive: {}", e.message())),
                    }
                });
            }
            "rename" => {
                let current = crate::transcript::title_from_prompt(&thread.title).unwrap_or_default();
                sidebar::rename_dialog(&self.window, &current, move |title| {
                    let app = app.clone();
                    let rpc = rpc.clone();
                    let id = id.clone();
                    glib::spawn_future_local(async move {
                        let payload = json!({"type": "thread.metadata.update", "commandId": new_id(), "threadId": id, "title": title});
                        if let Err(e) = rpc.call("orchestration.dispatchCommand", payload).await {
                            app.toast(&format!("Could not rename: {}", e.message()));
                        }
                    });
                });
            }
            "handoff" => {
                self.open_thread(thread);
                let app = self.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || app.handoff_to_repo());
            }
            "copy-link" => {
                self.window.clipboard().set_text(&format!("claude-pantheon-rs --thread '{id}'"));
                self.toast("Copied a command that opens this thread.");
            }
            _ => {}
        }
    }

    /// Opens a thread or session by id from outside (the Wingpanel indicator's `open-thread` action);
    /// a thread that is not in the list yet is retried once the shell arrives.
    pub fn open_by_id(self: &Rc<Self>, id: &str) {
        self.window.present();
        if self.select_id(id) {
            return;
        }
        if self.conn.borrow().is_none() {
            self.toast("Not connected to the server yet.");
            return;
        }
        let app = self.clone();
        let id = id.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            if !app.select_id(&id) {
                app.toast("That thread is not in the list.");
            }
        });
    }

    /// Collapses or expands a sidebar section (tests).
    pub fn set_section_collapsed(self: &Rc<Self>, key: &str, collapsed: bool) {
        {
            let mut cfg = self.sidebar_cfg.borrow_mut();
            if collapsed {
                cfg.collapsed.insert(key.to_string());
            } else {
                cfg.collapsed.remove(key);
            }
        }
        self.rebuild_list();
    }

    /// Visible sidebar rows (tests): true for thread or session rows, false for headers.
    pub fn row_count(&self) -> (usize, usize) {
        let rows = self.rows.borrow();
        (rows.iter().filter(|r| r.is_some()).count(), rows.iter().filter(|r| r.is_none()).count())
    }

    /// Selects a thread or session row by id (tests, and the offline/online twins).
    pub fn select_id(&self, id: &str) -> bool {
        let at = {
            let rows = self.rows.borrow();
            let sessions = self.sessions.borrow();
            let conn = self.conn.borrow();
            rows.iter().position(|r| match r {
                Some(Row::Session(i)) => sessions.get(*i).is_some_and(|s| s.id == id),
                Some(Row::Thread(i)) => conn.as_ref().and_then(|c| c.shell.threads.get(*i)).is_some_and(|t| t.id == id),
                None => false,
            })
        };
        match at.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                true
            }
            None => false,
        }
    }

    // ---- offline transcript ------------------------------------------------------------------

    /// Ctrl+Shift+T: the thread's terminal on the server, in a pane under the chat.
    pub fn toggle_terminal(self: &Rc<Self>) {
        let showing = self.terminal_revealer.reveals_child();
        if showing {
            self.terminal_revealer.set_reveal_child(false);
            self.terminal_pane.detach();
            self.entry.view.grab_focus();
            return;
        }
        let (rpc, thread_id, cwd) = {
            let conn = self.conn.borrow();
            let live = self.live.borrow();
            match (conn.as_ref(), live.as_ref()) {
                (Some(c), Some(l)) => {
                    let cwd = c.shell.projects.iter().find(|p| p.id == l.project_id).map(|p| p.workspace_root.clone());
                    (c.rpc.clone(), l.thread_id.clone(), cwd)
                }
                _ => {
                    self.toast("Open a live thread first; the terminal runs in its project on the server.");
                    return;
                }
            }
        };
        let Some(cwd) = cwd else {
            self.toast("This thread's project has no folder.");
            return;
        };
        self.terminal_revealer.set_reveal_child(true);
        self.terminal_pane.attach(rpc, thread_id, cwd);
        self.terminal_pane.grab_focus();
    }

    pub fn terminal_pane(&self) -> &Rc<terminal::TerminalPane> {
        &self.terminal_pane
    }

    fn leave_live(&self) {
        if self.terminal_revealer.reveals_child() {
            self.terminal_revealer.set_reveal_child(false);
            self.terminal_pane.detach();
        }
        let taken = self.live.borrow_mut().take();
        if let Some(live) = taken
            && let Some(c) = self.conn.borrow().as_ref()
        {
            c.rpc.interrupt(&live.sub_id);
        }
        *self.draft.borrow_mut() = None;
        self.stop_btn.set_visible(false);
    }

    pub fn open_session(self: &Rc<Self>, session: Session) {
        self.leave_live();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.current_session.borrow_mut() = Some(session.clone());
        self.title.set_text(&session.title);
        self.window.set_title(Some(&format!("{} - Claude", session.title)));
        self.subtitle.set_text(&format!("Offline transcript · {}", tilde(&session.cwd)));
        self.subtitle.set_visible(true);
        self.loading.set(true);
        if self.transcript.borrow().is_none() {
            self.show_page("busy");
        }
        let twin = self.imported_uuids().get(&session.id).cloned();
        self.entry.set_draft_key(None);
        self.composer.set_visible(false);
        self.composer_note.set_text(match twin {
            Some(_) => "Imported on the server: open it from the project above to continue.",
            None => "Offline transcript. Import it to continue it.",
        });
        let app = self.clone();
        let path = session.path.clone();
        glib::spawn_future_local(async move {
            let loaded = gio::spawn_blocking(move || transcript::load(&path)).await;
            if app.generation.get() != generation {
                return;
            }
            app.loading.set(false);
            match loaded {
                Ok(Ok(t)) => app.show_transcript(Rc::new(t)),
                Ok(Err(e)) => app.toast(&format!("Could not read the chat: {e}")),
                Err(_) => app.toast("Could not read the chat"),
            }
        });
    }

    fn clear_chat(&self) {
        while let Some(child) = self.chat.first_child() {
            self.chat.remove(&child);
        }
    }

    fn show_transcript(self: &Rc<Self>, t: Rc<Transcript>) {
        self.clear_chat();
        let total = t.turns.len();
        let from = total.saturating_sub(TAIL_TURNS);
        if from > 0 {
            let more = gtk::Button::with_label(&format!("Show {from} earlier messages"));
            more.set_halign(gtk::Align::Center);
            more.add_css_class("flat");
            let app = self.clone();
            let t2 = t.clone();
            more.connect_clicked(move |b| {
                b.set_visible(false);
                let mut after: Option<gtk::Widget> = Some(b.clone().upcast());
                for turn in t2.turns.iter().take(from) {
                    let w = chat::turn_widget(turn);
                    app.chat.insert_child_after(&w, after.as_ref());
                    after = Some(w);
                }
            });
            self.chat.append(&more);
        }
        for turn in &t.turns[from..] {
            self.chat.append(&chat::turn_widget(turn));
        }
        if t.bad_lines > 0 {
            let note = label(&format!("{} malformed lines were skipped", t.bad_lines), &["caption", "dim-label"]);
            note.set_halign(gtk::Align::Center);
            self.chat.append(&note);
        }
        *self.transcript.borrow_mut() = Some(t);
        self.show_page("chat");
        self.scroll_to_end();
    }

    pub fn scroll_to_end(&self) {
        let adj = self.chat_scroll.vadjustment();
        glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
    }

    /// From an imported thread to its JSONL file, or from a JSONL session to its imported thread.
    fn open_offline_twin(self: &Rc<Self>) {
        let live_id = self.live.borrow().as_ref().map(|l| l.thread_id.clone());
        if let Some(id) = live_id {
            if let Some(uuid) = id.strip_prefix("import:claudeAgent:") {
                if !self.select_id(uuid) {
                    self.toast("No offline transcript for this thread on this machine.");
                }
            } else {
                self.toast("This thread was not imported from Claude Code.");
            }
            return;
        }
        let session = self.current_session.borrow().clone();
        if let Some(s) = session {
            let twin = self.imported_uuids().get(&s.id).cloned();
            match twin {
                Some(tid) => {
                    self.select_id(&tid);
                }
                None => self.toast("Not imported yet. Use Import Sessions from the menu."),
            }
        }
    }

    // ---- live thread -------------------------------------------------------------------------

    pub fn open_thread(self: &Rc<Self>, thread: ThreadShell) {
        if let Some(application) = self.window.application() {
            notify::withdraw(&application, &format!("turn:{}", thread.id));
            notify::withdraw(&application, &format!("ask:{}", thread.id));
        }
        self.leave_live();
        *self.current_session.borrow_mut() = None;
        *self.transcript.borrow_mut() = None;
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => return,
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let title = if thread.title.trim().is_empty() { "Untitled thread".to_string() } else { thread.title.clone() };
        self.title.set_text(&title);
        self.window.set_title(Some(&format!("{title} - Claude")));
        self.subtitle.set_visible(true);
        self.subtitle.set_text(&self.thread_subtitle(&thread));
        self.show_page("busy");
        self.composer.set_visible(true);
        self.composer_note.set_text("");
        self.select_provider(&thread.provider_instance_id, thread.model_selection.get("model").and_then(Value::as_str));
        self.select_mode(&thread.runtime_mode);
        self.entry.set_draft_key(Some(thread.id.clone()));
        {
            let rpc = rpc.clone();
            let id = thread.id.clone();
            glib::spawn_future_local(async move {
                let payload = json!({"type": "thread.visit", "commandId": new_id(), "threadId": id, "visitedAt": workflow::now_iso()});
                let _ = rpc.call("orchestration.dispatchCommand", payload).await;
            });
        }
        let (sub_id, mut stream) = rpc.subscribe(
            "orchestration.subscribeThread",
            json!({"threadId": thread.id, "requestCompletionMarker": true, "acceptBoundedSnapshot": true, "acceptCompactTurnItems": true}),
        );
        *self.live.borrow_mut() = Some(Live {
            thread_id: thread.id.clone(),
            project_id: thread.project_id.clone(),
            sub_id,
            view: ThreadView::default(),
            widgets: HashMap::new(),
            running: thread.is_running(),
        });
        self.stop_btn.set_visible(thread.is_running());
        let app = self.clone();
        glib::spawn_future_local(async move {
            while let Some(item) = stream.next().await {
                if app.generation.get() != generation {
                    break;
                }
                match item {
                    Ok(v) => app.thread_event(&v),
                    Err(e) => {
                        app.toast(&format!("Thread stream ended: {e}"));
                        break;
                    }
                }
            }
        });
    }

    fn thread_subtitle(&self, thread: &ThreadShell) -> String {
        let providers = self.providers.borrow();
        let provider = providers
            .iter()
            .find(|p| p.instance_id == thread.provider_instance_id)
            .map(Provider::name)
            .unwrap_or_else(|| thread.provider_instance_id.clone());
        let model = thread.model_selection.get("model").and_then(Value::as_str).unwrap_or("");
        let mode = RUNTIME_MODES
            .iter()
            .find(|(k, _)| *k == thread.runtime_mode)
            .map(|(_, l)| *l)
            .unwrap_or(&thread.runtime_mode);
        let project = self
            .conn
            .borrow()
            .as_ref()
            .and_then(|c| c.shell.projects.iter().find(|p| p.id == thread.project_id).map(|p| tilde(&p.workspace_root)))
            .unwrap_or_default();
        format!("{provider} · {model} · {mode} · {project}")
    }

    fn thread_event(self: &Rc<Self>, v: &Value) {
        let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
        match kind {
            "snapshot" => {
                let view = ThreadView::from_projection(&v["projection"]);
                let running = view.active_run_id.is_some();
                if let Some(live) = self.live.borrow_mut().as_mut() {
                    live.view = view;
                    live.running = running;
                }
                self.render_live();
                if v.get("hasMoreHistory").and_then(Value::as_bool).unwrap_or(false) {
                    let note = label(
                        "Earlier history stays on the server; this is the latest window.",
                        &["caption", "dim-label"],
                    );
                    note.set_halign(gtk::Align::Center);
                    self.chat.prepend(&note);
                }
                self.show_page("chat");
                self.stop_btn.set_visible(running);
                self.scroll_to_end();
                self.show_issue_comments();
            }
            "event" => {
                let event = &v["event"];
                let etype = event.get("type").and_then(Value::as_str).unwrap_or("");
                let payload = &event["payload"];
                match etype {
                    "turn-item.updated" => {
                        if let Ok(item) = serde_json::from_value::<TurnItem>(payload.clone()) {
                            self.update_item(item);
                        }
                    }
                    "run.created" | "run.updated" => {
                        let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
                        let running = matches!(status, "running" | "pending" | "queued" | "starting" | "preparing");
                        let mut finished = false;
                        if let Some(live) = self.live.borrow_mut().as_mut() {
                            finished = live.running && !running;
                            live.running = running;
                            live.view.active_run_id = if running {
                                payload.get("id").and_then(Value::as_str).map(str::to_string)
                            } else {
                                None
                            };
                        }
                        self.stop_btn.set_visible(running);
                        if finished {
                            self.run_finished();
                        }
                    }
                    "checkpoint.captured" => {
                        if let Some(live) = self.live.borrow_mut().as_mut() {
                            live.view.checkpoints.push(payload.clone());
                        }
                    }
                    "thread.updated" | "thread.created" => {
                        if let Some(t) = payload.get("title").and_then(Value::as_str) {
                            self.title.set_text(t);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn decide_callback(self: &Rc<Self>) -> chat::Decide {
        let app = self.clone();
        Rc::new(move |request_id: &str, decision: &str| app.answer_request(request_id, decision))
    }

    /// Answers a permission prompt; a `decline` may carry a reason after a newline, which is sent to
    /// the thread as a steering message so the agent knows why.
    pub fn answer_request(self: &Rc<Self>, request_id: &str, decision: &str) {
        let (rpc, thread_id, run_id) = {
            let conn = self.conn.borrow();
            let live = self.live.borrow();
            match (conn.as_ref(), live.as_ref()) {
                (Some(c), Some(l)) => (c.rpc.clone(), l.thread_id.clone(), l.view.active_run_id.clone()),
                _ => return,
            }
        };
        let (decision, note) = match decision.split_once('\n') {
            Some((d, n)) => (d.to_string(), n.trim().to_string()),
            None => (decision.to_string(), String::new()),
        };
        let payload = json!({
            "type": "runtime-request.respond", "commandId": new_id(), "threadId": thread_id,
            "requestId": request_id, "decision": decision,
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = rpc.call("orchestration.dispatchCommand", payload).await {
                app.toast(&format!("Could not answer: {}", e.message()));
                return;
            }
            if !note.is_empty() {
                let dispatch_mode = match run_id {
                    Some(run) => json!({"type": "steer_active", "targetRunId": run}),
                    None => json!({"type": "start_immediately"}),
                };
                let message = json!({
                    "type": "message.dispatch", "commandId": new_id(), "threadId": thread_id, "messageId": new_id(),
                    "text": format!("Denied: {note}"), "attachments": [], "createdBy": "user", "creationSource": "web",
                    "dispatchMode": dispatch_mode,
                });
                let _ = rpc.call("orchestration.dispatchCommand", message).await;
            }
            if let Some(application) = app.window.application() {
                notify::withdraw(
                    &application,
                    &format!("ask:{}", app.live.borrow().as_ref().map(|l| l.thread_id.clone()).unwrap_or_default()),
                );
            }
        });
    }

    /// The oldest pending prompt of the open thread, for the Y/N keys.
    fn oldest_pending_request(&self) -> Option<String> {
        self.live
            .borrow()
            .as_ref()
            .and_then(|l| l.view.items.iter().find(|i| i.is_pending_request()).and_then(|i| i.request_id.clone()))
    }

    fn render_live(self: &Rc<Self>) {
        self.clear_chat();
        let decide = self.decide_callback();
        let mut live = self.live.borrow_mut();
        let Some(live) = live.as_mut() else { return };
        live.widgets.clear();
        let items = live.view.items.clone();
        let from = items.len().saturating_sub(TAIL_TURNS);
        if from > 0 {
            let note = label(&format!("{from} earlier items folded"), &["caption", "dim-label"]);
            note.set_halign(gtk::Align::Center);
            self.chat.append(&note);
        }
        for item in &items[from..] {
            if let Some(w) = chat::item_widget(item, Some(decide.clone())) {
                self.chat.append(&w);
                live.widgets.insert(item.id.clone(), w);
            }
        }
    }

    /// Replaces or appends the widget of one item; the rest of the column stays put.
    fn update_item(self: &Rc<Self>, item: TurnItem) {
        let decide = self.decide_callback();
        let should_scroll = {
            let adj = self.chat_scroll.vadjustment();
            adj.value() + adj.page_size() >= adj.upper() - 80.0
        };
        {
            let mut live = self.live.borrow_mut();
            let Some(live) = live.as_mut() else { return };
            live.view.upsert(item.clone());
            if item.is_pending_request() {
                let body = item
                    .prompt
                    .clone()
                    .or(item.reason.clone())
                    .unwrap_or_else(|| "A permission prompt is waiting".into());
                let thread_id = live.thread_id.clone();
                let title = self.title.text().to_string();
                self.notify_background(
                    "ask",
                    &thread_id,
                    &format!("{title} needs your answer"),
                    &transcript::first_line(&body, 120),
                    true,
                );
            }
            let new_widget = chat::item_widget(&item, Some(decide));
            match live.widgets.remove(&item.id) {
                Some(old) => {
                    if let Some(w) = new_widget {
                        self.chat.insert_child_after(&w, Some(&old));
                        live.widgets.insert(item.id.clone(), w);
                    }
                    self.chat.remove(&old);
                }
                None => {
                    if let Some(w) = new_widget {
                        self.chat.append(&w);
                        live.widgets.insert(item.id.clone(), w);
                    }
                }
            }
        }
        if should_scroll {
            self.scroll_to_end();
        }
    }

    fn run_finished(self: &Rc<Self>) {
        if let Some(thread_id) = self.live.borrow().as_ref().map(|l| l.thread_id.clone()) {
            let last = self.live.borrow().as_ref().and_then(|l| {
                l.view
                    .items
                    .iter()
                    .rev()
                    .find(|i| i.kind == "assistant_message")
                    .map(|i| transcript::first_line(&i.text, 120))
            });
            let title = self.title.text().to_string();
            self.notify_background(
                "turn",
                &thread_id,
                &format!("{title}: turn finished"),
                &last.unwrap_or_default(),
                false,
            );
        }
        self.refresh_launcher();
        let pending = self.handoff.borrow_mut().take();
        if let Some(p) = pending {
            let text = self.live.borrow().as_ref().and_then(|l| {
                l.view
                    .items
                    .iter()
                    .rev()
                    .find(|i| i.kind == "assistant_message" && !i.streaming)
                    .map(|i| i.text.clone())
            });
            if let Some(text) = text {
                handoff::finish(self, p, text);
            }
        }
    }

    // ---- composer --------------------------------------------------------------------------

    fn fill_providers(&self) {
        let providers = self.providers.borrow();
        let names: Vec<String> = providers
            .iter()
            .map(|p| {
                let state = if p.ready() {
                    "ready".to_string()
                } else if !p.enabled {
                    "disabled".to_string()
                } else if !p.installed {
                    "not installed".to_string()
                } else {
                    p.auth_text()
                };
                format!("{} · {}", p.name(), state)
            })
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.provider_dd.set_model(Some(&gtk::StringList::new(&refs)));
        if let Some(i) = providers.iter().position(Provider::ready) {
            self.provider_dd.set_selected(i as u32);
        }
        drop(providers);
        self.fill_models();
    }

    fn fill_models(&self) {
        let providers = self.providers.borrow();
        let Some(p) = providers.get(self.provider_dd.selected() as usize) else {
            self.model_dd.set_model(Some(&gtk::StringList::new(&[])));
            return;
        };
        let names: Vec<&str> = p.models.iter().map(|m| m.display()).collect();
        self.model_dd.set_model(Some(&gtk::StringList::new(&names)));
        if let Some(i) = p.models.iter().position(|m| m.is_default == Some(true)) {
            self.model_dd.set_selected(i as u32);
        }
    }

    fn select_provider(&self, instance_id: &str, model: Option<&str>) {
        let index = self.providers.borrow().iter().position(|p| p.instance_id == instance_id);
        let Some(i) = index else { return };
        self.provider_dd.set_selected(i as u32);
        self.fill_models();
        if let Some(model) = model {
            let providers = self.providers.borrow();
            let found = providers[i]
                .models
                .iter()
                .position(|m| m.key() == model || m.aliases.as_ref().is_some_and(|a| a.iter().any(|x| x == model)));
            if let Some(mi) = found {
                self.model_dd.set_selected(mi as u32);
            }
        }
    }

    fn select_mode(&self, mode: &str) {
        if let Some(i) = RUNTIME_MODES.iter().position(|(k, _)| *k == mode) {
            self.mode_dd.set_selected(i as u32);
        }
    }

    fn selection(&self) -> Option<(Provider, String, String)> {
        let providers = self.providers.borrow();
        let p = providers.get(self.provider_dd.selected() as usize)?.clone();
        let model = p.models.get(self.model_dd.selected() as usize).map(|m| m.key().to_string()).unwrap_or_default();
        let mode = RUNTIME_MODES
            .get(self.mode_dd.selected() as usize)
            .map(|(k, _)| k.to_string())
            .unwrap_or_else(|| "full-access".into());
        Some((p, model, mode))
    }

    pub fn new_chat(self: &Rc<Self>) {
        self.new_chat_in(None);
    }

    /// A new thread in `project_id`, else in the project of what is open, else the first project.
    pub fn new_chat_in(self: &Rc<Self>, project_id: Option<String>) {
        if self.conn.borrow().is_none() {
            self.toast("Connect to a T3 server to start a thread.");
            self.show_page("pair");
            return;
        }
        // The project of the open thread, else of the open transcript's folder, else the first project.
        let project = {
            let conn = self.conn.borrow();
            let c = conn.as_ref().unwrap();
            let by_arg = project_id.as_ref().and_then(|id| c.shell.projects.iter().find(|p| p.id == *id).cloned());
            let by_live = self
                .live
                .borrow()
                .as_ref()
                .and_then(|l| c.shell.projects.iter().find(|p| p.id == l.project_id).cloned());
            let by_session = self
                .current_session
                .borrow()
                .as_ref()
                .and_then(|s| c.shell.projects.iter().find(|p| s.cwd.starts_with(&p.workspace_root)).cloned());
            by_arg.or(by_live).or(by_session).or_else(|| c.shell.projects.first().cloned())
        };
        let Some(project) = project else {
            self.toast("No project on the server yet. Importing your sessions first.");
            self.import_sessions();
            return;
        };
        self.leave_live();
        *self.current_session.borrow_mut() = None;
        *self.transcript.borrow_mut() = None;
        *self.draft.borrow_mut() = Some(Draft { project_id: project.id.clone() });
        self.entry.set_draft_key(Some(format!("draft:{}", project.id)));
        self.list.unselect_all();
        self.clear_chat();
        let hint = granite::Placeholder::new("New thread");
        hint.set_description(&format!(
            "In {}. Pick a provider, model and permission mode, then write your first message.",
            tilde(&project.workspace_root)
        ));
        hint.set_icon(&gio::ThemedIcon::new("mail-message-new-symbolic"));
        hint.set_vexpand(true);
        self.chat.append(&hint);
        self.title.set_text("New thread");
        self.window.set_title(Some("New thread - Claude"));
        self.subtitle.set_text(&tilde(&project.workspace_root));
        self.subtitle.set_visible(true);
        self.composer.set_visible(true);
        self.composer_note.set_text("");
        self.show_page("chat");
        self.entry.view.grab_focus();
    }

    pub fn send(self: &Rc<Self>) {
        let text = self.entry.text().trim().to_string();
        if text.is_empty() && self.entry.attachment_count() == 0 {
            return;
        }
        let text = if text.is_empty() { "See the attached files.".to_string() } else { text };
        self.send_text(text, true);
    }

    /// Sends `text` to the open thread, or launches the drafted thread with it.
    pub fn send_text(self: &Rc<Self>, text: String, clear: bool) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Not connected.");
                return;
            }
        };
        let Some((provider, model, mode)) = self.selection() else { return };
        if !provider.ready() {
            self.toast(&format!("{} is not ready on the server ({}).", provider.name(), provider.auth_text()));
            return;
        }
        let attachments: Vec<Value> = if clear { self.entry.attachments_json() } else { Vec::new() };
        if clear {
            self.entry.clear();
        }
        let live_thread =
            self.live.borrow().as_ref().map(|l| (l.thread_id.clone(), l.running, l.view.active_run_id.clone()));
        let draft = self.draft.borrow().clone();
        let app = self.clone();
        glib::spawn_future_local(async move {
            if let Some((thread_id, running, run_id)) = live_thread {
                let dispatch_mode = match (running, run_id) {
                    (true, Some(run)) => json!({"type": "steer_active", "targetRunId": run}),
                    (true, None) => json!({"type": "queue_after_active"}),
                    _ => json!({"type": "start_immediately"}),
                };
                let payload = json!({
                    "type": "message.dispatch", "commandId": new_id(), "threadId": thread_id, "messageId": new_id(),
                    "text": text, "attachments": attachments, "createdBy": "user", "creationSource": "web",
                    "modelSelection": {"instanceId": provider.instance_id, "model": model},
                    "dispatchMode": dispatch_mode,
                });
                match rpc.call("orchestration.dispatchCommand", payload).await {
                    Ok(_) => {
                        app.stop_btn.set_visible(true);
                        if let Some(l) = app.live.borrow_mut().as_mut() {
                            l.running = true;
                        }
                    }
                    Err(e) => app.toast(&format!("Send failed: {}", e.message())),
                }
            } else if let Some(d) = draft {
                let title = transcript::first_line(&text, 60);
                let payload = json!({
                    "commandId": new_id(), "projectId": d.project_id, "title": title, "generateTitle": true,
                    "creationSource": "web",
                    "modelSelection": {"instanceId": provider.instance_id, "model": model},
                    "runtimeMode": mode, "interactionMode": "default",
                    "workspaceStrategy": {"type": "root"},
                    "initialMessage": {"messageId": new_id(), "text": text, "attachments": attachments},
                });
                match rpc.call("orchestration.launchThread", payload).await {
                    Ok(v) => {
                        let thread_id = v.get("threadId").and_then(Value::as_str).unwrap_or("").to_string();
                        let shell = ThreadShell {
                            id: thread_id,
                            project_id: d.project_id.clone(),
                            title,
                            provider_instance_id: provider.instance_id.clone(),
                            model_selection: json!({"instanceId": provider.instance_id, "model": model}),
                            runtime_mode: mode,
                            interaction_mode: "default".into(),
                            ..Default::default()
                        };
                        *app.draft.borrow_mut() = None;
                        app.open_thread(shell);
                    }
                    Err(e) => app.toast(&format!("Could not start the thread: {}", e.message())),
                }
            } else {
                app.toast("Open a thread or start a new one first.");
            }
        });
    }

    pub fn stop(self: &Rc<Self>) {
        let (rpc, thread_id) = {
            let conn = self.conn.borrow();
            let live = self.live.borrow();
            match (conn.as_ref(), live.as_ref()) {
                (Some(c), Some(l)) => (c.rpc.clone(), l.thread_id.clone()),
                _ => return,
            }
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let payload =
                json!({"type": "thread.stop", "commandId": new_id(), "threadId": thread_id, "reason": "user"});
            match rpc.call("orchestration.dispatchCommand", payload).await {
                Ok(_) => app.toast("Stopping…"),
                Err(e) => app.toast(&format!("Stop failed: {}", e.message())),
            }
        });
    }

    // ---- menu actions ------------------------------------------------------------------------

    /// Creates projects for every folder with Claude Code or Codex sessions and imports them.
    pub fn import_sessions(self: &Rc<Self>) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Not connected.");
                return;
            }
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            app.toast("Scanning for sessions…");
            let scan = match rpc.call("agentSessions.scan", json!({})).await {
                Ok(v) => v,
                Err(e) => {
                    app.toast(&format!("Scan failed: {}", e.message()));
                    return;
                }
            };
            let candidates = scan.get("candidates").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut imported = 0u64;
            let mut projects_made = 0u64;
            for c in candidates {
                let path = c.get("path").and_then(Value::as_str).unwrap_or("").to_string();
                let title = c.get("title").and_then(Value::as_str).unwrap_or("Project").to_string();
                if path.is_empty()
                    || path.starts_with("/tmp/")
                    || c.get("threadCount").and_then(Value::as_u64).unwrap_or(0) == 0
                {
                    continue;
                }
                let existing = app
                    .conn
                    .borrow()
                    .as_ref()
                    .and_then(|c| c.shell.projects.iter().find(|p| p.workspace_root == path).map(|p| p.id.clone()));
                let project_id = match existing
                    .or_else(|| c.get("projectId").and_then(Value::as_str).map(str::to_string))
                {
                    Some(id) => id,
                    None => {
                        let id = new_id();
                        let payload = json!({"type": "project.create", "commandId": new_id(), "projectId": id, "title": title, "workspaceRoot": path});
                        match rpc.call("projects.mutate", payload).await {
                            Ok(_) => {
                                projects_made += 1;
                                id
                            }
                            Err(e) => {
                                app.toast(&format!("Could not add {title}: {}", e.message()));
                                continue;
                            }
                        }
                    }
                };
                if c.get("alreadyImported").and_then(Value::as_bool).unwrap_or(false) {
                    continue;
                }
                match rpc
                    .call("agentSessions.import", json!({"projectId": project_id, "expectedWorkspaceRoot": path}))
                    .await
                {
                    Ok(v) => imported += v.get("importedCount").and_then(Value::as_u64).unwrap_or(0),
                    Err(e) => app.toast(&format!("Import of {title} failed: {}", e.message())),
                }
            }
            app.toast(&format!("Imported {imported} sessions ({projects_made} new projects)."));
            app.schedule_rebuild();
        });
    }

    pub fn show_usage(self: &Rc<Self>) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Connect to a T3 server to see usage.");
                return;
            }
        };
        self.leave_live();
        self.list.unselect_all();
        self.title.set_text("Usage");
        self.window.set_title(Some("Usage - Claude"));
        self.subtitle.set_text("Tokens and cost across providers, read by the server from local history");
        self.subtitle.set_visible(true);
        self.show_page("usage");
        self.usage_page.load(self.clone(), rpc);
    }

    // ---- repos, issues, runner -----------------------------------------------------------------

    pub fn show_repos(self: &Rc<Self>) {
        self.leave_live();
        self.list.unselect_all();
        self.title.set_text("Repos");
        self.window.set_title(Some("Repos - Claude"));
        self.subtitle.set_text("Pinned GitHub repos, their issue backlog, and the runner");
        self.subtitle.set_visible(true);
        self.show_page("repos");
        self.repos_page.wire_filters(self.clone());
        self.repos_page.load(self.clone());
    }

    /// The server project for a checkout, created when missing.
    async fn project_for_path(&self, path: &str, title: &str) -> Option<String> {
        let existing = self.projects().iter().find(|p| p.workspace_root == path).map(|p| p.id.clone());
        if existing.is_some() {
            return existing;
        }
        let rpc = self.conn.borrow().as_ref().map(|c| c.rpc.clone())?;
        let id = new_id();
        let payload = json!({"type": "project.create", "commandId": new_id(), "projectId": id, "title": title, "workspaceRoot": path});
        rpc.call("projects.mutate", payload).await.ok().map(|_| id)
    }

    /// "Start on this": a thread on the composer's provider, model and permission mode, the issue
    /// labelled agent:running.
    pub fn start_issue(self: &Rc<Self>, repo: PinnedRepo, issue: Issue) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Not connected.");
                return;
            }
        };
        let Some((provider, model, mode)) = self.selection() else { return };
        if !provider.ready() {
            self.toast(&format!("{} is not ready on the server.", provider.name()));
            return;
        }
        let app = self.clone();
        glib::spawn_future_local(async move {
            let name = repo.repo.rsplit('/').next().unwrap_or(&repo.repo).to_string();
            let Some(project_id) = app.project_for_path(&repo.path, &name).await else {
                app.toast("Could not create the project on the server.");
                return;
            };
            let title = format!("#{} {}", issue.number, transcript::first_line(&issue.title, 60));
            let payload = json!({
                "commandId": new_id(), "projectId": project_id, "title": title, "generateTitle": false,
                "creationSource": "web",
                "modelSelection": {"instanceId": provider.instance_id, "model": model},
                "runtimeMode": mode, "interactionMode": "default",
                "workspaceStrategy": {"type": "root"},
                "initialMessage": {"messageId": new_id(), "text": workflow::issue_prompt(&repo.repo, &issue), "attachments": []},
            });
            match rpc.call("orchestration.launchThread", payload).await {
                Ok(v) => {
                    let thread_id = v.get("threadId").and_then(Value::as_str).unwrap_or("").to_string();
                    let key = format!("{}#{}", repo.repo, issue.number);
                    app.repos_page.remember_started(&key, &thread_id);
                    let _ = workflow::remove_label(&repo.repo, issue.number, "agent:ready").await;
                    if let Err(e) = workflow::add_label(&repo.repo, issue.number, "agent:running").await {
                        app.toast(&format!("Thread started; could not label the issue: {e}"));
                    }
                    let shell = ThreadShell {
                        id: thread_id,
                        project_id,
                        title,
                        provider_instance_id: provider.instance_id.clone(),
                        model_selection: json!({"instanceId": provider.instance_id, "model": model}),
                        runtime_mode: mode,
                        interaction_mode: "default".into(),
                        ..Default::default()
                    };
                    app.open_thread(shell);
                }
                Err(e) => app.toast(&format!("Could not start the thread: {}", e.message())),
            }
        });
    }

    /// Creates (or re-points) the repo's runner: a T3 scheduled task every five minutes on the provider
    /// with the emptiest usage window right now.
    pub fn install_runner(self: &Rc<Self>, repo: PinnedRepo) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Not connected.");
                return;
            }
        };
        let Some((provider, model, reason)) = workflow::runner_selection(&self.providers()) else {
            self.toast("Every provider's window is full; try again after the earliest reset.");
            return;
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let name = repo.repo.rsplit('/').next().unwrap_or(&repo.repo).to_string();
            let Some(project_id) = app.project_for_path(&repo.path, &name).await else {
                app.toast("Could not create the project on the server.");
                return;
            };
            workflow::ensure_labels(&repo.repo).await;
            let existing = app.repos_page.runner_id(&repo.repo);
            let payload = workflow::runner_task(&repo.repo, &project_id, &provider, &model, existing.as_deref());
            match rpc.call("scheduledTasks.upsert", payload).await {
                Ok(v) => {
                    let id = v
                        .get("task")
                        .and_then(|t| t.get("id"))
                        .or_else(|| v.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !id.is_empty() {
                        app.repos_page.remember_runner(&repo.repo, &id);
                    }
                    app.toast(&format!("Runner for {} installed on {} ({reason})", repo.repo, provider.name()));
                }
                Err(e) => app.toast(&format!("Could not install the runner: {}", e.message())),
            }
        });
    }

    /// Comments of the issue a thread was started from, as a collapsed row at the top of the chat.
    fn show_issue_comments(self: &Rc<Self>) {
        let thread_id = match self.live.borrow().as_ref() {
            Some(l) => l.thread_id.clone(),
            None => return,
        };
        let Some(key) = self.repos_page.issue_for_thread(&thread_id) else { return };
        let Some((repo, number)) = key.rsplit_once('#') else { return };
        let (repo, number) = (repo.to_string(), number.parse::<u64>().unwrap_or(0));
        let app = self.clone();
        let generation = self.generation.get();
        glib::spawn_future_local(async move {
            let comments = workflow::comments(&repo, number).await.unwrap_or_default();
            if app.generation.get() != generation {
                return;
            }
            app.chat.prepend(&repos::comments_row(&key, &comments));
        });
    }

    // ---- changes and diagnostics ------------------------------------------------------------------

    /// The full diff of the open thread's checkpoints, one collapsed row per file.
    /// The Changes page: file tree, inline or side-by-side diff, per-hunk copy, restore to checkpoint.
    pub fn show_changes(self: &Rc<Self>) {
        let (rpc, thread_id, turns, checkpoints) = {
            let conn = self.conn.borrow();
            let live = self.live.borrow();
            match (conn.as_ref(), live.as_ref()) {
                (Some(c), Some(l)) => {
                    (c.rpc.clone(), l.thread_id.clone(), l.view.runs.len(), l.view.checkpoints.clone())
                }
                _ => {
                    self.toast("Open a live thread first.");
                    return;
                }
            }
        };
        let app = self.clone();
        let tid = thread_id.clone();
        let rpc2 = rpc.clone();
        self.changes_page.on_restore(Rc::new(move |scope, checkpoint| {
            let app = app.clone();
            let rpc = rpc2.clone();
            let payload = json!({
                "type": "checkpoint.rollback", "restoreFiles": true, "commandId": new_id(), "threadId": tid,
                "scopeId": scope, "checkpointId": checkpoint,
            });
            glib::spawn_future_local(async move {
                match rpc.call("orchestration.dispatchCommand", payload).await {
                    Ok(_) => {
                        app.toast("Restoring files to the checkpoint…");
                        app.show_changes();
                    }
                    Err(e) => app.toast(&format!("Restore failed: {}", e.message())),
                }
            });
        }));
        self.show_page("changes");
        self.changes_page.load(rpc, thread_id, turns, checkpoints);
    }

    pub fn changes_page(&self) -> &Rc<changes::ChangesPage> {
        &self.changes_page
    }

    pub fn show_diagnostics(self: &Rc<Self>) {
        let rpc = match self.conn.borrow().as_ref() {
            Some(c) => c.rpc.clone(),
            None => {
                self.toast("Not connected.");
                return;
            }
        };
        self.leave_live();
        self.list.unselect_all();
        self.title.set_text("Diagnostics");
        self.window.set_title(Some("Diagnostics - Claude"));
        self.subtitle.set_text("The server's resource telemetry: it samples only while this page is open");
        self.subtitle.set_visible(true);
        self.show_page("diagnostics");
        self.diagnostics_page.start(self.clone(), rpc);
    }

    pub fn usage_page(&self) -> &usage::UsagePage {
        &self.usage_page
    }

    /// Asks the thread's provider for a structured handoff and writes it into the repo's handoff doc.
    fn handoff_to_repo(self: &Rc<Self>) {
        let (thread_id, project_id) = match self.live.borrow().as_ref() {
            Some(l) => (l.thread_id.clone(), l.project_id.clone()),
            None => {
                self.toast("Open a thread first.");
                return;
            }
        };
        let root = self
            .conn
            .borrow()
            .as_ref()
            .and_then(|c| c.shell.projects.iter().find(|p| p.id == project_id).map(|p| p.workspace_root.clone()));
        let Some(root) = root else {
            self.toast("This thread's project has no folder.");
            return;
        };
        let doc = handoff::doc_path(std::path::Path::new(&root));
        *self.handoff.borrow_mut() = Some(handoff::Pending { thread_id, root: root.clone().into(), doc: doc.clone() });
        self.toast(&format!("Asking for a handoff summary; it goes to {}", tilde(&doc.to_string_lossy())));
        self.send_text(handoff::PROMPT.to_string(), false);
    }

    pub fn present_dialog(&self, dialog: &impl IsA<gtk::Window>) {
        dialog.set_transient_for(Some(&self.window));
        dialog.present();
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../data/style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
