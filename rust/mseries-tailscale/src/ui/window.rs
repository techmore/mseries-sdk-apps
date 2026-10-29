//! Main window. One column (utility window): status card, exit node, devices, settings.
//!
//! State comes from `tailscale status --json` and `tailscale debug prefs`, every 3 s while the window is mapped
//! and right after every change. Controls never hold their own state: each refresh writes the real state back,
//! so a refused change springs back and the reason shows in a banner or toast.

use crate::cli::{self, CliError};
use crate::model::{self, Backend, Device, Prefs, Status};
use crate::strings::tr;
use granite::prelude::{HeaderLabelExt, PlaceholderExt};
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{appearance, chrome, set_accessible_label};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub const APP_ID: &str = "org.techmore.mseries.Tailscale";
const ADMIN_URL: &str = "https://login.tailscale.com/admin/machines";

pub struct AppWindow {
    pub window: gtk::ApplicationWindow,
    toast: granite::Toast,
    pages: gtk::Stack,
    placeholder: granite::Placeholder,
    placeholder_command: RefCell<String>,
    copy_button: gtk::Button,
    login_button: gtk::Button,
    // Status card
    status_icon: gtk::Image,
    status_title: gtk::Label,
    status_detail: gtk::Label,
    connect: gtk::Switch,
    this_name: gtk::Label,
    this_ip: gtk::Label,
    this_row: gtk::Box,
    // One-time setup banner
    setup: gtk::Revealer,
    health: gtk::Label,
    // Exit node
    exit_section: gtk::Box,
    exit_choice: gtk::DropDown,
    exit_ids: RefCell<Vec<String>>,
    exit_note: gtk::Label,
    lan_access: gtk::Switch,
    // Devices
    devices_header: granite::HeaderLabel,
    search: gtk::SearchEntry,
    device_list: gtk::ListBox,
    devices_key: RefCell<String>,
    // Settings
    accept_dns: gtk::Switch,
    accept_routes: gtk::Switch,
    allow_incoming: gtk::Switch,
    run_ssh: gtk::Switch,
    version: gtk::Label,
    // State
    status: RefCell<Option<Status>>,
    prefs: RefCell<Prefs>,
    updating: Cell<bool>,
    busy: Cell<bool>,
    poll: RefCell<Option<mseries_pantheon_kit::Polling>>,
}

impl AppWindow {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder().application(app).title(tr("Tailscale")).build();
        appearance::size_window(&window, appearance::UTILITY_DEFAULT, appearance::UTILITY_MIN);
        window.add_css_class("mseries-tailscale");
        install_style(&window);

        // Header: refresh and the primary menu.
        let header = gtk::HeaderBar::new();
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_action_name(Some("win.refresh"));
        refresh.set_tooltip_text(Some(&tr("Refresh (Ctrl+R)")));
        set_accessible_label(&refresh, &tr("Refresh"));
        header.pack_start(&refresh);
        header.pack_end(&chrome::primary_menu(&tr("About Tailscale"), &tr("Quit")));
        window.set_titlebar(Some(&header));

        // ---- Status card ------------------------------------------------------------------------------------
        let status_icon = gtk::Image::from_icon_name("mseries-tailscale-offline-symbolic");
        status_icon.set_pixel_size(48);
        status_icon.set_valign(gtk::Align::Center);
        let status_title = gtk::Label::builder().xalign(0.0).label(tr("Checking…")).build();
        status_title.add_css_class("title-2");
        let status_detail = gtk::Label::builder().xalign(0.0).wrap(true).build();
        status_detail.add_css_class(granite::CssClassDIM);
        let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        titles.set_valign(gtk::Align::Center);
        titles.append(&status_title);
        titles.append(&status_detail);
        let connect = gtk::Switch::builder().valign(gtk::Align::Center).build();
        set_accessible_label(&connect, &tr("Connected"));
        let top = gtk::Box::builder().spacing(12).build();
        top.append(&status_icon);
        top.append(&titles);
        top.append(&connect);

        let this_name = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let this_ip = gtk::Label::builder().xalign(0.0).selectable(true).build();
        this_ip.add_css_class(granite::CssClassDIM);
        this_ip.add_css_class(granite::CssClassNUMERIC);
        let this_text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let this_caption = gtk::Label::builder().xalign(0.0).label(tr("This device")).build();
        this_caption.add_css_class(granite::CssClassSMALL);
        this_caption.add_css_class(granite::CssClassDIM);
        this_text.append(&this_caption);
        this_text.append(&this_name);
        this_text.append(&this_ip);
        let this_copy = icon_button("edit-copy-symbolic", &tr("Copy IP Address"));
        let this_row = gtk::Box::builder().spacing(6).build();
        this_row.append(&this_text);
        this_row.append(&this_copy);

        let card = card_box();
        card.set_spacing(12);
        card.append(&top);
        card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        card.append(&this_row);

        // One-time operator setup, shown after a change is refused.
        let setup_text = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .label(tr(
                "To connect, disconnect or change settings without a password prompt, run this once in Terminal:",
            ))
            .build();
        let setup_cmd = gtk::Label::builder()
            .xalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .label(cli::operator_command())
            .hexpand(true)
            .build();
        setup_cmd.add_css_class(granite::CssClassMONOSPACE);
        let setup_copy = gtk::Button::with_label(&tr("Copy"));
        setup_copy.set_valign(gtk::Align::Center);
        let setup_line = gtk::Box::builder().spacing(12).build();
        setup_line.append(&setup_cmd);
        setup_line.append(&setup_copy);
        let setup_box = card_box();
        setup_box.add_css_class("mseries-tailscale-setup");
        let setup_title = gtk::Label::builder().xalign(0.0).label(tr("One-time setup needed")).build();
        setup_title.add_css_class("heading");
        setup_box.append(&setup_title);
        setup_box.append(&setup_text);
        setup_box.append(&setup_line);
        let setup = gtk::Revealer::builder().child(&setup_box).reveal_child(false).visible(false).build();
        setup.connect_child_revealed_notify(|r| r.set_visible(r.reveals_child() || r.is_child_revealed()));

        let health = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
        health.add_css_class(granite::CssClassWARNING);

        // ---- Exit node --------------------------------------------------------------------------------------
        let exit_header = granite::HeaderLabel::new(&tr("Exit Node"));
        exit_header.set_secondary_text(Some(&tr("Send all internet traffic through another device")));
        let exit_choice = gtk::DropDown::from_strings(&[&tr("None")]);
        exit_choice.set_valign(gtk::Align::Center);
        set_accessible_label(&exit_choice, &tr("Exit node"));
        let exit_note = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
        exit_note.add_css_class(granite::CssClassDIM);
        let lan_access = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let exit_card = card_box();
        exit_card.append(&setting_row(&tr("Use exit node"), None, &exit_choice));
        exit_card.append(&exit_note);
        exit_card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        exit_card.append(&setting_row(
            &tr("Allow local network access"),
            Some(&tr("Reach printers and other devices at home while using an exit node")),
            &lan_access,
        ));
        let exit_section = section(&exit_header, &exit_card);

        // ---- Devices ----------------------------------------------------------------------------------------
        let devices_header = granite::HeaderLabel::new(&tr("Devices"));
        let search = gtk::SearchEntry::builder().placeholder_text(tr("Search devices")).build();
        let device_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        device_list.add_css_class("boxed-list");
        device_list.add_css_class(granite::CssClassCARD);
        device_list.add_css_class("mseries-tailscale-devices");
        let devices_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
        devices_box.append(&search);
        devices_box.append(&device_list);
        let devices_section = section(&devices_header, &devices_box);

        // ---- Settings ---------------------------------------------------------------------------------------
        let accept_dns = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let accept_routes = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let allow_incoming = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let run_ssh = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let settings_card = card_box();
        settings_card.append(&setting_row(
            &tr("Use Tailscale DNS"),
            Some(&tr("Reach devices by name, like sdolbec-ser8")),
            &accept_dns,
        ));
        settings_card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        settings_card.append(&setting_row(
            &tr("Use subnet routes"),
            Some(&tr("Reach networks that other devices share")),
            &accept_routes,
        ));
        settings_card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        settings_card.append(&setting_row(
            &tr("Allow incoming connections"),
            Some(&tr("Other devices can connect to this one")),
            &allow_incoming,
        ));
        settings_card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        settings_card.append(&setting_row(
            &tr("Tailscale SSH server"),
            Some(&tr("Let your tailnet SSH into this device")),
            &run_ssh,
        ));
        let settings_section = section(&granite::HeaderLabel::new(&tr("Settings")), &settings_card);

        let admin = gtk::LinkButton::with_label(ADMIN_URL, &tr("Admin Console…"));
        admin.set_halign(gtk::Align::Start);
        let version = gtk::Label::builder().xalign(1.0).hexpand(true).build();
        version.add_css_class(granite::CssClassDIM);
        version.add_css_class(granite::CssClassSMALL);
        let footer = gtk::Box::builder().spacing(12).build();
        footer.append(&admin);
        footer.append(&version);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(18)
            .margin_bottom(24)
            .margin_start(18)
            .margin_end(18)
            .build();
        column.append(&card);
        column.append(&setup);
        column.append(&health);
        column.append(&exit_section);
        column.append(&devices_section);
        column.append(&settings_section);
        column.append(&footer);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&column).build();

        // ---- Whole-window states: checking, not running, logged out ------------------------------------------
        let placeholder = granite::Placeholder::new(&tr("Checking Tailscale…"));
        placeholder.set_icon(&gio::ThemedIcon::new("mseries-tailscale-offline-symbolic"));
        let copy_button = placeholder.append_button(
            &gio::ThemedIcon::new("edit-copy"),
            &tr("Copy Command"),
            &tr("Paste it in Terminal"),
        );
        let login_button = placeholder.append_button(
            &gio::ThemedIcon::new("system-users"),
            &tr("Log In…"),
            &tr("Opens the Tailscale sign-in page in your browser"),
        );
        copy_button.set_visible(false);
        login_button.set_visible(false);
        let pages = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        pages.add_named(&placeholder, Some("placeholder"));
        pages.add_named(&scroller, Some("main"));
        pages.set_visible_child_name("placeholder");

        let (overlay, toast) = chrome::toast_overlay(&pages);
        window.set_child(Some(&overlay));

        let this = Rc::new(Self {
            window,
            toast,
            pages,
            placeholder,
            placeholder_command: RefCell::new(String::new()),
            copy_button,
            login_button,
            status_icon,
            status_title,
            status_detail,
            connect,
            this_name,
            this_ip,
            this_row,
            setup,
            health,
            exit_section,
            exit_choice,
            exit_ids: RefCell::new(Vec::new()),
            exit_note,
            lan_access,
            devices_header,
            search,
            device_list,
            devices_key: RefCell::new(String::new()),
            accept_dns,
            accept_routes,
            allow_incoming,
            run_ssh,
            version,
            status: RefCell::new(None),
            prefs: RefCell::new(Prefs::default()),
            updating: Cell::new(false),
            busy: Cell::new(false),
            poll: RefCell::new(None),
        });

        this.install_actions();
        this.connect_controls(&this_copy, &setup_copy);

        let weak = Rc::downgrade(&this);
        let poll = mseries_pantheon_kit::Polling::new(Duration::from_secs(3), move || {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
        });
        poll.bind_to_widget_mapped(&this.window);
        *this.poll.borrow_mut() = Some(poll);
        this.refresh();
        this
    }

    fn install_actions(self: &Rc<Self>) {
        let refresh = gio::SimpleAction::new("refresh", None);
        let weak = Rc::downgrade(self);
        refresh.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
        });
        self.window.add_action(&refresh);
        let find = gio::SimpleAction::new("find", None);
        let weak = Rc::downgrade(self);
        find.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.search.grab_focus();
            }
        });
        self.window.add_action(&find);
        if let Some(app) = self.window.application() {
            app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
            app.set_accels_for_action("win.find", &["<Control>f"]);
        }
    }

    fn connect_controls(self: &Rc<Self>, this_copy: &gtk::Button, setup_copy: &gtk::Button) {
        let weak = Rc::downgrade(self);
        this_copy.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let ip = this.status.borrow().as_ref().and_then(|s| s.this_device.as_ref().map(|d| d.ip().to_string()));
            if let Some(ip) = ip {
                this.copy(&ip, &format!("{} {ip}", tr("Copied")));
            }
        });
        let weak = Rc::downgrade(self);
        setup_copy.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.copy(&cli::operator_command(), &tr("Copied. Paste it in Terminal, then try again."));
            }
        });

        let weak = Rc::downgrade(self);
        self.copy_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                let command = this.placeholder_command.borrow().clone();
                this.copy(&command, &tr("Copied. Paste it in Terminal."));
            }
        });
        let weak = Rc::downgrade(self);
        self.login_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.log_in();
            }
        });

        let weak = Rc::downgrade(self);
        self.connect.connect_state_set(move |_, on| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Stop };
            if this.updating.get() {
                return glib::Propagation::Proceed;
            }
            this.change(
                if on { vec!["up"] } else { vec!["down"] },
                if on { tr("Connected") } else { tr("Disconnected") },
            );
            glib::Propagation::Stop
        });

        let weak = Rc::downgrade(self);
        self.exit_choice.connect_selected_notify(move |choice| {
            let Some(this) = weak.upgrade() else { return };
            if this.updating.get() {
                return;
            }
            let index = choice.selected() as usize;
            let ids = this.exit_ids.borrow().clone();
            let target = if index == 0 { String::new() } else { ids.get(index - 1).cloned().unwrap_or_default() };
            let done = if target.is_empty() { tr("Exit node turned off") } else { tr("Using exit node") };
            this.change(vec!["set", &format!("--exit-node={target}")], done);
        });

        self.bind_switch(&self.lan_access, "--exit-node-allow-lan-access", false);
        self.bind_switch(&self.accept_dns, "--accept-dns", false);
        self.bind_switch(&self.accept_routes, "--accept-routes", false);
        self.bind_switch(&self.allow_incoming, "--shields-up", true);
        self.bind_switch(&self.run_ssh, "--ssh", false);

        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.devices_key.borrow_mut().clear();
                this.show_devices();
            }
        });
    }

    /// `tailscale set FLAG=BOOL` from a switch; `inverted` for flags whose meaning is the switch's opposite.
    fn bind_switch(self: &Rc<Self>, switch: &gtk::Switch, flag: &'static str, inverted: bool) {
        let weak = Rc::downgrade(self);
        switch.connect_state_set(move |_, on| {
            let Some(this) = weak.upgrade() else { return glib::Propagation::Stop };
            if this.updating.get() {
                return glib::Propagation::Proceed;
            }
            let value = on != inverted;
            this.change(vec!["set", &format!("{flag}={value}")], tr("Setting saved"));
            glib::Propagation::Stop
        });
    }

    /// Runs a changing command, then refreshes so every control shows the real state.
    fn change(self: &Rc<Self>, args: Vec<&str>, done: String) {
        if self.busy.replace(true) {
            return;
        }
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = cli::run(&argv).await;
            this.busy.set(false);
            match result {
                Ok(_) => {
                    this.setup.set_reveal_child(false);
                    chrome::show_toast(&this.toast, &done);
                }
                Err(CliError::AccessDenied) => {
                    this.setup.set_visible(true);
                    this.setup.set_reveal_child(true);
                    chrome::show_toast(&this.toast, &CliError::AccessDenied.message());
                }
                Err(e) => chrome::show_toast(&this.toast, &e.message()),
            }
            this.refresh_now().await;
        });
    }

    pub fn refresh(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move { this.refresh_now().await });
    }

    async fn refresh_now(self: &Rc<Self>) {
        let status = cli::run(&["status", "--json"]).await;
        let prefs = cli::run(&["debug", "prefs"]).await.ok().and_then(|p| model::parse_prefs(&p).ok());
        match status.map(|json| model::parse_status(&json)) {
            Ok(Ok(status)) => {
                if let Some(prefs) = prefs {
                    *self.prefs.borrow_mut() = prefs;
                }
                self.apply(status);
            }
            Ok(Err(_)) => {
                self.show_problem(&tr("Couldn't read Tailscale's status"), &tr("Try again in a moment."), None)
            }
            Err(CliError::NotInstalled) => self.show_problem(
                &tr("Tailscale isn't installed"),
                &tr("Install Tailscale, then open this window again."),
                None,
            ),
            Err(_) => self.show_problem(
                &tr("Tailscale isn't running"),
                &tr("Start the Tailscale service, then this window connects by itself:"),
                Some(cli::START_SERVICE_COMMAND),
            ),
        }
    }

    fn show_problem(self: &Rc<Self>, title: &str, description: &str, command: Option<&str>) {
        self.placeholder.set_title(title);
        let text = match command {
            Some(c) => format!("{description}\n\n{c}"),
            None => description.to_string(),
        };
        self.placeholder.set_description(&text);
        self.placeholder.set_icon(&gio::ThemedIcon::new("mseries-tailscale-offline-symbolic"));
        self.set_placeholder_button(command.map(|c| PlaceholderAction::Copy(c.to_string())));
        self.pages.set_visible_child_name("placeholder");
    }

    fn set_placeholder_button(&self, action: Option<PlaceholderAction>) {
        self.copy_button.set_visible(matches!(action, Some(PlaceholderAction::Copy(_))));
        self.login_button.set_visible(matches!(action, Some(PlaceholderAction::LogIn)));
        if let Some(PlaceholderAction::Copy(command)) = action {
            *self.placeholder_command.borrow_mut() = command;
        }
    }

    fn log_in(self: &Rc<Self>) {
        let auth = self.status.borrow().as_ref().map(|s| s.auth_url.clone()).unwrap_or_default();
        if !auth.is_empty() {
            self.open(&auth);
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            match cli::login_url().await {
                Ok(url) if !url.is_empty() => this.open(&url),
                Ok(_) => this.refresh_now().await,
                Err(CliError::AccessDenied) => {
                    this.copy(
                        &cli::operator_command(),
                        &tr("Copied the one-time setup command. Run it, then try again."),
                    );
                }
                Err(e) => chrome::show_toast(&this.toast, &e.message()),
            }
        });
    }

    fn apply(self: &Rc<Self>, status: Status) {
        match status.backend {
            Backend::NeedsLogin | Backend::NeedsMachineAuth => {
                let (title, text) = if status.backend == Backend::NeedsLogin {
                    (tr("Log in to Tailscale"), tr("Sign in with your Tailscale account to join your tailnet."))
                } else {
                    (tr("Waiting for approval"), tr("An admin of your tailnet needs to approve this device."))
                };
                self.placeholder.set_title(&title);
                self.placeholder.set_description(&text);
                self.placeholder.set_icon(&gio::ThemedIcon::new("mseries-tailscale-offline-symbolic"));
                self.set_placeholder_button(
                    (status.backend == Backend::NeedsLogin).then_some(PlaceholderAction::LogIn),
                );
                *self.status.borrow_mut() = Some(status);
                self.pages.set_visible_child_name("placeholder");
                return;
            }
            _ => {}
        }
        self.pages.set_visible_child_name("main");
        self.updating.set(true);
        let connected = status.backend == Backend::Running;
        self.status_icon.set_icon_name(Some(if connected {
            "mseries-tailscale-symbolic"
        } else {
            "mseries-tailscale-offline-symbolic"
        }));
        self.status_title.set_label(&match status.backend {
            Backend::Running => tr("Connected"),
            Backend::Starting => tr("Connecting…"),
            _ => tr("Disconnected"),
        });
        let mut detail = Vec::new();
        if !status.tailnet.is_empty() {
            detail.push(status.tailnet.clone());
        }
        if !status.account.is_empty() && status.account != status.tailnet {
            detail.push(status.account.clone());
        }
        if !connected {
            detail.insert(0, tr("Turn on to reach your devices"));
        }
        self.status_detail.set_label(&detail.join(" · "));
        self.connect.set_active(connected || status.backend == Backend::Starting);
        self.connect.set_state(connected);

        if let Some(me) = &status.this_device {
            self.this_name.set_label(&me.name);
            let mut ip = me.ip().to_string();
            if !me.ssh_host.is_empty() && !me.ssh_host.eq_ignore_ascii_case(&me.name) {
                ip = format!("{ip} · {}", me.ssh_host);
            }
            self.this_ip.set_label(&ip);
        }
        self.this_row.set_visible(status.this_device.is_some());

        self.health.set_visible(!status.health.is_empty());
        self.health.set_label(&status.health.join("\n"));

        // Exit node
        let options = status.exit_node_options();
        let mut labels = vec![tr("None")];
        let mut ids = Vec::new();
        for d in &options {
            labels.push(if d.online { d.name.clone() } else { format!("{} ({})", d.name, tr("offline")) });
            ids.push(d.ip().to_string());
        }
        let current = status.current_exit_node().map(|d| d.ip().to_string());
        let selected = current.as_ref().and_then(|ip| ids.iter().position(|i| i == ip)).map(|i| i + 1).unwrap_or(0);
        if *self.exit_ids.borrow() != ids {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            self.exit_choice.set_model(Some(&gtk::StringList::new(&refs)));
            *self.exit_ids.borrow_mut() = ids;
        }
        self.exit_choice.set_selected(selected as u32);
        self.exit_choice.set_sensitive(connected && !options.is_empty());
        self.exit_note.set_visible(options.is_empty());
        self.exit_note.set_label(&tr(
            "No device on your tailnet offers itself as an exit node yet. Turn it on in that device's Tailscale settings.",
        ));
        let prefs = self.prefs.borrow().clone();
        self.lan_access.set_active(prefs.allow_lan_access);
        self.lan_access.set_state(prefs.allow_lan_access);
        self.exit_section.set_visible(true);

        self.accept_dns.set_active(prefs.accept_dns);
        self.accept_dns.set_state(prefs.accept_dns);
        self.accept_routes.set_active(prefs.accept_routes);
        self.accept_routes.set_state(prefs.accept_routes);
        self.allow_incoming.set_active(!prefs.shields_up);
        self.allow_incoming.set_state(!prefs.shields_up);
        self.run_ssh.set_active(prefs.run_ssh);
        self.run_ssh.set_state(prefs.run_ssh);
        self.version.set_label(&if status.version.is_empty() {
            String::new()
        } else {
            format!("Tailscale {}", status.version)
        });
        self.updating.set(false);

        self.devices_header.set_secondary_text(Some(&format!(
            "{} {} · {} {}",
            status.online_count(),
            tr("online"),
            status.peers.len(),
            tr("total")
        )));
        *self.status.borrow_mut() = Some(status);
        self.show_devices();
    }

    fn show_devices(self: &Rc<Self>) {
        let status = self.status.borrow();
        let Some(status) = status.as_ref() else { return };
        let query = self.search.text().to_lowercase();
        let now = glib::DateTime::now_utc().map(|t| t.to_unix()).unwrap_or(0);
        let shown: Vec<&Device> = status
            .peers
            .iter()
            .filter(|d| {
                query.is_empty()
                    || d.name.to_lowercase().contains(&query)
                    || d.ip().contains(&query)
                    || d.os_name().to_lowercase().contains(&query)
            })
            .collect();
        // Rebuild only when something visible changed, so hover and focus survive the 3 s refresh.
        let connected = status.backend == Backend::Running;
        let key = format!(
            "{connected}|{}",
            shown
                .iter()
                .map(|d| format!(
                    "{}:{}:{}:{:?}:{}",
                    d.id,
                    d.online,
                    d.name,
                    d.last_seen.as_ref().map(|_| now / 60),
                    d.exit_node
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        if *self.devices_key.borrow() == key {
            return;
        }
        *self.devices_key.borrow_mut() = key;
        while let Some(row) = self.device_list.first_child() {
            self.device_list.remove(&row);
        }
        if shown.is_empty() {
            let empty = gtk::Label::builder()
                .label(if query.is_empty() { tr("No other devices yet") } else { tr("No matching devices") })
                .margin_top(18)
                .margin_bottom(18)
                .build();
            empty.add_css_class(granite::CssClassDIM);
            let row = gtk::ListBoxRow::builder().child(&empty).activatable(false).build();
            self.device_list.append(&row);
            return;
        }
        for device in shown {
            self.device_list.append(&self.device_row(device, now, connected));
        }
    }

    fn device_row(self: &Rc<Self>, device: &Device, now: i64, connected: bool) -> gtk::ListBoxRow {
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class(if device.online { granite::CssClassSUCCESS } else { granite::CssClassDIM });
        dot.set_tooltip_text(Some(&if device.online { tr("Online") } else { tr("Offline") }));
        dot.set_valign(gtk::Align::Center);
        dot.add_css_class("mseries-tailscale-dot");

        let name =
            gtk::Label::builder().label(&device.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
        name.add_css_class("heading");
        let seen =
            device.last_seen.as_deref().and_then(|s| glib::DateTime::from_iso8601(s, None).ok()).map(|t| t.to_unix());
        let detail = gtk::Label::builder()
            .label(format!(
                "{} · {} · {}",
                device.ip(),
                device.os_name(),
                tr(&model::presence_text(device.online, seen, now))
            ))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        detail.add_css_class(granite::CssClassDIM);
        detail.add_css_class(granite::CssClassSMALL);
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        let title_line = gtk::Box::builder().spacing(6).build();
        title_line.append(&name);
        if device.exit_node {
            let badge = gtk::Label::new(Some(&tr("Exit node")));
            badge.add_css_class("mseries-tailscale-badge");
            badge.set_valign(gtk::Align::Center);
            title_line.append(&badge);
        }
        text.append(&title_line);
        text.append(&detail);

        let copy_ip = icon_button("edit-copy-symbolic", &format!("{} {}", tr("Copy IP Address"), device.ip()));
        let ssh = device.ssh_command();
        let copy_ssh = icon_button("utilities-terminal-symbolic", &format!("{} “{ssh}”", tr("Copy")));
        let (ip, ip_done) = (device.ip().to_string(), format!("{} {}", tr("Copied"), device.ip()));
        let weak = Rc::downgrade(self);
        copy_ip.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.copy(&ip, &ip_done);
            }
        });
        let ssh_done = format!("{} “{ssh}”", tr("Copied"));
        let weak = Rc::downgrade(self);
        copy_ssh.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.copy(&ssh, &ssh_done);
            }
        });

        let line =
            gtk::Box::builder().spacing(12).margin_top(9).margin_bottom(9).margin_start(12).margin_end(6).build();
        line.append(&dot);
        line.append(&text);
        line.append(&copy_ip);
        line.append(&copy_ssh);
        if !device.online || !connected {
            name.add_css_class(granite::CssClassDIM);
        }

        // Clicking the row copies the IP; the context menu has both.
        let row = gtk::ListBoxRow::builder().child(&line).activatable(true).build();
        row.set_tooltip_text(Some(&format!("{} {}", tr("Click to copy"), device.ip())));
        let (ip, ip_done) = (device.ip().to_string(), format!("{} {}", tr("Copied"), device.ip()));
        let weak = Rc::downgrade(self);
        row.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.copy(&ip, &ip_done);
            }
        });
        row
    }

    fn copy(&self, text: &str, confirmation: &str) {
        self.window.clipboard().set_text(text);
        chrome::show_toast(&self.toast, confirmation);
    }

    fn open(&self, uri: &str) {
        gtk::UriLauncher::new(uri).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
    }
}

enum PlaceholderAction {
    Copy(String),
    LogIn,
}

fn install_style(window: &gtk::ApplicationWindow) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../../data/style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::prelude::WidgetExt::display(window),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn card_box() -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
    b.add_css_class(granite::CssClassCARD);
    b.add_css_class("mseries-tailscale-card");
    b
}

fn section(header: &impl IsA<gtk::Widget>, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    b.append(header);
    b.append(body);
    b
}

fn setting_row(title: &str, description: Option<&str>, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let label = gtk::Label::builder().label(title).xalign(0.0).wrap(true).build();
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    text.set_valign(gtk::Align::Center);
    text.append(&label);
    if let Some(d) = description {
        let sub = gtk::Label::builder().label(d).xalign(0.0).wrap(true).build();
        sub.add_css_class(granite::CssClassDIM);
        sub.add_css_class(granite::CssClassSMALL);
        text.append(&sub);
    }
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&text);
    row.append(control);
    if let Some(sw) = control.dynamic_cast_ref::<gtk::Switch>() {
        set_accessible_label(sw, title);
    }
    row
}

fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(label));
    set_accessible_label(&b, label);
    b
}
