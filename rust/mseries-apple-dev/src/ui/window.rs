//! The Apple Dev window: sidebar (Toolchain, Devices, projects), the selected page, and the
//! Activity terminal underneath. The header bar carries Build, Run and Stop for the selected
//! project and the phone that Run targets.

use super::console::{Console, Outcome};
use super::dialogs;
use super::widgets::{self, button, label, page, page_header, row, section, tilde};
use crate::commands::{self, Action, After, Job, Section};
use crate::devices::{self, Device};
use crate::distro;
use crate::project::{self, Kind, Project};
use crate::settings::{Settings, home};
use crate::testflight::{self, Builds, Level};
use crate::toolkit::{self, CheckId, CheckState, Report};
use granite::prelude::{PlaceholderExt, ToastExt};
use gtk::{gdk, gio, glib, pango, prelude::*};
use mseries_pantheon_kit::{DockBadge, Polling, set_accessible_label, set_box_spacing_text_units};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

type WindowAction = Box<dyn Fn(&Rc<AppWindow>)>;
type DeviceJob = fn(&Path, &str) -> Job;

pub const APP_ID: &str = "org.techmore.mseries.AppleDev";

const CSS: &str = r#"
.apple-dev-console-bar { min-height: 2.2em; }
.apple-dev-badge { padding: 0 0.55em; border-radius: 999px; font-size: 0.85em; font-weight: 600;
                   background: alpha(@warning_color, 0.2); color: @warning_color; }
.apple-dev-count { padding: 0 0.55em; border-radius: 999px; font-size: 0.85em; font-weight: 600;
                   background: alpha(currentColor, 0.12); }
.apple-dev-ok { color: @success_color; }
.apple-dev-warn { color: @warning_color; }
.apple-dev-error { color: @error_color; }
.apple-dev-target { font-weight: 600; }
.apple-dev-sidebar-footer { padding: 0.35em; }
.apple-dev-console vte-terminal { padding: 0.35em 0.75em; }
"#;

/// App Store Connect state of a project's builds, shown after an upload.
#[derive(Debug, Clone)]
enum TestFlightView {
    Checking,
    Done { result: Result<Builds, String>, checked: SystemTime },
}

/// While Apple processes a build, ask again this often, at most this many times (20 minutes).
const TESTFLIGHT_POLL: Duration = Duration::from_secs(30);
const TESTFLIGHT_POLLS: u32 = 40;

#[derive(Debug, Clone, PartialEq)]
enum Side {
    Toolchain,
    Devices,
    Project(PathBuf),
    Other,
}

impl Side {
    fn key(&self) -> String {
        match self {
            Side::Toolchain => "toolchain".into(),
            Side::Devices => "devices".into(),
            Side::Project(p) => format!("project:{}", p.display()),
            Side::Other => String::new(),
        }
    }

    fn from_key(key: &str) -> Side {
        match key {
            "devices" => Side::Devices,
            k if k.starts_with("project:") => Side::Project(PathBuf::from(&k["project:".len()..])),
            _ => Side::Toolchain,
        }
    }
}

pub struct AppWindow {
    app: gtk::Application,
    pub window: gtk::ApplicationWindow,
    settings: RefCell<Settings>,
    report: RefCell<Option<Report>>,
    probing: Cell<bool>,
    devices: RefCell<Vec<Device>>,
    scanning: Cell<bool>,
    target_udid: RefCell<Option<String>>,

    sidebar: gtk::ListBox,
    side: RefCell<Vec<Side>>,
    syncing: Cell<bool>,
    toolchain_badge: gtk::Label,
    toolchain_ok: gtk::Image,
    devices_badge: gtk::Label,

    stack: gtk::Stack,
    toolchain_page: gtk::Box,
    devices_list: gtk::Box,
    tailnet_entry: gtk::Entry,
    project_page: gtk::Box,
    project_scroll: gtk::ScrolledWindow,

    console: Rc<Console>,
    console_toggle: gtk::ToggleButton,
    vpaned: gtk::Paned,
    build_btn: gtk::Button,
    target_label: gtk::Label,
    device_model: gtk::StringList,
    device_dd: gtk::DropDown,
    syncing_devices: Cell<bool>,
    toast: granite::Toast,
    badge: DockBadge,
    unseen: Cell<u64>,
    poller: RefCell<Option<Polling>>,
    testflight: RefCell<HashMap<PathBuf, TestFlightView>>,
    /// Checks left for a project whose newest build is still processing.
    testflight_polls: RefCell<HashMap<PathBuf, u32>>,
}

impl AppWindow {
    pub fn new(app: &gtk::Application) -> Rc<AppWindow> {
        let settings = Settings::load();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Apple Dev")
            .icon_name(APP_ID)
            .default_width(settings.width.max(720))
            .default_height(settings.height.max(520))
            .build();
        if settings.maximized {
            window.maximize();
        }
        if let Some(display) = gdk::Display::default() {
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }

        // Header bar: Build / Run / Stop at the start, the target in the middle, menu at the end.
        let header = gtk::HeaderBar::new();
        let build_btn = icon_button("system-run-symbolic", "Build (Ctrl+B)", "Build");
        let run_btn = icon_button("media-playback-start-symbolic", "Run on iPhone (Ctrl+R)", "Run");
        let stop_btn = icon_button("media-playback-stop-symbolic", "Stop (Ctrl+.)", "Stop");
        build_btn.set_action_name(Some("win.build"));
        run_btn.set_action_name(Some("win.run"));
        stop_btn.set_action_name(Some("win.stop"));
        header.pack_start(&build_btn);
        header.pack_start(&run_btn);
        header.pack_start(&stop_btn);

        let target = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let target_label = gtk::Label::builder().label("No project").ellipsize(pango::EllipsizeMode::End).build();
        target_label.add_css_class("apple-dev-target");
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        chevron.add_css_class("dim-label");
        chevron.set_accessible_role(gtk::AccessibleRole::Presentation);
        let device_model = gtk::StringList::new(&["No iPhone"]);
        let device_dd = gtk::DropDown::new(Some(device_model.clone()), None::<gtk::Expression>);
        device_dd.set_tooltip_text(Some("The phone Run and Debug install on"));
        set_accessible_label(&device_dd, "Run target");
        device_dd.add_css_class("flat");
        target.append(&target_label);
        target.append(&chevron);
        target.append(&device_dd);
        header.set_title_widget(Some(&target));

        let menu = gio::Menu::new();
        let section_a = gio::Menu::new();
        section_a.append(Some("App Store Connect…"), Some("win.preferences"));
        section_a.append(Some("Check Toolchain Again"), Some("win.recheck"));
        section_a.append(Some("Show Toolkit Folder"), Some("win.show-toolkit"));
        menu.append_section(None, &section_a);
        let section_b = gio::Menu::new();
        section_b.append(Some("About Apple Dev"), Some("app.about"));
        section_b.append(Some("Quit"), Some("app.quit"));
        menu.append_section(None, &section_b);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).build();
        menu_btn.set_tooltip_text(Some("Menu"));
        set_accessible_label(&menu_btn, "Main menu");
        let console_toggle = gtk::ToggleButton::builder().icon_name("utilities-terminal-symbolic").build();
        console_toggle.set_tooltip_text(Some("Show Activity (Ctrl+`)"));
        set_accessible_label(&console_toggle, "Show the Activity terminal");
        header.pack_end(&menu_btn);
        header.pack_end(&console_toggle);
        window.set_titlebar(Some(&header));

        // Sidebar.
        let sidebar = gtk::ListBox::builder().css_classes(["navigation-sidebar"]).build();
        set_accessible_label(&sidebar, "Toolchain, devices and projects");
        let (toolchain_row, toolchain_badge, toolchain_ok) =
            side_row("applications-engineering-symbolic", "Toolchain", None);
        let (devices_row, devices_badge, _) = side_row("phone-symbolic", "Devices", None);
        devices_badge.remove_css_class("apple-dev-badge");
        devices_badge.add_css_class("apple-dev-count");
        sidebar.append(&toolchain_row);
        sidebar.append(&devices_row);
        sidebar.append(
            &gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .child(&granite::HeaderLabel::new("Projects"))
                .build(),
        );
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let add_menu = gio::Menu::new();
        add_menu.append(Some("New iPhone App…"), Some("win.new-app"));
        add_menu.append(Some("Add Existing Folder…"), Some("win.add-folder"));
        add_menu.append(Some("Import Xcode Project…"), Some("win.import-xcode"));
        let add = gtk::MenuButton::builder().icon_name("list-add-symbolic").menu_model(&add_menu).build();
        add.add_css_class("flat");
        add.set_tooltip_text(Some("Add a project"));
        set_accessible_label(&add, "Add a project");
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        footer.add_css_class("apple-dev-sidebar-footer");
        footer.append(&add);
        let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side_box.add_css_class("sidebar");
        side_box.append(&sidebar_scroll);
        side_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        side_box.append(&footer);
        side_box.set_width_request(220);

        // Pages.
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .hexpand(true)
            .vexpand(true)
            .build();
        let (toolchain_scroll, toolchain_page) = page();
        stack.add_named(&toolchain_scroll, Some("toolchain"));

        let (devices_scroll, devices_column) = page();
        let (devices_header, _, _, _, devices_trailing) =
            page_header("phone", "Devices", "iPhones and iPads this computer can install on.");
        let rescan = icon_button("view-refresh-symbolic", "Look for devices now", "Look for devices");
        devices_trailing.append(&rescan);
        devices_column.append(&devices_header);
        let devices_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        devices_column.append(&devices_list);
        let (tailnet_group, tailnet_rows) = section("Over Tailscale");
        let (address_row, address_trailing, _, _) = row(
            Some("network-wireless-symbolic"),
            "Phone's tailnet address",
            "iOS 27 with Wi-Fi and Tailscale on; tailscale status shows it as 100.x.y.z. Used by Install over Tailscale.",
        );
        let tailnet_entry = gtk::Entry::builder().placeholder_text("100.x.y.z").width_chars(16).build();
        set_accessible_label(&tailnet_entry, "Phone's tailnet address");
        address_trailing.append(&tailnet_entry);
        tailnet_rows.append(&address_row);
        let (pair_row, pair_trailing, _, _) = row(
            Some("emblem-shared-symbolic"),
            "Pair for Tailscale",
            "Once, with the phone on USB. Copy ~/.local/share/pymobiledevice3/remote_<udid>.plist to other hosts; it holds a private key, so treat it like an SSH key.",
        );
        let tailnet_pair = button("Pair…", &[]);
        pair_trailing.append(&tailnet_pair);
        tailnet_rows.append(&pair_row);
        devices_column.append(&tailnet_group);
        stack.add_named(&devices_scroll, Some("devices"));

        let (project_scroll, project_page) = page();
        stack.add_named(&project_scroll, Some("project"));

        let console = Console::new();
        let vpaned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&stack)
            .end_child(&console.root)
            .resize_start_child(true)
            .resize_end_child(false)
            .shrink_end_child(false)
            .build();
        console.root.set_visible(false);

        let hpaned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&side_box)
            .end_child(&vpaned)
            .resize_start_child(false)
            .shrink_start_child(false)
            .position(232)
            .build();

        let toast = granite::Toast::new("");
        toast.set_halign(gtk::Align::Center);
        toast.set_valign(gtk::Align::Start);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&hpaned));
        overlay.add_overlay(&toast);
        window.set_child(Some(&overlay));

        let w = Rc::new(AppWindow {
            app: app.clone(),
            window,
            settings: RefCell::new(settings),
            report: RefCell::new(None),
            probing: Cell::new(false),
            devices: RefCell::new(Vec::new()),
            scanning: Cell::new(false),
            target_udid: RefCell::new(None),
            sidebar,
            side: RefCell::new(vec![Side::Toolchain, Side::Devices, Side::Other]),
            syncing: Cell::new(false),
            toolchain_badge,
            toolchain_ok,
            devices_badge,
            stack,
            toolchain_page,
            devices_list,
            tailnet_entry,
            project_page,
            project_scroll,
            console,
            console_toggle,
            vpaned,
            build_btn,
            target_label,
            device_model,
            device_dd,
            syncing_devices: Cell::new(false),
            toast,
            badge: DockBadge::new(APP_ID),
            unseen: Cell::new(0),
            poller: RefCell::new(None),
            testflight: RefCell::new(HashMap::new()),
            testflight_polls: RefCell::new(HashMap::new()),
        });

        w.tailnet_entry.set_text(&w.settings.borrow().tailnet_host);
        w.install_actions();
        w.connect_signals(&rescan, &tailnet_pair);
        w.rebuild_projects();
        let selected = Side::from_key(&w.settings.borrow().selected.clone());
        w.select(selected);
        w.render_toolchain();
        w.render_devices();
        w.recheck();
        w.start_device_polling();
        w
    }

    // --- Wiring -------------------------------------------------------------------------------

    fn install_actions(self: &Rc<Self>) {
        let add = |name: &str, f: WindowAction| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            self.window.add_action(&action);
        };
        add("build", Box::new(|w| w.primary(true)));
        add("run", Box::new(|w| w.primary(false)));
        add("stop", Box::new(|w| w.console.stop()));
        add("new-app", Box::new(|w| w.new_app()));
        add("add-folder", Box::new(|w| w.choose_folder(false)));
        add("import-xcode", Box::new(|w| w.choose_folder(true)));
        add("preferences", Box::new(|w| w.preferences()));
        add("recheck", Box::new(|w| w.recheck()));
        add("show-toolkit", Box::new(|w| w.show_toolkit()));
        add("toggle-console", Box::new(|w| w.console_toggle.set_active(!w.console_toggle.is_active())));
        add(
            "focus-console",
            Box::new(|w| {
                w.show_console(true);
                w.console.focus_terminal();
            }),
        );
        for (action, accels) in [
            ("win.build", &["<Control>b"][..]),
            ("win.run", &["<Control>r"]),
            ("win.stop", &["<Control>period"]),
            ("win.new-app", &["<Control>n"]),
            ("win.add-folder", &["<Control>o"]),
            ("win.preferences", &["<Control>comma"]),
            ("win.toggle-console", &["<Control>grave"]),
        ] {
            self.app.set_accels_for_action(action, accels);
        }
        self.update_header();
    }

    fn connect_signals(self: &Rc<Self>, rescan: &gtk::Button, tailnet_pair: &gtk::Button) {
        let weak = Rc::downgrade(self);
        self.sidebar.connect_row_selected(move |_, row| {
            let (Some(w), Some(row)) = (weak.upgrade(), row) else { return };
            if w.syncing.get() {
                return;
            }
            let side = w.side.borrow().get(row.index() as usize).cloned();
            if let Some(side) = side.filter(|s| *s != Side::Other) {
                w.select(side);
            }
        });

        let weak = Rc::downgrade(self);
        self.console_toggle.connect_toggled(move |b| {
            if let Some(w) = weak.upgrade() {
                w.show_console(b.is_active());
            }
        });

        let weak = Rc::downgrade(self);
        self.console.connect_running_changed(move |running| {
            if let Some(w) = weak.upgrade() {
                let _ = running;
                w.update_header();
            }
        });
        let weak = Rc::downgrade(self);
        self.console.connect_finished(move |job, outcome| {
            if let Some(w) = weak.upgrade() {
                w.job_finished(job, outcome);
            }
        });

        let weak = Rc::downgrade(self);
        rescan.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.scan_devices();
            }
        });
        let weak = Rc::downgrade(self);
        tailnet_pair.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade()
                && let Some(tk) = w.toolkit_or_explain()
            {
                w.run_job(commands::tailnet_pair_job(&tk));
            }
        });
        let weak = Rc::downgrade(self);
        self.tailnet_entry.connect_changed(move |e| {
            if let Some(w) = weak.upgrade() {
                w.settings.borrow_mut().tailnet_host = e.text().trim().to_string();
                w.save();
            }
        });

        let weak = Rc::downgrade(self);
        self.device_dd.connect_selected_notify(move |dd| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing_devices.get() {
                return;
            }
            let udid = w.devices.borrow().get(dd.selected() as usize).map(|d| d.udid.clone());
            *w.target_udid.borrow_mut() = udid;
            w.render_devices();
        });

        // Drop project folders anywhere on the window.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(w), Ok(list)) = (weak.upgrade(), value.get::<gdk::FileList>()) else { return false };
            let mut added = false;
            for path in list.files().iter().filter_map(|f| f.path()) {
                added |= w.add_project(&path, true);
            }
            added
        });
        self.window.add_controller(drop);

        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |win| {
            if let Some(w) = weak.upgrade()
                && win.is_active()
            {
                w.unseen.set(0);
                w.badge.set_count(0);
                w.app.withdraw_notification("job");
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |win| {
            if let Some(w) = weak.upgrade() {
                {
                    let mut s = w.settings.borrow_mut();
                    s.maximized = win.is_maximized();
                    if !s.maximized {
                        s.width = win.width();
                        s.height = win.height();
                    }
                    if w.console.root.is_visible() {
                        s.console_height = (w.vpaned.height() - w.vpaned.position()).max(120);
                    }
                }
                w.save();
                if let Some(p) = w.poller.borrow().as_ref() {
                    p.set_enabled(false);
                }
                if w.console.is_running() {
                    w.console.stop();
                }
            }
            glib::Propagation::Proceed
        });
    }

    fn save(&self) {
        if let Err(e) = self.settings.borrow().save() {
            eprintln!("apple-dev: could not save settings: {e}");
        }
    }

    pub fn toast(&self, text: &str) {
        self.toast.set_title(text);
        self.toast.send_notification();
    }

    fn show_console(&self, show: bool) {
        if self.console_toggle.is_active() != show {
            self.console_toggle.set_active(show);
            return;
        }
        if show && !self.console.root.is_visible() {
            self.console.root.set_visible(true);
            let height = self.settings.borrow().console_height.max(160);
            let paned = self.vpaned.clone();
            glib::idle_add_local_once(move || {
                let total = paned.height();
                if total > 0 {
                    paned.set_position((total - height).max(total / 3));
                }
            });
        } else if !show && self.console.root.is_visible() {
            self.settings.borrow_mut().console_height = (self.vpaned.height() - self.vpaned.position()).max(120);
            self.console.root.set_visible(false);
        }
    }

    // --- Selection and header -------------------------------------------------------------------

    fn current_project(&self) -> Option<Project> {
        match Side::from_key(&self.settings.borrow().selected) {
            Side::Project(path) => project::inspect(&path),
            _ => None,
        }
    }

    fn select(self: &Rc<Self>, side: Side) {
        self.settings.borrow_mut().selected = side.key();
        self.save();
        let index = self.side.borrow().iter().position(|s| *s == side);
        self.syncing.set(true);
        match index.and_then(|i| self.sidebar.row_at_index(i as i32)) {
            Some(row) => self.sidebar.select_row(Some(&row)),
            None => self.sidebar.unselect_all(),
        }
        self.syncing.set(false);
        match &side {
            Side::Toolchain => self.stack.set_visible_child_name("toolchain"),
            Side::Devices => {
                self.stack.set_visible_child_name("devices");
                self.scan_devices();
            }
            Side::Project(_) => {
                self.render_project();
                self.stack.set_visible_child_name("project");
                self.project_scroll.vadjustment().set_value(0.0);
            }
            Side::Other => {}
        }
        self.update_header();
    }

    fn update_header(&self) {
        let running = self.console.is_running();
        let project = self.current_project();
        let (build, run) = project.as_ref().map(|p| commands::primary_actions(p.kind)).unwrap_or((None, None));
        let toolkit = self.toolkit().is_some();
        let set = |name: &str, enabled: bool| {
            if let Some(a) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                a.set_enabled(enabled);
            }
        };
        set("build", build.is_some() && toolkit && !running);
        set("run", run.is_some() && toolkit && !running);
        set("stop", running);
        if let Some(a) = build {
            self.build_btn.set_tooltip_text(Some(&format!("{} (Ctrl+B)", a.title())));
        }
        self.target_label.set_text(project.as_ref().map(|p| p.name.as_str()).unwrap_or("No project selected"));
        self.console.root.set_tooltip_text(self.console.running_title().as_deref());
    }

    // --- Running jobs ------------------------------------------------------------------------

    fn toolkit(&self) -> Option<PathBuf> {
        if let Some(r) = self.report.borrow().as_ref() {
            return r.toolkit.clone();
        }
        toolkit::locate(self.settings.borrow().toolkit_path.as_deref())
    }

    fn toolkit_or_explain(self: &Rc<Self>) -> Option<PathBuf> {
        let tk = self.toolkit();
        if tk.is_none() {
            self.toast("Download omarchy-apple-dev on the Toolchain page first");
            self.select(Side::Toolchain);
        }
        tk
    }

    pub fn run_job(self: &Rc<Self>, job: Job) {
        let title = job.title.clone();
        if !self.console.run(job) {
            let running = self.console.running_title().unwrap_or_default();
            self.toast(&format!("“{running}” is still running. Stop it first."));
            return;
        }
        self.show_console(true);
        self.window.set_title(Some(&format!("{title} — Apple Dev")));
    }

    fn primary(self: &Rc<Self>, build: bool) {
        let Some(project) = self.current_project() else { return };
        let (b, r) = commands::primary_actions(project.kind);
        if let Some(action) = if build { b } else { r } {
            self.run_action(action, &project);
        }
    }

    fn run_action(self: &Rc<Self>, action: Action, project: &Project) {
        let Some(tk) = self.toolkit_or_explain() else { return };
        if let Some(reason) = commands::blocker(action, project, &self.settings.borrow()) {
            self.toast(reason);
            return;
        }
        if action.uses_device() && self.devices.borrow().is_empty() {
            self.toast("No iPhone found yet. Running anyway; connect it and tap Trust.");
        }
        let udid = self.target_udid.borrow().clone();
        let job = commands::project_job(action, project, &tk, &self.settings.borrow(), udid.as_deref());
        self.run_job(job);
    }

    fn job_finished(self: &Rc<Self>, job: &Job, outcome: Outcome) {
        self.window.set_title(Some("Apple Dev"));
        let summary = match outcome {
            Outcome::Succeeded => format!("{} finished", job.title),
            Outcome::Failed(code) => format!("{} failed (exit {code})", job.title),
            Outcome::Stopped => format!("{} stopped", job.title),
        };
        self.toast(&summary);
        if outcome == Outcome::Succeeded {
            match &job.after {
                After::RecheckToolchain => self.recheck(),
                After::AddProject(path) => {
                    if !self.add_project(path, true) {
                        self.toast(&format!("{summary}, but {} is not a project folder", tilde(path)));
                    }
                }
                After::RefreshProject => {
                    if matches!(Side::from_key(&self.settings.borrow().selected), Side::Project(_)) {
                        self.render_project();
                    }
                }
                After::CheckTestFlight(path) => {
                    self.check_testflight(path, true);
                    if matches!(Side::from_key(&self.settings.borrow().selected), Side::Project(_)) {
                        self.render_project();
                    }
                }
                After::Nothing => {}
            }
        } else if job.after == After::RecheckToolchain {
            self.recheck();
        }
        if !self.window.is_active() && outcome != Outcome::Stopped {
            let n = gio::Notification::new(&summary);
            n.set_body(Some(match outcome {
                Outcome::Succeeded => "Open Apple Dev to see the output.",
                _ => "The Activity pane in Apple Dev has the full output.",
            }));
            n.set_icon(&gio::ThemedIcon::new(APP_ID));
            self.app.send_notification(Some("job"), &n);
            self.unseen.set(self.unseen.get() + 1);
            self.badge.set_count(self.unseen.get());
        }
    }

    // --- Projects ----------------------------------------------------------------------------

    /// Add a folder (or Xcode project) to the sidebar. Returns false when nothing there is runnable.
    pub fn add_project(self: &Rc<Self>, path: &Path, select: bool) -> bool {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(project) = project::inspect(&path) else {
            self.toast(&format!("{} has no xtool.yml, pubspec.yaml, Package.swift or Xcode project", tilde(&path)));
            return false;
        };
        let added = self.settings.borrow_mut().add_project(project.path.clone());
        if added {
            self.save();
            self.rebuild_projects();
        }
        if select {
            self.select(Side::Project(project.path));
        }
        true
    }

    fn remove_project(self: &Rc<Self>, path: &Path) {
        self.settings.borrow_mut().remove_project(path);
        self.save();
        self.rebuild_projects();
        self.select(Side::Toolchain);
        self.toast("Removed from the sidebar. The folder was not touched.");
    }

    fn rebuild_projects(self: &Rc<Self>) {
        self.syncing.set(true);
        while let Some(row) = self.sidebar.row_at_index(3) {
            self.sidebar.remove(&row);
        }
        let mut side = vec![Side::Toolchain, Side::Devices, Side::Other];
        let projects = self.settings.borrow().projects.clone();
        if projects.is_empty() {
            let hint =
                label("No projects yet. Use + below, or drop a folder here.", &["dim-label", "pantheon-caption"]);
            hint.set_margin_start(12);
            hint.set_margin_end(12);
            self.sidebar.append(&gtk::ListBoxRow::builder().selectable(false).activatable(false).child(&hint).build());
            side.push(Side::Other);
        }
        for path in projects {
            let (icon, name, subtitle) = match project::inspect(&path) {
                Some(p) => (p.kind.icon(), p.name, p.kind.label().to_string()),
                None => ("dialog-warning-symbolic", tilde(&path), "Folder not found".into()),
            };
            let (row, _, _) = side_row(icon, &name, Some(&subtitle));
            row.set_tooltip_text(Some(&path.display().to_string()));
            self.sidebar.append(&row);
            side.push(Side::Project(path));
        }
        *self.side.borrow_mut() = side;
        self.syncing.set(false);
    }

    fn new_app(self: &Rc<Self>) {
        let default = home().join("Projects");
        let default = if default.is_dir() { default } else { home() };
        let weak = Rc::downgrade(self);
        dialogs::new_app(self.window.upcast_ref(), default, move |dir, name| {
            if let Some(w) = weak.upgrade() {
                w.run_job(commands::new_app_job(&dir, &name));
            }
        });
    }

    fn choose_folder(self: &Rc<Self>, xcode: bool) {
        let dialog = gtk::FileDialog::builder()
            .title(if xcode { "Import Xcode Project" } else { "Add Project Folder" })
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(&self.window), None::<&gio::Cancellable>, move |result| {
            let (Some(w), Some(path)) = (weak.upgrade(), result.ok().and_then(|f| f.path())) else { return };
            match project::inspect(&path) {
                Some(p) if xcode && p.kind == Kind::XcodeProject => {
                    if let Some(tk) = w.toolkit_or_explain() {
                        w.run_job(commands::project_job(Action::ConvertXcode, &p, &tk, &w.settings.borrow(), None));
                    }
                }
                _ => {
                    w.add_project(&path, true);
                }
            }
        });
    }

    fn preferences(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let current = self.settings.borrow().clone();
        dialogs::preferences(self.window.upcast_ref(), &current, move |s| {
            if let Some(w) = weak.upgrade() {
                *w.settings.borrow_mut() = s;
                w.save();
                if matches!(Side::from_key(&w.settings.borrow().selected), Side::Project(_)) {
                    w.render_project();
                }
                w.toast(if w.settings.borrow().asc_ready() {
                    "App Store Connect key saved"
                } else {
                    "Saved. Uploads need the key file, Key ID and Issuer ID."
                });
            }
        });
    }

    fn show_toolkit(self: &Rc<Self>) {
        if let Some(tk) = self.toolkit_or_explain() {
            widgets::open_path(&self.window, &tk);
        }
    }

    // --- Toolchain ---------------------------------------------------------------------------

    pub fn recheck(self: &Rc<Self>) {
        if self.probing.replace(true) {
            return;
        }
        self.render_toolchain();
        let configured = self.settings.borrow().toolkit_path.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let report = gio::spawn_blocking(move || toolkit::probe(configured.as_deref())).await;
            let Some(w) = weak.upgrade() else { return };
            w.probing.set(false);
            if let Ok(report) = report {
                *w.report.borrow_mut() = Some(report);
            }
            w.render_toolchain();
            w.update_header();
            if matches!(Side::from_key(&w.settings.borrow().selected), Side::Project(_)) {
                w.render_project();
            }
        });
    }

    fn render_toolchain(self: &Rc<Self>) {
        let page = &self.toolchain_page;
        while let Some(child) = page.first_child() {
            page.remove(&child);
        }
        let report = self.report.borrow().clone();
        let probing = self.probing.get();
        let subtitle = match (&report, probing) {
            (None, _) => "Checking what is installed…".to_string(),
            (Some(r), _) if r.can_build() && r.problems() == 0 => "Ready to build and run iPhone apps.".into(),
            (Some(r), _) if r.can_build() => format!("Builds work. {} more to set up.", plural(r.problems(), "item")),
            (Some(r), _) => format!("{} to set up before the first build.", plural(r.problems(), "item")),
        };
        let (header, _, _, _, trailing) = page_header("applications-development", "Toolchain", &subtitle);
        if probing {
            let spinner = gtk::Spinner::builder().spinning(true).build();
            set_accessible_label(&spinner, "Checking");
            trailing.append(&spinner);
        }
        let again = icon_button("view-refresh-symbolic", "Check again", "Check the toolchain again");
        again.set_sensitive(!probing);
        again.set_action_name(Some("win.recheck"));
        trailing.append(&again);
        page.append(&header);

        // Sidebar badge.
        let problems = report.as_ref().map(Report::problems).unwrap_or(0);
        self.toolchain_badge.set_text(&problems.to_string());
        self.toolchain_badge.set_visible(problems > 0);
        self.toolchain_ok.set_visible(report.is_some() && problems == 0);

        let Some(report) = report else { return };

        if !report.host.arch_based {
            page.append(&self.this_computer(&report));
        }

        // The toolkit checkout.
        let (group, list) = section("omarchy-apple-dev");
        match &report.toolkit {
            Some(tk) => {
                let detail = match (&report.revision, &report.mode) {
                    (Some(rev), Some(mode)) => format!("Revision {rev} · install mode {mode}"),
                    (Some(rev), None) => format!("Revision {rev} · not installed yet"),
                    (None, _) => "Not a git checkout; Update is unavailable".into(),
                };
                let (r, t, _, _) = row(Some("folder-symbolic"), &tilde(tk), &detail);
                let update = button("Update", &[]);
                update.set_sensitive(report.revision.is_some());
                update.set_tooltip_text(Some("git pull --ff-only"));
                let tk_c = tk.clone();
                let weak = Rc::downgrade(self);
                update.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_job(commands::update_job(&tk_c));
                    }
                });
                let change = button("Change…", &["flat"]);
                self.connect_choose_toolkit(&change);
                t.append(&change);
                t.append(&update);
                list.append(&r);
            }
            None => {
                let dest = toolkit::default_location();
                let (r, t, _, _) = row(
                    Some("folder-download-symbolic"),
                    "Not downloaded",
                    &format!(
                        "Apple Dev runs the toolkit's own scripts. Download it to {} or choose an existing checkout.",
                        tilde(&dest)
                    ),
                );
                let choose = button("Choose…", &["flat"]);
                self.connect_choose_toolkit(&choose);
                let download = button("Download", &["suggested-action"]);
                let weak = Rc::downgrade(self);
                download.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_job(commands::clone_job(&dest));
                    }
                });
                t.append(&choose);
                t.append(&download);
                list.append(&r);
            }
        }
        page.append(&group);

        // Components.
        let (group, list) = section("Components");
        for check in &report.checks {
            let (icon, class) = match check.state {
                CheckState::Ready => ("emblem-ok-symbolic", "apple-dev-ok"),
                CheckState::Attention => ("dialog-warning-symbolic", "apple-dev-warn"),
                CheckState::Missing if check.optional => ("list-remove-symbolic", "dim-label"),
                CheckState::Missing => ("dialog-warning-symbolic", "apple-dev-warn"),
            };
            let (r, t, _, _) = row(None, check.title, &check.detail);
            let tooltip = match check.state {
                CheckState::Ready => "Ready",
                CheckState::Attention => "Needs attention",
                CheckState::Missing => "Not set up",
            };
            prepend_status(&r, icon, class, tooltip);
            let tk = report.toolkit.clone();
            let action: Option<(&str, Job)> = match (check.id, check.state, &tk) {
                (CheckId::AppleId, CheckState::Missing, _) if report.state(CheckId::Xtool) == CheckState::Ready => {
                    Some(("Sign In…", commands::sign_in_job(&home())))
                }
                (CheckId::AppleId, CheckState::Ready, _) => Some(("Sign In Again…", commands::sign_in_job(&home()))),
                (CheckId::DarwinSdk, CheckState::Missing, Some(tk))
                    if report.state(CheckId::Swift) == CheckState::Ready =>
                {
                    Some(("Repair", commands::repair_job(tk)))
                }
                (CheckId::Flutter, CheckState::Ready, Some(tk)) => {
                    Some(("Set Up for iOS", commands::flutter_setup_job(tk)))
                }
                _ => None,
            };
            if let Some((text, job)) = action {
                let b = button(text, &["flat"]);
                let weak = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_job(job.clone());
                    }
                });
                t.append(&b);
            }
            list.append(&r);
        }
        page.append(&group);

        // Install and repair.
        if let Some(tk) = report.toolkit.clone() {
            let (group, list) = section("Install");
            let (r, t, _, _) = row(
                Some("system-software-install-symbolic"),
                "Install or update the toolchain",
                "Safe to re-run. Stops at the iOS SDK step until you give it the Xcode .xip it names, unless you pick the no-Xcode mode.",
            );
            let install = button("Install…", &["suggested-action"]);
            let weak = Rc::downgrade(self);
            let tk_c = tk.clone();
            install.connect_clicked(move |_| {
                let Some(w) = weak.upgrade() else { return };
                let weak = Rc::downgrade(&w);
                let tk = tk_c.clone();
                dialogs::install(
                    w.window.upcast_ref(),
                    "Xcode 27 for Swift 6.4. Download it from developer.apple.com with a free Apple ID; no Mac needed.",
                    move |mode, xip| {
                        if let Some(w) = weak.upgrade() {
                            w.run_job(commands::install_job(&tk, mode, xip.as_deref()));
                        }
                    },
                );
            });
            t.append(&install);
            list.append(&r);
            let (r, t, _, _) = row(
                Some("view-refresh-symbolic"),
                "Repair after a Swift change",
                "Re-registers a current cached SDK after a Swift change. An outdated cache needs a rebuild from the matching Xcode .xip; choose Install above.",
            );
            let repair = button("Repair", &[]);
            let weak = Rc::downgrade(self);
            repair.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.run_job(commands::repair_job(&tk));
                }
            });
            t.append(&repair);
            list.append(&r);
            page.append(&group);
        }
    }

    fn connect_choose_toolkit(self: &Rc<Self>, b: &gtk::Button) {
        let weak = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            let Some(w) = weak.upgrade() else { return };
            let dialog = gtk::FileDialog::builder().title("Choose the omarchy-apple-dev Folder").modal(true).build();
            let weak = Rc::downgrade(&w);
            dialog.select_folder(Some(&w.window), None::<&gio::Cancellable>, move |result| {
                let (Some(w), Some(path)) = (weak.upgrade(), result.ok().and_then(|f| f.path())) else { return };
                if toolkit::is_toolkit(&path) {
                    w.settings.borrow_mut().toolkit_path = Some(path);
                    w.save();
                    w.recheck();
                } else {
                    w.toast("That folder has no install-toolchain.sh and device-run.sh");
                }
            });
        });
    }

    /// Non-Arch hosts: the apt packages and the swift.org Swift that match the iOS SDK.
    fn this_computer(self: &Rc<Self>, report: &Report) -> gtk::Box {
        let (group, list) = section("This Computer");
        let host = &report.host;
        if host.apt {
            let full = distro::apt_command(distro::APT_PACKAGES);
            let prefix_note = if report.limd_prefix.is_some() {
                format!(" libimobiledevice comes from ~/{}.", toolkit::LIMD_PREFIX)
            } else {
                String::new()
            };
            let (detail, ready, missing) = match &report.apt_missing {
                Some(m) if m.is_empty() => (format!("All installed.{prefix_note}"), true, Vec::new()),
                Some(m) => (
                    format!(
                        "{} to install: {}. Install runs apt in Activity; sudo asks for your password there.{prefix_note}",
                        plural(m.len(), "package"),
                        m.join(" ")
                    ),
                    false,
                    m.clone(),
                ),
                None => (format!("Install with: {full}"), false, Vec::new()),
            };
            let (r, t, _, d) = row(None, &format!("Packages for {}", host.pretty_name), &detail);
            d.set_selectable(true);
            if ready {
                prepend_status(&r, "emblem-ok-symbolic", "apple-dev-ok", "Ready");
            } else {
                prepend_status(&r, "dialog-warning-symbolic", "apple-dev-warn", "Not set up");
            }
            let copy = button("Copy", &["flat"]);
            copy.set_tooltip_text(Some(&full));
            let weak = Rc::downgrade(self);
            copy.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.window.clipboard().set_text(&full);
                    w.toast("Copied the apt command");
                }
            });
            t.append(&copy);
            if !missing.is_empty() {
                let install = button("Install", &["suggested-action"]);
                let weak = Rc::downgrade(self);
                install.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_job(commands::apt_job(&home(), &missing));
                    }
                });
                t.append(&install);
            }
            list.append(&r);
        }

        let xcode = report.xcode.clone().unwrap_or_else(|| distro::default_xcode().to_string());
        let tarball = host
            .ubuntu
            .as_deref()
            .zip(distro::swift_for_xcode(&xcode))
            .and_then(|(ubuntu, swift)| distro::swift_tarball(swift, ubuntu, std::env::consts::ARCH));
        match tarball {
            Some(t) => {
                let installed = distro::swift_matches(&report.swift_version, &t.swift);
                let sdk_note =
                    if report.xcode.is_some() { "" } else { " No iOS SDK yet: use the Xcode .xip of this version." };
                let detail = if installed {
                    format!("{}. Matches the iOS SDK from Xcode {xcode}.", report.swift_version)
                } else if report.swift_version.is_empty() {
                    format!(
                        "Not installed. Install downloads the swift.org build for Ubuntu {} {} (signature checked) into ~/{} and links ~/.local/bin/swift. No sudo.{sdk_note}",
                        host.ubuntu.as_deref().unwrap_or_default(),
                        std::env::consts::ARCH,
                        toolkit::SWIFT_DIR
                    )
                } else {
                    format!(
                        "{} does not match Xcode {xcode}. Install puts Swift {} in ~/{} and points ~/.local/bin/swift at it.{sdk_note}",
                        report.swift_version,
                        t.swift,
                        toolkit::SWIFT_DIR
                    )
                };
                let (r, tr, _, _) = row(None, &format!("Swift {} for Xcode {xcode}", t.swift), &detail);
                if installed {
                    prepend_status(&r, "emblem-ok-symbolic", "apple-dev-ok", "Ready");
                } else {
                    prepend_status(&r, "dialog-warning-symbolic", "apple-dev-warn", "Not set up");
                    let install = button("Install Swift", &["suggested-action"]);
                    install.set_tooltip_text(Some(&t.url));
                    let weak = Rc::downgrade(self);
                    let tk = report.toolkit.clone();
                    install.connect_clicked(move |_| {
                        if let Some(w) = weak.upgrade() {
                            w.run_job(commands::swift_install_job(&home(), &t, tk.as_deref()));
                        }
                    });
                    tr.append(&install);
                }
                list.append(&r);
            }
            None => {
                let (r, t, _, _) = row(
                    Some("dialog-information-symbolic"),
                    &format!("{} is not Arch-based", host.pretty_name),
                    "The installer skips its pacman steps here. Install the packages in docs/DISTROS.md and put a swift.org Swift 6.4 toolchain on PATH, then run Install.",
                );
                let read = button("Read Guide", &[]);
                let doc = report.toolkit.as_ref().map(|t| t.join("docs/DISTROS.md")).filter(|p| p.is_file());
                let weak = Rc::downgrade(self);
                read.connect_clicked(move |_| {
                    let Some(w) = weak.upgrade() else { return };
                    match &doc {
                        Some(path) => widgets::open_path(&w.window, path),
                        None => {
                            widgets::open_uri(&w.window, &format!("{}/blob/main/docs/DISTROS.md", toolkit::UPSTREAM))
                        }
                    }
                });
                t.append(&read);
                list.append(&r);
            }
        }
        group
    }

    // --- TestFlight --------------------------------------------------------------------------

    fn showing_project(&self, path: &Path) -> bool {
        matches!(Side::from_key(&self.settings.borrow().selected), Side::Project(p) if p == path)
    }

    /// Ask App Store Connect for the project's newest builds. `fresh` (after an upload or a click)
    /// restarts the follow-up checks while Apple is still processing. Does not re-render itself
    /// until the answer arrives.
    fn check_testflight(self: &Rc<Self>, path: &Path, fresh: bool) {
        if matches!(self.testflight.borrow().get(path), Some(TestFlightView::Checking)) {
            return;
        }
        if fresh {
            self.testflight_polls.borrow_mut().insert(path.to_path_buf(), TESTFLIGHT_POLLS);
        }
        let done = |w: &Rc<Self>, result: Result<Builds, String>| {
            w.testflight
                .borrow_mut()
                .insert(path.to_path_buf(), TestFlightView::Done { result, checked: SystemTime::now() });
        };
        let app = project::inspect(path).as_ref().and_then(testflight::app_ref);
        let tk = self.toolkit().or_else(|| testflight::fixture().map(|_| PathBuf::new()));
        let (Some(tk), Some(app)) = (tk, app) else {
            done(self, Err("Needs a bundleID in xtool.yml or a signed .ipa to look the app up.".into()));
            self.testflight_polls.borrow_mut().remove(path);
            return;
        };
        let env = self.settings.borrow().asc_env();
        self.testflight.borrow_mut().insert(path.to_path_buf(), TestFlightView::Checking);
        let weak = Rc::downgrade(self);
        let path = path.to_path_buf();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || testflight::fetch(&tk, &app, &env))
                .await
                .unwrap_or_else(|_| Err("The App Store Connect check stopped unexpectedly.".into()));
            let Some(w) = weak.upgrade() else { return };
            let pending = result.as_ref().is_ok_and(Builds::pending);
            w.testflight.borrow_mut().insert(path.clone(), TestFlightView::Done { result, checked: SystemTime::now() });
            let again = {
                let mut polls = w.testflight_polls.borrow_mut();
                match polls.get_mut(&path) {
                    Some(left) if pending && *left > 0 => {
                        *left -= 1;
                        true
                    }
                    _ => {
                        polls.remove(&path);
                        false
                    }
                }
            };
            if again {
                let weak = Rc::downgrade(&w);
                let path = path.clone();
                glib::timeout_add_local_once(TESTFLIGHT_POLL, move || {
                    if let Some(w) = weak.upgrade() {
                        w.check_testflight(&path, false);
                        if w.showing_project(&path) {
                            w.render_project();
                        }
                    }
                });
            }
            if w.showing_project(&path) {
                w.render_project();
            }
        });
    }

    fn testflight_row(self: &Rc<Self>, project: &Project, view: Option<TestFlightView>) -> gtk::ListBoxRow {
        let polling = self.testflight_polls.borrow().contains_key(&project.path);
        let check_button = |text: &str| {
            let b = button(text, &["flat"]);
            let weak = Rc::downgrade(self);
            let path = project.path.clone();
            b.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.check_testflight(&path, true);
                    w.render_project();
                }
            });
            b
        };
        match view {
            None => {
                let (r, t, _, _) = row(
                    Some("send-to-symbolic"),
                    "TestFlight",
                    "Apple's processing and TestFlight state of the newest build. Checked after every upload.",
                );
                t.append(&check_button("Check"));
                r
            }
            Some(TestFlightView::Checking) => {
                let (r, t, _, _) = row(Some("send-to-symbolic"), "TestFlight", "Asking App Store Connect…");
                let spinner = gtk::Spinner::builder().spinning(true).build();
                set_accessible_label(&spinner, "Checking");
                t.append(&spinner);
                r
            }
            Some(TestFlightView::Done { result: Err(e), .. }) => {
                let (r, t, _, d) = row(None, "TestFlight status unavailable", &e);
                d.set_selectable(true);
                prepend_status(&r, "dialog-warning-symbolic", "apple-dev-warn", "Needs attention");
                t.append(&check_button("Check Again"));
                r
            }
            Some(TestFlightView::Done { result: Ok(builds), checked }) => {
                let (title, mut detail, level) = match builds.builds.first() {
                    Some(b) => {
                        let (level, state) = b.state();
                        let mut detail = state.to_string();
                        if let Some(age) = iso_age(&b.uploaded) {
                            detail.push_str(&format!(" · uploaded {age}"));
                        }
                        (format!("TestFlight: {}", b.title()), detail, level)
                    }
                    None => (
                        "TestFlight".to_string(),
                        "No builds in App Store Connect yet; an upload shows up within a few minutes.".to_string(),
                        Level::Waiting,
                    ),
                };
                detail.push_str(&format!("\nChecked {}", widgets::age(checked)));
                if polling {
                    detail.push_str("; checks again every 30 seconds while Apple works on it.");
                }
                let (r, t, _, _) = row(None, &title, &detail);
                match level {
                    Level::Ready => prepend_status(&r, "emblem-ok-symbolic", "apple-dev-ok", "Ready"),
                    Level::Waiting => prepend_status(&r, "content-loading-symbolic", "dim-label", "Processing"),
                    Level::Problem => {
                        prepend_status(&r, "dialog-warning-symbolic", "apple-dev-warn", "Needs attention")
                    }
                }
                let open = button("Open", &["flat"]);
                open.set_tooltip_text(Some("Open TestFlight in App Store Connect"));
                let url = builds.testflight_url();
                let weak = Rc::downgrade(self);
                open.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        widgets::open_uri(&w.window, &url);
                    }
                });
                open.set_visible(!builds.app_id.is_empty());
                t.append(&open);
                t.append(&check_button("Check Again"));
                r
            }
        }
    }

    // --- Devices -----------------------------------------------------------------------------

    fn start_device_polling(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let poller = Polling::new(Duration::from_secs(4), move || {
            if let Some(w) = weak.upgrade() {
                w.scan_devices();
            }
        });
        poller.bind_to_widget_mapped(&self.window);
        poller.set_enabled(true);
        poller.poll_now();
        *self.poller.borrow_mut() = Some(poller);
    }

    fn scan_devices(self: &Rc<Self>) {
        if self.scanning.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(devices::scan).await.unwrap_or_default();
            let Some(w) = weak.upgrade() else { return };
            w.scanning.set(false);
            if *w.devices.borrow() == found {
                return;
            }
            *w.devices.borrow_mut() = found;
            w.sync_device_picker();
            w.render_devices();
        });
    }

    fn sync_device_picker(&self) {
        let devices = self.devices.borrow();
        let mut target = self.target_udid.borrow_mut();
        if !target.as_ref().is_some_and(|u| devices.iter().any(|d| &d.udid == u)) {
            *target = devices.first().map(|d| d.udid.clone());
        }
        self.syncing_devices.set(true);
        let names: Vec<&str> =
            if devices.is_empty() { vec!["No iPhone"] } else { devices.iter().map(|d| d.name.as_str()).collect() };
        self.device_model.splice(0, self.device_model.n_items(), &names);
        let index = target.as_ref().and_then(|u| devices.iter().position(|d| &d.udid == u)).unwrap_or(0);
        self.device_dd.set_selected(index as u32);
        self.device_dd.set_sensitive(!devices.is_empty());
        self.syncing_devices.set(false);
        self.devices_badge.set_text(&devices.len().to_string());
        self.devices_badge.set_visible(!devices.is_empty());
    }

    fn render_devices(self: &Rc<Self>) {
        let list_box = &self.devices_list;
        while let Some(child) = list_box.first_child() {
            list_box.remove(&child);
        }
        let devices = self.devices.borrow().clone();
        if devices.is_empty() {
            let empty = granite::Placeholder::new("No iPhone Connected");
            empty.set_description(
                "Connect it with a USB cable, unlock it and tap Trust. It appears here within a few seconds; Run and Debug install on the phone selected here.",
            );
            empty.set_icon(&gio::ThemedIcon::new("phone"));
            empty.set_vexpand(false);
            list_box.append(&empty);
            return;
        }
        let (group, list) = section("Connected");
        let target = self.target_udid.borrow().clone();
        let mut first_check: Option<gtk::CheckButton> = None;
        for device in devices {
            let detail = format!("{}\n{}", device.summary(), device.udid);
            let (r, t, _, d) = row(Some("phone-symbolic"), &device.name, &detail);
            d.set_selectable(true);
            let target_check = gtk::CheckButton::with_label("Run target");
            if let Some(first) = &first_check {
                target_check.set_group(Some(first));
            } else {
                first_check = Some(target_check.clone());
            }
            target_check.set_active(target.as_deref() == Some(device.udid.as_str()));
            let weak = Rc::downgrade(self);
            let udid = device.udid.clone();
            target_check.connect_toggled(move |c| {
                if let Some(w) = weak.upgrade()
                    && c.is_active()
                {
                    *w.target_udid.borrow_mut() = Some(udid.clone());
                    w.sync_device_picker();
                }
            });
            t.append(&target_check);
            let buttons: [(&str, DeviceJob); 3] =
                [("Pair", commands::pair_job), ("Details", commands::info_job), ("Live Log", commands::syslog_job)];
            for (text, make) in buttons {
                let b = button(text, &["flat"]);
                let weak = Rc::downgrade(self);
                let udid = device.udid.clone();
                b.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_job(make(&home(), &udid));
                    }
                });
                t.append(&b);
            }
            list.append(&r);
        }
        list_box.append(&group);
    }

    // --- Project page ------------------------------------------------------------------------

    fn render_project(self: &Rc<Self>) {
        let page = &self.project_page;
        while let Some(child) = page.first_child() {
            page.remove(&child);
        }
        let Side::Project(path) = Side::from_key(&self.settings.borrow().selected) else { return };
        let Some(project) = project::inspect(&path) else {
            let missing = granite::Placeholder::new("Folder Not Found");
            missing.set_description(&format!("{} was moved or deleted.", tilde(&path)));
            missing.set_icon(&gio::ThemedIcon::new("dialog-warning"));
            let remove = missing.append_button(
                &gio::ThemedIcon::new("list-remove"),
                "Remove from Sidebar",
                "The project list forgets it; nothing is deleted.",
            );
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.remove_project(&path);
                }
            });
            page.append(&missing);
            return;
        };

        let mut subtitle = project.kind.label().to_string();
        if let Some(id) = &project.bundle_id {
            subtitle.push_str(" · ");
            subtitle.push_str(id);
        }
        let icon = match project.kind {
            Kind::MacPackage => "computer",
            Kind::XcodeProject => "folder-documents",
            _ => "phone",
        };
        let (header, _, _, sub, trailing) = page_header(icon, &project.name, &subtitle);
        sub.set_selectable(true);
        let open = icon_button("folder-open-symbolic", "Open the project folder", "Open folder");
        let folder = project.path.clone();
        let weak = Rc::downgrade(self);
        open.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                widgets::open_path(&w.window, &folder);
            }
        });
        let remove =
            icon_button("list-remove-symbolic", "Remove from the sidebar (files are kept)", "Remove from sidebar");
        let weak = Rc::downgrade(self);
        let p = project.path.clone();
        remove.connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.remove_project(&p);
            }
        });
        trailing.append(&open);
        trailing.append(&remove);
        page.append(&header);

        let path_label = label(&tilde(&project.path), &["dim-label", "pantheon-caption"]);
        path_label.set_selectable(true);
        path_label.set_wrap(false);
        path_label.set_ellipsize(pango::EllipsizeMode::Middle);
        page.append(&path_label);

        // Last build and toolchain readiness.
        let (group, list) = section("Status");
        let (r, t, _, _) = match project::latest_artifact(&project) {
            Some(a) => {
                let name = a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (r, t, title, detail) =
                    row(Some("package-x-generic-symbolic"), &format!("Last build: {name}"), &widgets::age(a.modified));
                let show = button("Show", &["flat"]);
                let artifact = a.path.clone();
                let weak = Rc::downgrade(self);
                show.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        widgets::show_in_files(&w.window, &artifact);
                    }
                });
                t.append(&show);
                (r, t, title, detail)
            }
            None => row(
                Some("package-x-generic-symbolic"),
                "Not built yet",
                "Build output appears here after the first build.",
            ),
        };
        let _ = t;
        list.append(&r);
        let uploads =
            commands::actions_for(project.kind).iter().any(|a| matches!(a, Action::Upload | Action::FlutterUpload));
        if uploads {
            if testflight::fixture().is_some() && !self.testflight.borrow().contains_key(&project.path) {
                self.check_testflight(&project.path, false);
            }
            let view = self.testflight.borrow().get(&project.path).cloned();
            if view.is_some() || self.settings.borrow().asc_ready() {
                list.append(&self.testflight_row(&project, view));
            }
        }
        let report = self.report.borrow().clone();
        let ready = report.as_ref().map(Report::can_build);
        if project.kind != Kind::XcodeProject && ready == Some(false) {
            let (r, t, _, _) = row(
                Some("dialog-warning-symbolic"),
                "The toolchain is not ready",
                "Builds need Swift, the iOS SDK and xtool. The Toolchain page shows what is missing.",
            );
            let go = button("Open Toolchain", &[]);
            let weak = Rc::downgrade(self);
            go.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.select(Side::Toolchain);
                }
            });
            t.append(&go);
            list.append(&r);
        }
        page.append(&group);

        let settings = self.settings.borrow().clone();
        let device_name = {
            let target = self.target_udid.borrow();
            self.devices.borrow().iter().find(|d| Some(&d.udid) == target.as_ref()).map(|d| d.name.clone())
        };
        for (title, wanted) in [("Develop", Section::Develop), ("Distribute", Section::Distribute)] {
            let actions: Vec<Action> =
                commands::actions_for(project.kind).iter().copied().filter(|a| a.section() == wanted).collect();
            if actions.is_empty() {
                continue;
            }
            let (group, list) = section(title);
            for action in actions {
                let mut detail = action.description().to_string();
                if action.uses_device() {
                    match &device_name {
                        Some(name) => detail.push_str(&format!(" Target: {name}.")),
                        None => detail.push_str(" No iPhone connected."),
                    }
                }
                let blocker = commands::blocker(action, &project, &settings);
                if let Some(reason) = blocker {
                    detail = format!("{detail}\n{reason}");
                }
                let (r, t, _, _) = row(Some(action.icon()), action.title(), &detail);
                let short = match action {
                    Action::Run => "Run",
                    Action::Debug => "Debug",
                    Action::Attach => "Attach",
                    Action::Upload | Action::FlutterUpload => "Upload",
                    Action::TailnetInstall | Action::FlutterInstall => "Install",
                    Action::ConvertXcode => "Convert",
                    Action::MacNotarize => "Notarize",
                    _ => "Build",
                };
                let primary = commands::primary_actions(project.kind);
                let classes: &[&str] =
                    if Some(action) == primary.0 || Some(action) == primary.1 { &["suggested-action"] } else { &[] };
                let b = button(short, classes);
                b.set_sensitive(blocker.is_none());
                b.set_tooltip_text(Some(action.title()));
                let weak = Rc::downgrade(self);
                let p = project.clone();
                b.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.run_action(action, &p);
                    }
                });
                t.append(&b);
                list.append(&r);
            }
            page.append(&group);
        }
    }
}

fn icon_button(icon: &str, tooltip: &str, accessible: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b.set_valign(gtk::Align::Center);
    set_accessible_label(&b, accessible);
    b
}

/// A sidebar row: icon, title, optional subtitle, a count badge and a ready mark.
fn side_row(icon: &str, title: &str, subtitle: Option<&str>) -> (gtk::ListBoxRow, gtk::Label, gtk::Image) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    set_box_spacing_text_units(&content, 0.45);
    content.set_margin_top(3);
    content.set_margin_bottom(3);
    let image = gtk::Image::from_icon_name(icon);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = gtk::Label::builder().label(title).xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
    text.append(&t);
    if let Some(sub) = subtitle {
        let s = gtk::Label::builder().label(sub).xalign(0.0).ellipsize(pango::EllipsizeMode::End).build();
        s.add_css_class("dim-label");
        s.add_css_class("pantheon-caption");
        text.append(&s);
    }
    let badge = gtk::Label::new(None);
    badge.add_css_class("apple-dev-badge");
    badge.set_valign(gtk::Align::Center);
    badge.set_visible(false);
    let ok = gtk::Image::from_icon_name("emblem-ok-symbolic");
    ok.add_css_class("apple-dev-ok");
    ok.set_visible(false);
    ok.set_tooltip_text(Some("Ready"));
    content.append(&image);
    content.append(&text);
    content.append(&badge);
    content.append(&ok);
    (gtk::ListBoxRow::builder().child(&content).build(), badge, ok)
}

/// A status icon in front of a row's text.
fn prepend_status(row: &gtk::ListBoxRow, icon: &str, class: &str, tooltip: &str) {
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class(class);
    image.set_tooltip_text(Some(tooltip));
    if let Some(content) = row.child().and_downcast::<gtk::Box>() {
        content.prepend(&image);
    }
}

/// "5 minutes ago" for an App Store Connect ISO 8601 time.
fn iso_age(iso: &str) -> Option<String> {
    let dt = glib::DateTime::from_iso8601(iso, None).ok()?;
    let secs = u64::try_from(dt.to_unix()).ok()?;
    Some(widgets::age(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)))
}

fn plural(n: usize, unit: &str) -> String {
    if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") }
}
