//! Main window. One column (utility window): status card, exit node, devices, settings.
//!
//! State comes from `tailscale status --json` and `tailscale debug prefs`, every 3 s while the window is mapped
//! and right after every change. Controls never hold their own state: each refresh writes the real state back,
//! so a refused change springs back and the reason shows in a banner or toast.

use crate::cli::{self, CliError};
use crate::model::{self, Backend, Device, Prefs, Status};
use crate::net;
use crate::strings::tr;
use granite::prelude::{HeaderLabelExt, PlaceholderExt};
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{appearance, chrome, set_accessible_label};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
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
    route_line: gtk::Label,
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
    // NetworkManager VPNs
    vpn_list: gtk::Box,
    vpn_key: RefCell<String>,
    vpn_pending: RefCell<std::collections::HashSet<String>>,
    vpn_live: RefCell<HashMap<String, Live>>,
    vpn_failures: RefCell<HashMap<String, String>>,
    vpn_connecting: RefCell<HashSet<String>>,
    vpn_stopping: RefCell<HashSet<String>>,
    vpn_ticks: RefCell<Vec<Tick>>,
    vpn_banner: gtk::Box,
    vpn_banner_text: gtk::Label,
    route_now: gtk::Label,
    sessions: Sessions,
    ts_timer: gtk::Label,
    remote_list: gtk::Box,
    remote_key: RefCell<String>,
    ticker: RefCell<Option<mseries_pantheon_kit::Polling>>,
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
    refreshing: Cell<bool>,
    network: RefCell<Option<glib::SignalHandlerId>>,
    sheet: RefCell<Option<gtk::Window>>,
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
        let route_line = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
        route_line.add_css_class(granite::CssClassACCENT);
        titles.append(&route_line);
        let ts_timer = gtk::Label::builder().xalign(0.0).visible(false).build();
        ts_timer.add_css_class(granite::CssClassNUMERIC);
        ts_timer.add_css_class(granite::CssClassDIM);
        titles.append(&ts_timer);
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
        let exit_header = granite::HeaderLabel::new(&tr("Route All Traffic Through"));
        exit_header.set_secondary_text(Some(&tr("Send all internet traffic through another device on your tailnet")));
        let exit_choice = gtk::DropDown::from_strings(&[&tr("None")]);
        exit_choice.set_valign(gtk::Align::Center);
        set_accessible_label(&exit_choice, &tr("Exit node"));
        let exit_note = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
        exit_note.add_css_class(granite::CssClassDIM);
        let lan_access = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let exit_card = card_box();
        exit_card.append(&setting_row(&tr("Exit node"), None, &exit_choice));
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

        // ---- Other VPNs (NetworkManager) ---------------------------------------------------------------------
        let vpn_header = granite::HeaderLabel::new(&tr("VPN Connections"));
        vpn_header.set_secondary_text(Some(&tr("Other VPNs on this computer")));
        let vpn_list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
        let add_vpn = gtk::Button::with_label(&tr("Add VPN…"));
        add_vpn.set_halign(gtk::Align::Start);
        add_vpn.set_tooltip_text(Some(&tr("Opens Network settings")));
        add_vpn.connect_clicked(|b| {
            let window = b.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new("settings://network").launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
        });
        let vpn_banner_text = gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).build();
        let vpn_banner = card_box();
        vpn_banner.add_css_class("mseries-tailscale-setup");
        let banner_line = gtk::Box::builder().spacing(12).build();
        banner_line.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        banner_line.append(&vpn_banner_text);
        vpn_banner.append(&banner_line);
        vpn_banner.set_visible(false);
        let route_now = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
        route_now.add_css_class("heading");
        let vpn_body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
        vpn_body.append(&vpn_banner);
        vpn_body.append(&route_now);
        vpn_body.append(&vpn_list);
        vpn_body.append(&add_vpn);
        let vpn_section = section(&vpn_header, &vpn_body);

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
        let remote_header = granite::HeaderLabel::new(&tr("Remote Into a Mac"));
        remote_header.set_secondary_text(Some(&tr("Screen Sharing over Tailscale, from anywhere")));
        let remote_list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
        let remote_section = section(&remote_header, &remote_list);
        column.append(&setup);
        column.append(&health);
        column.append(&remote_section);
        column.append(&vpn_section);
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
            route_line,
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
            vpn_list,
            vpn_key: RefCell::new(String::new()),
            vpn_pending: RefCell::new(Default::default()),
            vpn_live: RefCell::new(HashMap::new()),
            vpn_failures: RefCell::new(HashMap::new()),
            vpn_connecting: RefCell::new(HashSet::new()),
            vpn_stopping: RefCell::new(HashSet::new()),
            vpn_ticks: RefCell::new(Vec::new()),
            vpn_banner,
            vpn_banner_text,
            route_now,
            sessions: Sessions::load(),
            ts_timer,
            remote_list,
            remote_key: RefCell::new(String::new()),
            ticker: RefCell::new(None),
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
            refreshing: Cell::new(false),
            network: RefCell::new(None),
            sheet: RefCell::new(None),
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
        let weak = Rc::downgrade(&this);
        let ticker = mseries_pantheon_kit::Polling::new(Duration::from_secs(1), move || {
            if let Some(this) = weak.upgrade() {
                this.tick();
            }
        });
        ticker.bind_to_widget_mapped(&this.window);
        *this.ticker.borrow_mut() = Some(ticker);
        // Wi-Fi changes make tailscaled reconnect; show it at once instead of waiting for the next poll.
        let weak = Rc::downgrade(&this);
        let id = gio::NetworkMonitor::default().connect_network_changed(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.refresh();
            }
        });
        *this.network.borrow_mut() = Some(id);
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
        if args.first().map(String::as_str) == Some("up") {
            self.status_title.set_label(&tr("Connecting…"));
        } else if args.first().map(String::as_str) == Some("down") {
            self.status_title.set_label(&tr("Disconnecting…"));
        }
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
        // A change in flight refreshes when it ends; never stack refreshes behind a slow command.
        if self.busy.get() || self.refreshing.get() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move { this.refresh_now().await });
    }

    async fn refresh_now(self: &Rc<Self>) {
        self.refreshing.set(true);
        self.refresh_vpns().await;
        let status = cli::run(&["status", "--json"]).await;
        self.refreshing.set(false);
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
            Err(CliError::TimedOut(_)) => {
                // Keep the last good view; say why it is not updating.
                if self.status.borrow().is_some() {
                    self.status_detail.set_label(&tr("Tailscale isn't answering. Retrying…"));
                } else {
                    self.show_problem(
                        &tr("Tailscale isn't answering"),
                        &tr("The Tailscale service is slow to respond. This window keeps trying."),
                        None,
                    );
                }
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
        let route = status.current_exit_node().filter(|_| connected).map(|d| d.name.clone());
        self.route_line.set_visible(route.is_some());
        if let Some(name) = &route {
            self.route_line.set_label(&format!("{} {name}", tr("All internet traffic goes through")));
        }
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
            "No device on your tailnet offers to route traffic yet. On an always-on device, like sdolbec-ser8, run \
             “sudo tailscale set --advertise-exit-node”, then approve it under Edit route settings in the Admin Console.",
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
        if connected {
            self.sessions.start("tailscale", 1, now_unix());
        } else {
            self.sessions.stop("tailscale");
        }
        self.sessions.save();
        *self.status.borrow_mut() = Some(status);
        self.show_devices();
        self.show_remote();
        self.tick();
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

    async fn refresh_vpns(self: &Rc<Self>) {
        let listed = cli::run_program(&["nmcli", "-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"], 8).await;
        let mut vpns = match listed {
            Ok((true, out, _)) => model::parse_vpn_list(&out),
            _ => Vec::new(),
        };
        let active = match cli::run_program(
            &["nmcli", "-t", "-f", "UUID,STATE,DEVICE,TYPE", "connection", "show", "--active"],
            8,
        )
        .await
        {
            Ok((true, out, _)) => model::parse_active(&out),
            _ => Default::default(),
        };
        let snap = net::probe().await;
        let now = now_unix();
        let mut lives = Vec::new();
        for vpn in &mut vpns {
            if let Ok((true, out, _)) =
                cli::run_program(&["nmcli", "-g", "ipv4.never-default", "connection", "show", &vpn.uuid], 8).await
            {
                vpn.all_traffic = out.trim() == "no";
            }
            if let Ok((true, out, _)) =
                cli::run_program(&["nmcli", "-g", "vpn.data", "connection", "show", &vpn.uuid], 8).await
            {
                vpn.secrets_saved = model::secrets_saved(&out);
            }
            // Never trust NetworkManager's "activated" alone: find the tunnel and check it is up.
            let (nm_state, device, kind) = active.get(&vpn.uuid).cloned().unwrap_or_default();
            let mut iface = None;
            if nm_state == "activated" {
                if kind == "wireguard" {
                    iface = snap.iface(&device).filter(|i| i.up && !i.ipv4.is_empty()).map(|i| i.name.clone());
                } else if let Ok((true, out, _)) =
                    cli::run_program(&["nmcli", "-g", "IP4.ADDRESS", "connection", "show", &vpn.uuid], 8).await
                {
                    let addr = out.lines().next().unwrap_or_default().trim().to_string();
                    iface = (!addr.is_empty())
                        .then(|| snap.iface_with_address(&addr))
                        .flatten()
                        .filter(|i| i.up)
                        .map(|i| i.name.clone());
                }
            }
            let connecting = self.vpn_connecting.borrow().contains(&vpn.uuid) || nm_state == "activating";
            let previous = self.vpn_live.borrow().get(&vpn.uuid).cloned();
            let live = if let Some(iface) = iface.clone() {
                self.vpn_failures.borrow_mut().remove(&vpn.uuid);
                Live::Connected(iface)
            } else if connecting {
                Live::Connecting
            } else if nm_state == "activated" {
                Live::Dropped
            } else if let Some(reason) = self.vpn_failures.borrow().get(&vpn.uuid).cloned() {
                Live::Failed(reason)
            } else {
                Live::Off
            };
            // It was up and went away without us turning it off: record why.
            let live = match (&previous, &live) {
                (Some(Live::Connected(_)) | Some(Live::Connecting) | Some(Live::Dropped), Live::Off)
                    if !self.vpn_stopping.borrow().contains(&vpn.uuid) =>
                {
                    let reason = failure_reason(&vpn.name, &snap, iface.as_deref()).await;
                    self.vpn_failures.borrow_mut().insert(vpn.uuid.clone(), reason.clone());
                    Live::Failed(reason)
                }
                _ => live,
            };
            if matches!(live, Live::Off) {
                self.vpn_stopping.borrow_mut().remove(&vpn.uuid);
            }
            // Session timer: keep the start while the same tunnel interface stays up.
            if let Live::Connected(name) = &live {
                let index = snap.iface(name).map(|i| i.index).unwrap_or(0);
                self.sessions.start(&vpn.uuid, index, now);
            } else {
                self.sessions.stop(&vpn.uuid);
            }
            self.vpn_live.borrow_mut().insert(vpn.uuid.clone(), live.clone());
            lives.push(live);
        }
        self.sessions.save();
        self.show_vpn_overview(&vpns, &lives, &snap);

        let pending = self.vpn_pending.borrow().clone();
        let key = format!("{vpns:?}|{lives:?}|{pending:?}");
        if *self.vpn_key.borrow() == key {
            return;
        }
        *self.vpn_key.borrow_mut() = key;
        while let Some(child) = self.vpn_list.first_child() {
            self.vpn_list.remove(&child);
        }
        self.vpn_ticks.borrow_mut().clear();
        if vpns.is_empty() {
            let none = gtk::Label::builder().label(tr("No other VPNs set up")).xalign(0.0).build();
            none.add_css_class(granite::CssClassDIM);
            self.vpn_list.append(&none);
        }
        for (vpn, live) in vpns.iter().zip(lives) {
            let needs_reconnect = pending.contains(&vpn.uuid) && matches!(live, Live::Connected(_));
            self.vpn_list.append(&self.vpn_card(vpn, &live, needs_reconnect));
        }
        self.tick();
    }

    /// Where internet traffic goes now, and a warning when two VPNs fight.
    fn show_vpn_overview(&self, vpns: &[model::Vpn], lives: &[Live], snap: &net::Snapshot) {
        let ours: Vec<(&model::Vpn, &Live)> = vpns.iter().zip(lives).collect();
        let iface_name = |iface: &str| -> String {
            if let Some((vpn, _)) = ours.iter().find(|(_, l)| matches!(l, Live::Connected(i) if i == iface)) {
                return model::display_name(&vpn.name);
            }
            if let Some((vpn, _)) = ours.iter().find(|(v, _)| v.name == iface) {
                return model::display_name(&vpn.name);
            }
            match iface {
                "tailscale0" => tr("Tailscale exit node"),
                i if snap.tunnels().any(|t| t.name == i) => net::tunnel_owner(i),
                _ => tr("this computer's own connection (no VPN)"),
            }
        };
        match &snap.internet_iface {
            Some(iface) => {
                self.route_now.set_label(&format!("{} {}", tr("Internet traffic now goes through"), iface_name(iface)));
                self.route_now.set_visible(true);
            }
            None => self.route_now.set_visible(false),
        }

        // Tunnels up right now: our connected VPNs plus tunnels nobody listed (another app's VPN).
        let mut up: Vec<String> = ours
            .iter()
            .filter(|(_, l)| matches!(l, Live::Connected(_)))
            .map(|(v, _)| model::display_name(&v.name))
            .collect();
        for t in snap.tunnels() {
            let ours_iface =
                ours.iter().any(|(v, l)| matches!(l, Live::Connected(i) if *i == t.name) || v.name == t.name);
            if !ours_iface && t.up {
                up.push(net::tunnel_owner(&t.name));
            }
        }
        let knocked = ours.iter().find(|(_, l)| matches!(l, Live::Dropped | Live::Failed(_)));
        let message = match (knocked, up.as_slice()) {
            (Some((victim, _)), [other, ..]) => Some(format!(
                "{other} {} {} {}",
                tr("started its own VPN; it knocked"),
                model::display_name(&victim.name),
                tr("off. Use one VPN at a time.")
            )),
            (None, [a, b, ..]) => Some(format!(
                "{a} {} {b} {}",
                tr("and"),
                tr("are both on. Two VPNs at once fight over your traffic; use one at a time.")
            )),
            _ => None,
        };
        self.vpn_banner.set_visible(message.is_some());
        if let Some(m) = message {
            self.vpn_banner_text.set_label(&m);
        }
    }

    fn vpn_card(self: &Rc<Self>, vpn: &model::Vpn, live: &Live, needs_reconnect: bool) -> gtk::Box {
        let card = card_box();
        let name = model::display_name(&vpn.name);
        let on_now = !matches!(live, Live::Off | Live::Failed(_));
        let on = gtk::Switch::builder().valign(gtk::Align::Center).active(on_now).state(on_now).build();
        let gear = icon_button("emblem-system-symbolic", &format!("{name} {}", tr("Settings")));
        let controls = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).build();
        if matches!(live, Live::Connecting) {
            let spinner = gtk::Spinner::builder().spinning(true).valign(gtk::Align::Center).build();
            controls.append(&spinner);
        }
        controls.append(&gear);
        controls.append(&on);
        set_accessible_label(&on, &format!("{} {name}", tr("Connect")));
        let (status_text, status_class) = match live {
            Live::Connected(iface) => (format!("{} · {iface}", tr("Connected")), Some(granite::CssClassSUCCESS)),
            Live::Connecting => (tr("Connecting…"), None),
            Live::Dropped => (tr("Dropped: on, but not carrying any traffic"), Some(granite::CssClassERROR)),
            Live::Failed(reason) => (format!("{}: {reason}", tr("Failed")), Some(granite::CssClassERROR)),
            Live::Off => (tr("Not connected"), None),
        };
        let row = setting_row(&name, Some(&status_text), &controls);
        if let Some(class) = status_class {
            // The description label is the second child of the text box.
            if let Some(desc) = row.first_child().and_then(|t| t.last_child()) {
                desc.remove_css_class(granite::CssClassDIM);
                desc.add_css_class(class);
            }
        }
        card.append(&row);

        if let Live::Connected(iface) = live {
            let timer = gtk::Label::builder().xalign(0.0).hexpand(true).build();
            timer.add_css_class(granite::CssClassNUMERIC);
            timer.add_css_class("title-4");
            let bytes = gtk::Label::builder().xalign(1.0).build();
            bytes.add_css_class(granite::CssClassNUMERIC);
            bytes.add_css_class(granite::CssClassDIM);
            let line = gtk::Box::builder().spacing(12).build();
            line.append(&timer);
            line.append(&bytes);
            card.append(&line);
            self.vpn_ticks.borrow_mut().push(Tick { key: vpn.uuid.clone(), iface: iface.clone(), timer, bytes });
        }

        let (weak, edit_vpn) = (Rc::downgrade(self), vpn.clone());
        gear.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                // The sheet covers plugin VPNs (L2TP and friends); WireGuard opens the system editor.
                if edit_vpn.kind_has_routing() {
                    this.open_vpn_sheet(&edit_vpn);
                } else {
                    this.edit_vpn(&edit_vpn.uuid);
                }
            }
        });
        if vpn.kind_has_routing() {
            card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let all = gtk::Switch::builder()
                .valign(gtk::Align::Center)
                .active(vpn.all_traffic)
                .state(vpn.all_traffic)
                .build();
            let mode = if vpn.all_traffic {
                format!("{} {name}", tr("On: all internet traffic goes through"))
            } else {
                format!("{} {name}", tr("Off: only its own networks go through"))
            };
            card.append(&setting_row(&tr("Send all traffic over VPN connection"), Some(&mode), &all));
            let (weak, uuid, connected) = (Rc::downgrade(self), vpn.uuid.clone(), matches!(live, Live::Connected(_)));
            all.connect_state_set(move |_, everything| {
                if let Some(this) = weak.upgrade() {
                    let never_default = if everything { "no" } else { "yes" };
                    let done = if everything {
                        tr("All traffic will go over the VPN")
                    } else {
                        tr("Only the VPN's own networks will use it")
                    };
                    if connected {
                        this.vpn_pending.borrow_mut().insert(uuid.clone());
                    }
                    this.vpn_command(
                        &uuid,
                        vec![
                            "nmcli",
                            "connection",
                            "modify",
                            "uuid",
                            &uuid,
                            "ipv4.never-default",
                            never_default,
                            "ipv6.never-default",
                            never_default,
                        ],
                        done,
                        false,
                    );
                }
                glib::Propagation::Stop
            });
        }
        if !vpn.secrets_saved {
            card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let save = gtk::Button::with_label(&tr("Save Passwords…"));
            card.append(&note_line(
                &tr("Passwords aren't saved, so connecting from here can't work. Save them once."),
                &save,
            ));
            let (weak, edit_vpn) = (Rc::downgrade(self), vpn.clone());
            save.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.open_vpn_sheet(&edit_vpn);
                }
            });
        }
        let broken = matches!(live, Live::Dropped | Live::Failed(_));
        if needs_reconnect || broken {
            let reconnect = gtk::Button::with_label(&tr("Reconnect"));
            reconnect.add_css_class(granite::CssClassSUGGESTED);
            let text = if broken { tr("Reconnect to bring it back.") } else { tr("Reconnect to use the new setting.") };
            card.append(&note_line(&text, &reconnect));
            let (weak, uuid, label) = (Rc::downgrade(self), vpn.uuid.clone(), name.clone());
            reconnect.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.vpn_failures.borrow_mut().remove(&uuid);
                    this.vpn_connecting.borrow_mut().insert(uuid.clone());
                    this.vpn_command(
                        &uuid,
                        vec![
                            "sh",
                            "-c",
                            "nmcli connection down uuid \"$0\" >/dev/null 2>&1; nmcli connection up uuid \"$0\"",
                            &uuid,
                        ],
                        format!("{} {label}", tr("Reconnected")),
                        true,
                    );
                }
            });
        }

        let (weak, uuid) = (Rc::downgrade(self), vpn.uuid.clone());
        on.connect_state_set(move |_, active| {
            if let Some(this) = weak.upgrade() {
                let verb = if active { "up" } else { "down" };
                let done = if active { tr("Connected to") } else { tr("Disconnected from") };
                this.vpn_failures.borrow_mut().remove(&uuid);
                if active {
                    this.vpn_connecting.borrow_mut().insert(uuid.clone());
                } else {
                    this.vpn_stopping.borrow_mut().insert(uuid.clone());
                }
                this.vpn_key.borrow_mut().clear();
                this.vpn_command(
                    &uuid,
                    vec!["nmcli", "connection", verb, "uuid", &uuid],
                    format!("{done} {name}"),
                    false,
                );
                // Show "Connecting…" right away instead of waiting for nmcli.
                let this2 = this.clone();
                glib::spawn_future_local(async move { this2.refresh_vpns().await });
            }
            glib::Propagation::Stop
        });
        card
    }

    /// One-second update of session timers and traffic counters (no rebuilds).
    fn tick(&self) {
        let now = now_unix();
        for t in self.vpn_ticks.borrow().iter() {
            if let Some(start) = self.sessions.started(&t.key) {
                t.timer.set_label(&net::duration_text(now - start));
            }
            if let Some((rx, tx)) = net::counters(&t.iface) {
                t.bytes.set_label(&format!("↓ {}   ↑ {}", net::bytes_text(rx), net::bytes_text(tx)));
            }
        }
        match self.sessions.started("tailscale") {
            Some(start) if self.status_is_running() => {
                self.ts_timer.set_label(&format!("{} {}", tr("Connected for"), net::duration_text(now - start)));
                self.ts_timer.set_visible(true);
            }
            _ => self.ts_timer.set_visible(false),
        }
    }

    fn status_is_running(&self) -> bool {
        self.status.borrow().as_ref().is_some_and(|s| s.backend == Backend::Running)
    }

    /// "Remote Into a Mac": the tailnet's Macs with Screen Sharing (VNC over Tailscale).
    fn show_remote(self: &Rc<Self>) {
        let status = self.status.borrow();
        let running = status.as_ref().is_some_and(|s| s.backend == Backend::Running);
        let macs: Vec<Device> = status
            .as_ref()
            .map(|s| {
                // The Mac mini at BFS always; other Macs only while online (the full list is under Devices).
                let mut m: Vec<Device> = s
                    .peers
                    .iter()
                    .filter(|d| d.os == "macOS" && (d.online || d.ssh_host == "csp-bfs-mini"))
                    .cloned()
                    .collect();
                // The Mac mini at BFS first, then online ones.
                m.sort_by_key(|d| (d.ssh_host != "csp-bfs-mini", !d.online, d.name.to_lowercase()));
                m
            })
            .unwrap_or_default();
        drop(status);
        let have_viewer = glib::find_program_in_path("remmina").is_some();
        let key = format!(
            "{running}|{have_viewer}|{}",
            macs.iter().map(|d| format!("{}:{}", d.id, d.online)).collect::<Vec<_>>().join(",")
        );
        if *self.remote_key.borrow() == key {
            return;
        }
        *self.remote_key.borrow_mut() = key;
        while let Some(child) = self.remote_list.first_child() {
            self.remote_list.remove(&child);
        }
        if !running {
            let turn_on = gtk::Button::with_label(&tr("Turn On Tailscale"));
            turn_on.add_css_class(granite::CssClassSUGGESTED);
            let card = card_box();
            card.append(&note_line(&tr("Tailscale is off. Turn it on to reach your Macs from anywhere."), &turn_on));
            self.remote_list.append(&card);
            let weak = Rc::downgrade(self);
            turn_on.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.change(vec!["up"], tr("Connected"));
                }
            });
        }
        if !have_viewer {
            let copy = gtk::Button::with_label(&tr("Copy Command"));
            let card = card_box();
            card.append(&note_line(
                &tr("Screen Sharing needs the Remmina app. Install it once: sudo apt install remmina remmina-plugin-vnc"),
                &copy,
            ));
            self.remote_list.append(&card);
            let weak = Rc::downgrade(self);
            copy.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.copy(
                        "sudo apt install remmina remmina-plugin-vnc",
                        &tr("Copied. Paste it in Terminal, then come back."),
                    );
                }
            });
        }
        if macs.is_empty() {
            return;
        }
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list.add_css_class("boxed-list");
        list.add_css_class(granite::CssClassCARD);
        list.add_css_class("mseries-tailscale-devices");
        let now = now_unix();
        for mac in macs {
            let dot = gtk::Label::new(Some("●"));
            dot.add_css_class("mseries-tailscale-dot");
            dot.add_css_class(if mac.online && running { granite::CssClassSUCCESS } else { granite::CssClassDIM });
            let name =
                gtk::Label::builder().label(&mac.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
            name.add_css_class("heading");
            let seen =
                mac.last_seen.as_deref().and_then(|s| glib::DateTime::from_iso8601(s, None).ok()).map(|t| t.to_unix());
            let detail = gtk::Label::builder()
                .label(format!(
                    "{} · {}",
                    mac.ip(),
                    if running { tr(&model::presence_text(mac.online, seen, now)) } else { tr("Tailscale is off") }
                ))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            detail.add_css_class(granite::CssClassDIM);
            detail.add_css_class(granite::CssClassSMALL);
            let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
            text.append(&name);
            text.append(&detail);
            let share = gtk::Button::with_label(&tr("Screen Sharing"));
            share.set_valign(gtk::Align::Center);
            if mac.online && running {
                share.add_css_class(granite::CssClassSUGGESTED);
            }
            share.set_tooltip_text(Some(&format!("vnc://{}", mac.ip())));
            let ssh = icon_button("utilities-terminal-symbolic", &format!("{} “{}”", tr("Copy"), mac.ssh_command()));
            let line =
                gtk::Box::builder().spacing(12).margin_top(9).margin_bottom(9).margin_start(12).margin_end(6).build();
            line.append(&dot);
            line.append(&text);
            line.append(&share);
            line.append(&ssh);
            list.append(&gtk::ListBoxRow::builder().child(&line).activatable(false).build());
            let (weak, ip, label) = (Rc::downgrade(self), mac.ip().to_string(), mac.name.clone());
            share.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.screen_share(&ip, &label);
                }
            });
            let (weak, cmd) = (Rc::downgrade(self), mac.ssh_command());
            ssh.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.copy(&cmd, &format!("{} “{cmd}”", tr("Copied")));
                }
            });
        }
        self.remote_list.append(&list);
    }

    fn screen_share(&self, ip: &str, name: &str) {
        if !self.status_is_running() {
            chrome::show_toast(&self.toast, &tr("Turn on Tailscale first."));
            return;
        }
        if glib::find_program_in_path("remmina").is_none() {
            self.copy(
                "sudo apt install remmina remmina-plugin-vnc",
                &tr("Remmina isn't installed. Copied the install command."),
            );
            return;
        }
        let uri = format!("vnc://{ip}");
        match gio::Subprocess::newv(&["remmina".as_ref(), "-c".as_ref(), uri.as_ref()], gio::SubprocessFlags::NONE) {
            Ok(_) => chrome::show_toast(&self.toast, &format!("{} {name}…", tr("Opening Screen Sharing on"))),
            Err(e) => chrome::show_toast(&self.toast, e.message()),
        }
    }

    pub fn open_vpn_sheet(self: &Rc<Self>, vpn: &model::Vpn) {
        let weak = Rc::downgrade(self);
        let uuid = vpn.uuid.clone();
        let sheet = super::vpn_edit::open(self.window.upcast_ref(), &self.toast, vpn, move |active| {
            if let Some(this) = weak.upgrade() {
                if active {
                    // New settings apply on the next connect; offer Reconnect on the card.
                    this.vpn_pending.borrow_mut().insert(uuid.clone());
                }
                this.vpn_key.borrow_mut().clear();
                this.refresh();
            }
        });
        *self.sheet.borrow_mut() = Some(sheet);
    }

    /// The first VPN, for fixture screenshots of the edit sheet.
    pub fn open_first_vpn_sheet(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok((true, out, _)) =
                cli::run_program(&["nmcli", "-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"], 8).await
                && let Some(vpn) = model::parse_vpn_list(&out).into_iter().next()
            {
                this.open_vpn_sheet(&vpn);
            }
        });
    }

    pub fn sheet(&self) -> Option<gtk::Window> {
        self.sheet.borrow().clone()
    }

    /// Opens NetworkManager's editor for one connection, where "Store the password for all users" lives.
    fn edit_vpn(&self, uuid: &str) {
        if let Err(e) = gio::Subprocess::newv(
            &[std::ffi::OsStr::new("nm-connection-editor"), "--edit".as_ref(), uuid.as_ref()],
            gio::SubprocessFlags::NONE,
        ) {
            chrome::show_toast(&self.toast, &format!("{} {}", tr("Couldn't open VPN settings:"), e.message()));
        }
    }

    /// Runs an `nmcli` change (which may ask for a password through polkit), then refreshes.
    fn vpn_command(self: &Rc<Self>, uuid: &str, argv: Vec<&str>, done: String, clears_pending: bool) {
        let uuid = uuid.to_string();
        let argv: Vec<String> = argv.into_iter().map(String::from).collect();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            // Connecting an L2TP/IPsec VPN takes a while; a polkit prompt waits for the user.
            let result = cli::run_program(&refs, 90).await;
            this.vpn_connecting.borrow_mut().remove(&uuid);
            match result {
                Ok((true, _, _)) => {
                    if clears_pending {
                        this.vpn_pending.borrow_mut().remove(&uuid);
                    }
                    chrome::show_toast(&this.toast, &done);
                }
                Ok((false, _, err)) if model::needs_secrets(&err) => {
                    chrome::show_toast(
                        &this.toast,
                        &tr("This VPN needs its passwords saved first. Opening its settings…"),
                    );
                    this.edit_vpn(&uuid);
                }
                Ok((false, _, err)) => {
                    let first = err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("The VPN change failed.");
                    let first = first.trim_start_matches("Error: ").to_string();
                    if refs.iter().any(|a| *a == "up" || a.contains("connection up")) {
                        let snap = net::probe().await;
                        let others: Vec<String> = snap
                            .tunnels()
                            .filter(|t| t.up && !t.name.starts_with("ppp"))
                            .map(|t| net::tunnel_owner(&t.name))
                            .collect();
                        let reason =
                            if others.is_empty() { first.clone() } else { model::classify_vpn_failure("", &others) };
                        this.vpn_failures.borrow_mut().insert(uuid.clone(), reason);
                    }
                    chrome::show_toast(&this.toast, &first);
                }
                Err(e) => chrome::show_toast(&this.toast, &e.message()),
            }
            this.vpn_key.borrow_mut().clear();
            this.refresh_vpns().await;
        });
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

/// What a VPN is really doing, from the kernel's point of view.
#[derive(Debug, Clone, PartialEq)]
enum Live {
    /// Its tunnel interface is up.
    Connected(String),
    Connecting,
    /// NetworkManager says on, but no tunnel carries traffic.
    Dropped,
    Failed(String),
    Off,
}

/// Labels the one-second timer updates.
struct Tick {
    key: String,
    iface: String,
    timer: gtk::Label,
    bytes: gtk::Label,
}

/// Session start times, kept across app restarts (keyed by connection and tunnel interface index, so a
/// reconnect starts a new session).
struct Sessions {
    path: std::path::PathBuf,
    map: RefCell<HashMap<String, (u32, i64)>>,
    dirty: Cell<bool>,
}

impl Sessions {
    fn load() -> Self {
        let path = mseries_pantheon_kit::settings::app_config_dir(APP_ID).join("sessions.json");
        let map = if std::env::var_os("MSERIES_TAILSCALE_FIXTURES").is_some() {
            // Fixture screenshots show a session that has been up for a while.
            HashMap::new()
        } else {
            mseries_pantheon_kit::settings::load_json(&path)
        };
        Self { path, map: RefCell::new(map), dirty: Cell::new(false) }
    }

    fn start(&self, key: &str, index: u32, now: i64) {
        let mut map = self.map.borrow_mut();
        if map.get(key).is_some_and(|(i, _)| *i == index) {
            return;
        }
        let start = if std::env::var_os("MSERIES_TAILSCALE_FIXTURES").is_some() { now - 4 * 3600 - 249 } else { now };
        map.insert(key.into(), (index, start));
        self.dirty.set(true);
    }

    fn stop(&self, key: &str) {
        if self.map.borrow_mut().remove(key).is_some() {
            self.dirty.set(true);
        }
    }

    fn started(&self, key: &str) -> Option<i64> {
        self.map.borrow().get(key).map(|(_, s)| *s)
    }

    fn save(&self) {
        if self.dirty.replace(false) && std::env::var_os("MSERIES_TAILSCALE_FIXTURES").is_none() {
            let _ = mseries_pantheon_kit::settings::save_json(&self.path, &*self.map.borrow());
        }
    }
}

fn now_unix() -> i64 {
    glib::DateTime::now_utc().map(|t| t.to_unix()).unwrap_or(0)
}

/// Reason for a VPN that dropped on its own, from the last few minutes of logs.
async fn failure_reason(_name: &str, snap: &net::Snapshot, _iface: Option<&str>) -> String {
    let others: Vec<String> =
        snap.tunnels().filter(|t| t.up && !t.name.starts_with("ppp")).map(|t| net::tunnel_owner(&t.name)).collect();
    let log = cli::run_program(
        &[
            "journalctl",
            "--since",
            "-3min",
            "--no-pager",
            "-o",
            "cat",
            "-t",
            "NetworkManager",
            "-t",
            "pppd",
            "-t",
            "nm-l2tp-service",
            "-t",
            "charon",
            "-t",
            "ipsec",
        ],
        5,
    )
    .await
    .map(|(_, out, _)| out)
    .unwrap_or_default();
    model::classify_vpn_failure(&log, &others)
}

/// A wrapped note with a button at the end.
fn note_line(text: &str, button: &gtk::Button) -> gtk::Box {
    let note = gtk::Label::builder().label(text).xalign(0.0).hexpand(true).wrap(true).build();
    button.set_valign(gtk::Align::Center);
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&note);
    line.append(button);
    line
}
