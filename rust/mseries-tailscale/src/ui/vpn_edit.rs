//! Edit sheet for one NetworkManager VPN: server, username, password, IPsec pre-shared key, "Send all traffic
//! over VPN connection", and whether to remember the passwords. Secret fields start empty; empty keeps what is
//! stored. Saving goes through `nm::save_vpn` (D-Bus), so secrets are never on a command line or in a log.

use crate::cli;
use crate::model;
use crate::nm::{self, VpnEdit};
use crate::strings::tr;
use gtk::{glib, prelude::*};
use mseries_pantheon_kit::{chrome, set_accessible_label};
use std::rc::Rc;

/// Opens the sheet over `parent`. `on_saved(active)` runs after a successful save.
pub fn open(
    parent: &gtk::Window,
    toast: &granite::Toast,
    vpn: &model::Vpn,
    on_saved: impl Fn(bool) + 'static,
) -> gtk::Window {
    let sheet = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .title(format!("{} {}", vpn.name, tr("Settings")))
        .default_width(460)
        .resizable(false)
        .build();
    sheet.add_css_class("mseries-tailscale");

    let header = gtk::HeaderBar::builder().show_title_buttons(false).build();
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class(granite::CssClassSUGGESTED);
    save.set_sensitive(false);
    header.pack_start(&cancel);
    header.pack_end(&save);
    let title = gtk::Label::new(Some(&vpn.name));
    title.add_css_class("title");
    header.set_title_widget(Some(&title));
    sheet.set_titlebar(Some(&header));

    let server = gtk::Entry::builder().hexpand(true).build();
    let user = gtk::Entry::builder().hexpand(true).build();
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).build();
    let psk = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).build();
    set_accessible_label(&server, &tr("Server"));
    set_accessible_label(&user, &tr("Username"));
    set_accessible_label(&password, &tr("Password"));
    set_accessible_label(&psk, &tr("Pre-shared key"));
    let all = gtk::Switch::builder().valign(gtk::Align::Center).active(vpn.all_traffic).build();
    let remember = gtk::Switch::builder().valign(gtk::Align::Center).active(true).build();
    set_accessible_label(&all, &tr("Send all traffic over VPN connection"));
    set_accessible_label(&remember, &tr("Remember passwords"));

    let grid = gtk::Grid::builder().row_spacing(9).column_spacing(12).build();
    let mut row = 0;
    let mut add = |label: &str, widget: &gtk::Widget| {
        let l = gtk::Label::builder().label(label).xalign(1.0).build();
        l.add_css_class(granite::CssClassDIM);
        grid.attach(&l, 0, row, 1, 1);
        grid.attach(widget, 1, row, 1, 1);
        row += 1;
    };
    add(&tr("Server"), server.upcast_ref());
    add(&tr("Username"), user.upcast_ref());
    add(&tr("Password"), password.upcast_ref());
    add(&tr("Pre-shared key"), psk.upcast_ref());
    let psk_row = row - 1;

    let note = gtk::Label::builder().xalign(0.0).wrap(true).build();
    note.add_css_class(granite::CssClassDIM);
    note.add_css_class(granite::CssClassSMALL);

    let switches = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(9).build();
    switches.add_css_class(granite::CssClassCARD);
    switches.add_css_class("mseries-tailscale-card");
    switches.append(&row_with(
        &tr("Send all traffic over VPN connection"),
        &tr("Off sends only the VPN's own networks through it"),
        &all,
    ));
    switches.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    switches.append(&row_with(
        &tr("Remember passwords"),
        &tr("Saved on this computer, readable only by the system. Off asks every time."),
        &remember,
    ));

    let error = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).build();
    error.add_css_class(granite::CssClassERROR);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(15)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    body.append(&grid);
    body.append(&note);
    body.append(&switches);
    body.append(&error);
    sheet.set_child(Some(&body));

    // Fill in the non-secret fields; secrets stay empty (empty keeps the stored value).
    let keys = Rc::new(std::cell::RefCell::new(("gateway".to_string(), "user".to_string(), false)));
    {
        let (server, user, password, psk, note, save, keys, grid) = (
            server.clone(),
            user.clone(),
            password.clone(),
            psk.clone(),
            note.clone(),
            save.clone(),
            keys.clone(),
            grid.clone(),
        );
        let uuid = vpn.uuid.clone();
        glib::spawn_future_local(async move {
            let data = match cli::run_program(&["nmcli", "-g", "vpn.data", "connection", "show", &uuid], 8).await {
                Ok((true, out, _)) => model::parse_vpn_data(&out),
                _ => Default::default(),
            };
            let server_key = if data.contains_key("remote") { "remote" } else { "gateway" };
            let user_key = if data.contains_key("username") { "username" } else { "user" };
            let has_psk = data.contains_key("ipsec-psk-flags") || data.get("ipsec-enabled").is_some_and(|v| v == "yes");
            server.set_text(data.get(server_key).map(String::as_str).unwrap_or_default());
            user.set_text(data.get(user_key).map(String::as_str).unwrap_or_default());
            let saved =
                model::secrets_saved(&data.iter().map(|(k, v)| format!("{k} = {v}")).collect::<Vec<_>>().join(", "));
            let hint = if saved { tr("Saved. Leave empty to keep.") } else { tr("Not saved yet") };
            password.set_property("placeholder-text", &hint);
            psk.set_property("placeholder-text", &hint);
            for col in 0..2 {
                if let Some(w) = grid.child_at(col, psk_row) {
                    w.set_visible(has_psk);
                }
            }
            note.set_label(&if saved {
                tr("Type a new password or key only to change it.")
            } else {
                tr("Type the password and pre-shared key once. They will be remembered.")
            });
            *keys.borrow_mut() = (server_key.into(), user_key.into(), has_psk);
            save.set_sensitive(true);
        });
    }

    let s = sheet.clone();
    cancel.connect_clicked(move |_| s.close());
    let keyc = gtk::EventControllerKey::new();
    let s = sheet.clone();
    keyc.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            s.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    sheet.add_controller(keyc);

    let on_saved = Rc::new(on_saved);
    let (uuid, active, toast) = (vpn.uuid.clone(), vpn.active, toast.clone());
    let s = sheet.clone();
    save.connect_clicked(move |button| {
        let (server_key, user_key, has_psk) = keys.borrow().clone();
        let secret = |e: &gtk::PasswordEntry| Some(e.text().to_string()).filter(|t| !t.is_empty());
        let edit = VpnEdit {
            server_key,
            server: server.text().to_string(),
            user_key,
            user: user.text().to_string(),
            password: secret(&password),
            psk: if has_psk { secret(&psk) } else { None },
            all_traffic: all.is_active(),
            remember: remember.is_active(),
        };
        button.set_sensitive(false);
        error.set_visible(false);
        let (uuid, s, button, error, toast, on_saved) =
            (uuid.clone(), s.clone(), button.clone(), error.clone(), toast.clone(), on_saved.clone());
        glib::spawn_future_local(async move {
            match nm::save_vpn(&uuid, edit).await {
                Ok(()) => {
                    chrome::show_toast(&toast, &tr("VPN settings saved"));
                    on_saved(active);
                    s.close();
                }
                Err(message) => {
                    error.set_label(&message);
                    error.set_visible(true);
                    button.set_sensitive(true);
                }
            }
        });
    });

    sheet.present();
    sheet
}

fn row_with(title: &str, description: &str, control: &gtk::Switch) -> gtk::Box {
    let label = gtk::Label::builder().label(title).xalign(0.0).wrap(true).build();
    let sub = gtk::Label::builder().label(description).xalign(0.0).wrap(true).build();
    sub.add_css_class(granite::CssClassDIM);
    sub.add_css_class(granite::CssClassSMALL);
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    text.append(&label);
    text.append(&sub);
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&text);
    row.append(control);
    row
}
