use granite::prelude::PlaceholderExt;
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{
    BlueFerryMessagesClient as Backend, BlueFerryStatus, ContactMatch, Message, Thread,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

#[derive(Default)]
struct State {
    threads: Vec<Thread>,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    pending: Vec<String>,
    map_ready: bool,
}

pub struct App {
    backend: Backend,
    history_subscription: RefCell<Option<gio::SignalSubscription>>,
    window: gtk::ApplicationWindow,
    status: gtk::Label,
    feedback: gtk::Label,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    favorites_section: gtk::Box,
    favorites_strip: gtk::Box,
    favorite_buttons: RefCell<HashMap<String, gtk::ToggleButton>>,
    contact_photos: RefCell<HashMap<String, Option<mseries_pantheon_kit::gtk::gdk_pixbuf::Pixbuf>>>,
    requested_contact_photos: RefCell<HashSet<String>>,
    contact_photo_revision: RefCell<Option<(bool, Option<u64>)>>,
    contact_photo_generation: Cell<u64>,
    sidebar: gtk::Box,
    detail: gtk::Box,
    pane: gtk::Paned,
    back_button: gtk::Button,
    order: RefCell<Vec<String>>,
    conversation_title: gtk::Label,
    messages: gtk::Box,
    editor: gtk::TextView,
    send_button: gtk::Button,
    star_button: gtk::ToggleButton,
    syncing_star: Cell<bool>,
    state: RefCell<State>,
}

impl App {
    pub fn build(application: &gtk::Application) -> Option<Rc<Self>> {
        let backend = Backend::connect();
        let backend = match backend {
            Ok(backend) => backend,
            Err(error) => {
                let window = gtk::ApplicationWindow::builder()
                    .application(application)
                    .title("Messages")
                    .build();
                window.set_default_size(620, 440);
                let placeholder = granite::Placeholder::new("Messages could not start");
                placeholder.set_description(&error);
                placeholder.set_icon(&gio::ThemedIcon::new("mail-message-new"));
                window.set_child(Some(&placeholder));
                window.present();
                return None;
            }
        };

        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("Messages"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        let new_message = gtk::Button::from_icon_name("mail-message-new-symbolic");
        new_message.set_tooltip_text(Some("New message"));
        mseries_pantheon_kit::set_accessible_label(&new_message, "New message");
        header.pack_start(&new_message);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Refresh conversations"));
        mseries_pantheon_kit::set_accessible_label(&refresh, "Refresh conversations");
        header.pack_start(&refresh);

        let status = gtk::Label::builder()
            .label("Connecting to BlueFerry…")
            .xalign(0.0)
            .wrap(true)
            .build();
        status.add_css_class("dim-label");
        let feedback = gtk::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(true)
            .margin_start(12)
            .margin_end(12)
            .build();
        feedback.add_css_class("dim-label");

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search conversations")
            .margin_start(8)
            .margin_end(8)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        let favorites_title = gtk::Label::builder().label("Favorites").xalign(0.0).build();
        favorites_title.add_css_class("heading");
        let favorites_strip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        mseries_pantheon_kit::set_box_spacing_text_units(&favorites_strip, 0.5);
        let favorites_scroll = gtk::ScrolledWindow::builder()
            .child(&favorites_strip)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(mseries_pantheon_kit::text_unit(&search, 5.6))
            .build();
        let favorites_section = gtk::Box::new(gtk::Orientation::Vertical, 4);
        mseries_pantheon_kit::set_box_spacing_text_units(&favorites_section, 0.25);
        mseries_pantheon_kit::set_margins_text_units(&favorites_section, 0.45, 0.65, 0.45, 0.65);
        favorites_section.append(&favorites_title);
        favorites_section.append(&favorites_scroll);
        favorites_section.set_visible(false);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.append(&search);
        sidebar.append(&favorites_section);
        sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        sidebar.append(&list_scroll);

        let conversation_title = gtk::Label::builder()
            .label("Select a conversation")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        conversation_title.add_css_class("title-2");
        let star_button = gtk::ToggleButton::builder()
            .icon_name("non-starred-symbolic")
            .tooltip_text("Favorite conversation")
            .sensitive(false)
            .build();
        let conversation_header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        mseries_pantheon_kit::set_box_spacing_text_units(&conversation_header, 0.6);
        mseries_pantheon_kit::set_margins_text_units(&conversation_header, 0.65, 0.75, 0.65, 1.0);
        let back_button = gtk::Button::with_label("Back");
        back_button.set_visible(false);
        mseries_pantheon_kit::set_accessible_label(&back_button, "Back to conversations");
        conversation_header.append(&back_button);
        conversation_header.append(&conversation_title);
        conversation_header.append(&star_button);

        let messages = gtk::Box::new(gtk::Orientation::Vertical, 10);
        mseries_pantheon_kit::set_box_spacing_text_units(&messages, 0.65);
        mseries_pantheon_kit::set_margins_text_units(&messages, 1.0, 1.0, 1.0, 1.0);
        let message_scroll = gtk::ScrolledWindow::builder()
            .child(&messages)
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let editor = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .left_margin(10)
            .right_margin(10)
            .top_margin(8)
            .bottom_margin(8)
            .height_request(72)
            .build();
        let editor_scroll = gtk::ScrolledWindow::builder()
            .child(&editor)
            .hexpand(true)
            .min_content_height(64)
            .max_content_height(150)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let send_button = gtk::Button::with_label("Send");
        send_button.add_css_class("suggested-action");
        send_button.set_sensitive(false);
        let compose = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        mseries_pantheon_kit::set_box_spacing_text_units(&compose, 0.5);
        mseries_pantheon_kit::set_margins_text_units(&compose, 0.65, 0.75, 0.65, 0.75);
        compose.append(&editor_scroll);
        compose.append(&send_button);

        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.append(&status);
        detail.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        detail.append(&conversation_header);
        detail.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        detail.append(&message_scroll);
        detail.append(&feedback);
        detail.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        detail.append(&compose);

        let pane = gtk::Paned::new(gtk::Orientation::Horizontal);
        pane.set_start_child(Some(&sidebar));
        pane.set_end_child(Some(&detail));
        pane.set_position(mseries_pantheon_kit::text_unit(&pane, 19.0));
        pane.set_shrink_start_child(true);
        pane.set_shrink_end_child(true);
        pane.set_vexpand(true);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&pane);

        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Messages")
            .build();
        window.set_default_size(1040, 730);
        window.set_titlebar(Some(&header));
        window.set_child(Some(&root));

        let app = Rc::new(Self {
            backend,
            history_subscription: RefCell::new(None),
            window,
            status,
            feedback,
            search,
            list,
            favorites_section,
            favorites_strip,
            favorite_buttons: RefCell::new(HashMap::new()),
            contact_photos: RefCell::new(HashMap::new()),
            requested_contact_photos: RefCell::new(HashSet::new()),
            contact_photo_revision: RefCell::new(None),
            contact_photo_generation: Cell::new(0),
            sidebar,
            detail,
            pane,
            back_button,
            order: RefCell::new(Vec::new()),
            conversation_title,
            messages,
            editor,
            send_button,
            star_button,
            syncing_star: Cell::new(false),
            state: RefCell::new(State::default()),
        });
        app.connect_signals(&refresh, &new_message);
        app.connect_responsive_layout();
        app.watch_history();
        app.window.present();
        app.refresh();
        Some(app)
    }

    fn connect_signals(self: &Rc<Self>, refresh: &gtk::Button, new_message: &gtk::Button) {
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.refresh();
            }
        });

        let weak = Rc::downgrade(self);
        new_message.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.show_new_message_dialog();
            }
        });

        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.rebuild_sidebar();
            }
        });

        let weak = Rc::downgrade(self);
        self.list.connect_row_selected(move |_, row| {
            let Some(app) = weak.upgrade() else { return };
            let Some(row) = row else { return };
            let index = row.index();
            let key = app.order.borrow().get(index as usize).cloned();
            if let Some(key) = key {
                app.select_thread(&key);
            }
        });

        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.show_selected_conversation();
            }
        });

        let weak = Rc::downgrade(self);
        self.back_button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.show_conversation_list();
            }
        });

        let weak = Rc::downgrade(self);
        self.editor.buffer().connect_changed(move |buffer| {
            if let Some(app) = weak.upgrade() {
                let Some(key) = app.state.borrow().selected.clone() else {
                    return;
                };
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string();
                app.state.borrow_mut().drafts.insert(key, text);
            }
        });

        let weak = Rc::downgrade(self);
        self.send_button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.send_current();
            }
        });

        let weak = Rc::downgrade(self);
        self.star_button.connect_toggled(move |button| {
            if let Some(app) = weak.upgrade()
                && !app.syncing_star.get()
            {
                app.toggle_star(button.is_active());
            }
        });
    }

    fn connect_responsive_layout(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        mseries_pantheon_kit::watch_responsive_width(&self.window, 52.0, move |class| {
            if let Some(app) = weak.upgrade() {
                app.apply_layout_class(class);
            }
        });
    }

    fn apply_layout_class(&self, class: mseries_pantheon_kit::LayoutClass) {
        let compact = class == mseries_pantheon_kit::LayoutClass::Compact;
        let selected = self.state.borrow().selected.is_some();
        self.sidebar.set_visible(!compact || !selected);
        self.detail.set_visible(!compact || selected);
        self.back_button.set_visible(compact && selected);
        if !compact && self.pane.width() > 0 {
            self.pane
                .set_position(mseries_pantheon_kit::text_unit(&self.pane, 19.0));
        }
    }

    fn show_selected_conversation(&self) {
        if self.window.has_css_class("pantheon-compact") {
            self.sidebar.set_visible(false);
            self.detail.set_visible(true);
            self.back_button.set_visible(true);
        }
    }

    fn show_conversation_list(&self) {
        if self.window.has_css_class("pantheon-compact") {
            self.sidebar.set_visible(true);
            self.detail.set_visible(false);
            self.back_button.set_visible(false);
        }
    }

    fn watch_history(self: &Rc<Self>) {
        let weak: Weak<Self> = Rc::downgrade(self);
        let id = self.backend.watch_history(move || {
            if let Some(app) = weak.upgrade() {
                app.refresh();
            }
        });
        *self.history_subscription.borrow_mut() = Some(id);
    }

    fn refresh(self: &Rc<Self>) {
        self.status.set_text("Refreshing conversations…");
        let weak = Rc::downgrade(self);
        let backend = self.backend.clone();
        glib::MainContext::default().spawn_local(async move {
            let outcome = async {
                let status = backend.status().await?;
                let threads = backend.threads().await?;
                Ok::<_, String>((status, threads))
            }
            .await;
            if let Some(app) = weak.upgrade() {
                match outcome {
                    Ok((status, threads)) => {
                        app.update_contact_photo_revision(&status);
                        let ready = status.map;
                        let detail = status.connectivity_detail.as_deref().unwrap_or("");
                        app.status.set_text(if ready {
                            "Connected to BlueFerry · iPhone Messages available"
                        } else if detail.is_empty() {
                            "BlueFerry is connected · iPhone Messages are unavailable"
                        } else {
                            detail
                        });
                        let mut state = app.state.borrow_mut();
                        state.map_ready = ready;
                        let mut server_threads: Vec<Thread> = threads
                            .into_iter()
                            .filter(|thread| !thread.key.is_empty())
                            .collect();
                        let local_threads = state
                            .threads
                            .iter()
                            .filter(|thread| thread.local_only)
                            .cloned()
                            .collect::<Vec<_>>();
                        let mut keep_local = Vec::new();
                        for local in local_threads {
                            if state.pending.contains(&local.key) {
                                keep_local.push(local);
                                continue;
                            }
                            let address =
                                local.recipients.first().map(String::as_str).unwrap_or("");
                            if let Some(server) = server_threads.iter().find(|server| {
                                !server.is_group
                                    && server.recipients.iter().any(|recipient| {
                                        normalize_recipient(recipient).as_deref() == Some(address)
                                    })
                            }) {
                                let server_key = server.key.clone();
                                if state.selected.as_deref() == Some(local.key.as_str()) {
                                    state.selected = Some(server_key.clone());
                                }
                                if let Some(draft) = state.drafts.remove(&local.key) {
                                    state.drafts.entry(server_key).or_insert(draft);
                                }
                            } else {
                                keep_local.push(local);
                            }
                        }
                        server_threads.extend(keep_local);
                        state.threads = server_threads;
                        drop(state);
                        app.rebuild_sidebar();
                        app.load_contact_photos();
                    }
                    Err(error) => {
                        app.state.borrow_mut().map_ready = false;
                        app.send_button.set_sensitive(false);
                        app.status.set_text(&error);
                    }
                }
            }
        });
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        let filter = self.search.text().to_string().to_lowercase();
        let (threads, selected) = {
            let state = self.state.borrow();
            let threads = state
                .threads
                .iter()
                .filter(|thread| {
                    filter.is_empty()
                        || thread.name.to_lowercase().contains(&filter)
                        || thread.preview().to_lowercase().contains(&filter)
                        || thread
                            .recipients
                            .iter()
                            .any(|recipient| recipient.to_lowercase().contains(&filter))
                })
                .cloned()
                .collect::<Vec<_>>();
            (threads, state.selected.clone())
        };
        self.list.remove_all();
        let mut order = Vec::with_capacity(threads.len());
        let mut selected_index = None;
        for thread in &threads {
            let row = gtk::ListBoxRow::new();
            let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
            copy.add_css_class("pantheon-row");
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let name = gtk::Label::builder()
                .label(&thread.name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            name.add_css_class("heading");
            if thread.unread() > 0 {
                name.add_css_class("accent");
            }
            heading.append(&name);
            if thread.starred {
                heading.append(&gtk::Image::from_icon_name("starred-symbolic"));
            }
            copy.append(&heading);
            let preview = gtk::Label::builder()
                .label(thread.preview().replace('\n', " "))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(34)
                .build();
            preview.add_css_class("dim-label");
            copy.append(&preview);
            let meta = gtk::Label::builder()
                .label(if thread.is_group {
                    "Group conversation"
                } else {
                    &thread.last_ts
                })
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            meta.add_css_class("caption");
            meta.add_css_class("dim-label");
            copy.append(&meta);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            content.set_valign(gtk::Align::Center);
            content.append(
                &self.avatar_for_thread(thread, mseries_pantheon_kit::text_unit(&self.list, 2.8)),
            );
            content.append(&copy);
            row.set_child(Some(&content));
            if selected.as_deref() == Some(thread.key.as_str()) {
                selected_index = Some(order.len() as i32);
            }
            order.push(thread.key.clone());
            self.list.append(&row);
        }
        *self.order.borrow_mut() = order;
        if let Some(index) = selected_index.or_else(|| (!threads.is_empty()).then_some(0)) {
            if let Some(row) = self.list.row_at_index(index) {
                self.list.select_row(Some(&row));
            }
        } else {
            self.state.borrow_mut().selected = None;
            self.render_conversation(None);
            self.show_conversation_list();
        }
        let selected = self.state.borrow().selected.clone();
        self.rebuild_favorites(&threads, selected.as_deref());
    }

    fn rebuild_favorites(self: &Rc<Self>, threads: &[Thread], selected: Option<&str>) {
        while let Some(child) = self.favorites_strip.first_child() {
            self.favorites_strip.remove(&child);
        }
        self.favorite_buttons.borrow_mut().clear();

        let mut favorites = threads
            .iter()
            .filter(|thread| thread.starred)
            .cloned()
            .collect::<Vec<_>>();
        favorites.sort_by(|left, right| right.last_ts.cmp(&left.last_ts));
        for thread in &favorites {
            let button = gtk::ToggleButton::new();
            button.add_css_class("flat");
            button.set_tooltip_text(Some(&thread.name));
            mseries_pantheon_kit::set_accessible_label(
                &button,
                &format!("Favorite conversation: {}", thread.name),
            );
            let tile = gtk::Box::new(gtk::Orientation::Vertical, 3);
            mseries_pantheon_kit::set_box_spacing_text_units(&tile, 0.2);
            mseries_pantheon_kit::set_margins_text_units(&tile, 0.25, 0.35, 0.25, 0.35);
            let avatar =
                self.avatar_for_thread(thread, mseries_pantheon_kit::text_unit(&tile, 2.8));
            tile.append(&avatar);
            let name = gtk::Label::builder()
                .label(&thread.name)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(10)
                .build();
            name.add_css_class("caption");
            tile.append(&name);
            button.set_child(Some(&tile));
            button.set_active(selected == Some(thread.key.as_str()));

            let weak = Rc::downgrade(self);
            let key = thread.key.clone();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    if let Some(index) = app.order.borrow().iter().position(|entry| entry == &key)
                        && let Some(row) = app.list.row_at_index(index as i32)
                    {
                        app.list.select_row(Some(&row));
                    }
                    app.sync_favorite_selection();
                }
            });
            self.favorites_strip.append(&button);
            self.favorite_buttons
                .borrow_mut()
                .insert(thread.key.clone(), button);
        }
        self.favorites_section.set_visible(!favorites.is_empty());
    }

    fn avatar_for_thread(&self, thread: &Thread, size: i32) -> gtk::Widget {
        let pixbuf = if thread.is_group {
            None
        } else {
            thread
                .recipients
                .first()
                .and_then(|address| self.contact_photos.borrow().get(address).cloned().flatten())
        };
        let picture = if let Some(pixbuf) = pixbuf {
            let scaled = pixbuf
                .scale_simple(size, size, gtk::gdk_pixbuf::InterpType::Bilinear)
                .unwrap_or(pixbuf);
            let pixels = scaled.read_pixel_bytes();
            let format = if scaled.has_alpha() {
                gtk::gdk::MemoryFormat::R8g8b8a8
            } else {
                gtk::gdk::MemoryFormat::R8g8b8
            };
            let texture = gtk::gdk::MemoryTexture::new(
                scaled.width(),
                scaled.height(),
                format,
                &pixels,
                scaled.rowstride() as usize,
            );
            gtk::Image::from_paintable(Some(&texture))
        } else {
            let icon = gtk::Image::from_icon_name(if thread.is_group {
                "system-users-symbolic"
            } else {
                "avatar-default-symbolic"
            });
            icon.set_pixel_size(size);
            icon.set_size_request(size, size);
            icon.add_css_class("pantheon-avatar");
            return icon.upcast();
        };
        picture.set_size_request(size, size);
        picture.add_css_class("pantheon-avatar");
        picture.upcast()
    }

    fn update_contact_photo_revision(&self, status: &BlueFerryStatus) {
        let enabled = status.contact_photos;
        let revision = status.contact_photo_revision;
        let next = (enabled, revision);
        if self.contact_photo_revision.borrow().as_ref() == Some(&next) {
            return;
        }
        *self.contact_photo_revision.borrow_mut() = Some(next);
        self.contact_photo_generation
            .set(self.contact_photo_generation.get().wrapping_add(1));
        self.contact_photos.borrow_mut().clear();
        self.requested_contact_photos.borrow_mut().clear();
    }

    fn load_contact_photos(self: &Rc<Self>) {
        let enabled = self
            .contact_photo_revision
            .borrow()
            .as_ref()
            .is_some_and(|(enabled, _)| *enabled);
        if !enabled {
            return;
        }

        let addresses = {
            let state = self.state.borrow();
            let photos = self.contact_photos.borrow();
            let mut requested = self.requested_contact_photos.borrow_mut();
            state
                .threads
                .iter()
                .filter(|thread| !thread.is_group)
                .take(40)
                .filter_map(|thread| thread.recipients.first())
                .filter(|address| !address.is_empty() && !photos.contains_key(*address))
                .filter(|address| requested.insert((*address).clone()))
                .cloned()
                .collect::<Vec<_>>()
        };
        if addresses.is_empty() {
            return;
        }

        let generation = self.contact_photo_generation.get();
        let backend = self.backend.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let mut loaded = HashMap::new();
            let mut failed = Vec::new();
            for address in &addresses {
                match backend.contact_photo(address).await {
                    Ok(bytes) => {
                        let pixbuf = mseries_pantheon_kit::decode_contact_avatar(&bytes, 96);
                        loaded.insert(address.clone(), pixbuf);
                    }
                    Err(_) => failed.push(address.clone()),
                }
            }
            let Some(app) = weak.upgrade() else { return };
            if app.contact_photo_generation.get() != generation {
                return;
            }
            for address in failed {
                app.requested_contact_photos.borrow_mut().remove(&address);
            }
            let changed = loaded.values().any(Option::is_some);
            app.contact_photos.borrow_mut().extend(loaded);
            if changed {
                app.rebuild_sidebar();
            }
        });
    }

    fn sync_favorite_selection(&self) {
        let selected = self.state.borrow().selected.clone();
        for (key, button) in self.favorite_buttons.borrow().iter() {
            button.set_active(selected.as_deref() == Some(key.as_str()));
        }
    }

    fn select_thread(self: &Rc<Self>, key: &str) {
        let thread = {
            let mut state = self.state.borrow_mut();
            state.selected = Some(key.to_string());
            state
                .threads
                .iter()
                .find(|thread| thread.key == key)
                .cloned()
        };
        self.render_conversation(thread.clone());
        self.show_selected_conversation();
        self.sync_favorite_selection();
        if let Some(thread) = thread.filter(|thread| !thread.local_only && thread.unread() > 0) {
            let backend = self.backend.clone();
            let weak = Rc::downgrade(self);
            let key = thread.key.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(error) = backend.mark_read(&key).await
                    && let Some(app) = weak.upgrade()
                {
                    app.status
                        .set_text(&format!("Could not mark conversation read: {error}"));
                }
            });
        }
    }

    fn render_conversation(self: &Rc<Self>, thread: Option<Thread>) {
        while let Some(child) = self.messages.first_child() {
            self.messages.remove(&child);
        }
        if let Some(thread) = thread {
            self.conversation_title.set_text(&thread.name);
            self.star_button.set_sensitive(!thread.local_only);
            self.syncing_star.set(true);
            self.star_button.set_active(thread.starred);
            self.syncing_star.set(false);
            mseries_pantheon_kit::set_accessible_label(
                &self.star_button,
                if thread.starred {
                    "Remove favorite"
                } else {
                    "Add favorite"
                },
            );
            self.star_button.set_icon_name(if thread.starred {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            });
            self.star_button.set_tooltip_text(Some(if thread.starred {
                "Remove favorite"
            } else {
                "Favorite conversation"
            }));
            if thread.messages.is_empty() {
                let empty = gtk::Label::new(Some("No messages in this conversation yet."));
                empty.add_css_class("dim-label");
                self.messages.append(&empty);
            }
            for message in &thread.messages {
                let bubble = gtk::Box::new(gtk::Orientation::Vertical, 4);
                bubble.add_css_class(if message.outgoing {
                    "pantheon-message-outgoing"
                } else {
                    "pantheon-message-incoming"
                });
                bubble.set_halign(if message.outgoing {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                });
                let visible_body = message.visible_body();
                if !visible_body.is_empty() {
                    let text = gtk::Label::builder()
                        .label(&visible_body)
                        .xalign(0.0)
                        .wrap(true)
                        .selectable(true)
                        .max_width_chars(72)
                        .build();
                    text.add_css_class("card");
                    text.add_css_class("pantheon-surface");
                    let markup = crate::links::linkify_message(&visible_body);
                    text.set_markup(&markup);
                    let expected = visible_body.replace("\r\n", "\n").replace('\r', "\n");
                    if text.text() != expected {
                        text.set_text(&visible_body);
                    }
                    bubble.append(&text);
                }
                if message.has_attachment() {
                    let attachment = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    attachment.add_css_class("card");
                    attachment.add_css_class("pantheon-surface");
                    attachment.append(&gtk::Image::from_icon_name("mail-attachment-symbolic"));
                    attachment.append(&gtk::Label::new(Some("Attachment")));
                    attachment.set_tooltip_text(Some(
                        "Your iPhone shares only message text over Bluetooth. Open Messages on the iPhone to view this attachment.",
                    ));
                    mseries_pantheon_kit::set_accessible_label(
                        &attachment,
                        "Attachment. Open Messages on your iPhone to view it.",
                    );
                    bubble.append(&attachment);
                }
                let (reaction_symbols, reaction_description) = message.reaction_summary();
                if !reaction_symbols.is_empty() {
                    let reactions = gtk::Label::builder()
                        .label(&reaction_symbols)
                        .xalign(if message.outgoing { 1.0 } else { 0.0 })
                        .build();
                    reactions.add_css_class("caption");
                    reactions.add_css_class("card");
                    reactions.add_css_class("pantheon-surface");
                    reactions.set_tooltip_text(Some(&reaction_description));
                    mseries_pantheon_kit::set_accessible_label(
                        &reactions,
                        &format!("Reactions: {reaction_description}"),
                    );
                    bubble.append(&reactions);
                }
                if message.is_shortened() {
                    let shortened = gtk::Label::new(Some(
                        "Shortened by your iPhone · open it there for the rest",
                    ));
                    shortened.add_css_class("caption");
                    shortened.add_css_class("dim-label");
                    shortened.set_xalign(if message.outgoing { 1.0 } else { 0.0 });
                    bubble.append(&shortened);
                }
                if !message.timestamp.is_empty() {
                    let timestamp = gtk::Label::builder()
                        .label(&message.timestamp)
                        .xalign(if message.outgoing { 1.0 } else { 0.0 })
                        .build();
                    timestamp.add_css_class("caption");
                    timestamp.add_css_class("dim-label");
                    bubble.append(&timestamp);
                }
                self.attach_message_actions(&bubble, message, thread.is_group);
                self.messages.append(&bubble);
            }
            let permitted = thread.local_only
                || (thread.reply_ready && !thread.participants_required && !thread.roster_changed);
            self.editor.set_sensitive(true);
            let state = self.state.borrow();
            self.send_button.set_sensitive(
                permitted && state.map_ready && !state.pending.contains(&thread.key),
            );
            drop(state);
            let draft = self
                .state
                .borrow()
                .drafts
                .get(&thread.key)
                .cloned()
                .unwrap_or_default();
            let buffer = self.editor.buffer();
            let current = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            if current != draft {
                buffer.set_text(&draft);
            }
        } else {
            self.conversation_title.set_text("Select a conversation");
            self.star_button.set_sensitive(false);
            self.editor.set_sensitive(false);
            self.send_button.set_sensitive(false);
            let empty = granite::Placeholder::new("Your conversations appear here");
            empty.set_description("Choose a conversation from the list to read and reply.");
            empty.set_icon(&gio::ThemedIcon::new("mail-message-new"));
            self.messages.append(&empty);
        }
    }

    fn attach_message_actions(
        self: &Rc<Self>,
        bubble: &gtk::Box,
        message: &Message,
        is_group: bool,
    ) {
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        mseries_pantheon_kit::set_margins_text_units(&actions, 0.45, 0.45, 0.45, 0.45);

        let reply = gtk::Button::with_label("Reply");
        reply.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&reply, "Reply to message");
        let weak_app = Rc::downgrade(self);
        let reply_message = message.clone();
        let reply_popover = popover.downgrade();
        reply.connect_clicked(move |_| {
            if let Some(popover) = reply_popover.upgrade() {
                popover.popdown();
            }
            if let Some(app) = weak_app.upgrade() {
                app.reply_to_message(&reply_message, is_group);
            }
        });
        actions.append(&reply);

        let copy = gtk::Button::with_label("Copy Text");
        copy.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&copy, "Copy message text");
        let weak_app = Rc::downgrade(self);
        let copy_body = message.visible_body();
        let copy_popover = popover.downgrade();
        copy.connect_clicked(move |_| {
            if let Some(popover) = copy_popover.upgrade() {
                popover.popdown();
            }
            if let Some(app) = weak_app.upgrade() {
                gtk::prelude::WidgetExt::display(&app.window)
                    .clipboard()
                    .set_text(&copy_body);
                app.feedback.set_text("Message text copied.");
            }
        });
        actions.append(&copy);

        popover.set_child(Some(&actions));
        popover.set_parent(bubble);
        let right_click = gtk::GestureClick::builder().button(3).build();
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let right_popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            right_popover
                .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            right_popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        bubble.add_controller(right_click);

        let long_press = gtk::GestureLongPress::new();
        long_press.set_propagation_phase(gtk::PropagationPhase::Capture);
        let long_popover = popover.clone();
        long_press.connect_pressed(move |gesture, x, y| {
            long_popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            long_popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        bubble.add_controller(long_press);
        bubble.connect_destroy(move |_| popover.unparent());
    }

    fn reply_to_message(&self, message: &Message, is_group: bool) {
        if self.state.borrow().selected.is_none() {
            return;
        }
        let quote = message_quote(message, is_group);
        let buffer = self.editor.buffer();
        let current = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let prefix = format!("↪ {quote}\n");
        let draft = if current.trim().is_empty() {
            prefix
        } else {
            format!("{prefix}{current}")
        };
        buffer.set_text(&draft);
        buffer.place_cursor(&buffer.end_iter());
        self.editor.grab_focus();
    }

    fn toggle_star(self: &Rc<Self>, starred: bool) {
        let Some(key) = self.state.borrow().selected.clone() else {
            return;
        };
        let backend = self.backend.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = backend.set_starred(&key, starred).await;
            if let Some(app) = weak.upgrade() {
                if let Err(error) = result {
                    app.status
                        .set_text(&format!("Could not update favorite: {error}"));
                }
                app.refresh();
            }
        });
    }

    fn send_current(self: &Rc<Self>) {
        let Some(key) = self.state.borrow().selected.clone() else {
            return;
        };
        let body = {
            let buffer = self.editor.buffer();
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string()
        };
        if body.trim().is_empty() {
            self.status.set_text("Write a message before sending.");
            return;
        }
        let thread = self
            .state
            .borrow()
            .threads
            .iter()
            .find(|thread| thread.key == key)
            .cloned();
        let Some(thread) = thread else { return };
        if !self.state.borrow().map_ready {
            self.status
                .set_text("Your iPhone is offline. Your draft is kept while this window is open.");
            return;
        }
        if thread.local_only {
            let Some(recipient) = thread.recipients.first() else {
                self.status
                    .set_text("Choose a valid phone number or email address first.");
                return;
            };
            self.dispatch_send(
                thread.key,
                body,
                false,
                String::new(),
                Some(recipient.clone()),
            );
            return;
        }
        if !thread.reply_ready || thread.participants_required || thread.roster_changed {
            self.status.set_text("BlueFerry has blocked replies to this conversation. Review its status before sending.");
            return;
        }
        if thread.is_group && !thread.group_confirmed {
            self.confirm_group(thread, body);
        } else {
            self.dispatch_send(thread.key, body, false, thread.confirmation_token, None);
        }
    }

    fn confirm_group(self: &Rc<Self>, thread: Thread, body: String) {
        if thread.recipients.len() < 2 || thread.confirmation_token.is_empty() {
            self.status.set_text(
                "BlueFerry did not provide a complete group roster. This message was not sent.",
            );
            return;
        }
        let dialog = gtk::AlertDialog::builder()
            .message("Review group participants")
            .detail(format!(
                "This message will be sent to every participant below. Review the list before confirming.\n\n{}",
                thread.recipients.join("\n")
            ))
            .buttons(["Cancel", "Review & Send"])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        let key = thread.key;
        let token = thread.confirmation_token;
        dialog.choose(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |response| {
                if response == Ok(1)
                    && let Some(app) = weak.upgrade()
                {
                    app.dispatch_send(key.clone(), body.clone(), true, token.clone(), None);
                }
            },
        );
    }

    fn dispatch_send(
        self: &Rc<Self>,
        key: String,
        body: String,
        confirm_group: bool,
        token: String,
        recipient: Option<String>,
    ) {
        if self.state.borrow().pending.contains(&key) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            state.pending.push(key.clone());
            state
                .drafts
                .entry(key.clone())
                .or_insert_with(|| body.clone());
        }
        self.send_button.set_sensitive(false);
        self.feedback.set_text("Sending to your iPhone…");
        let backend = self.backend.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = if let Some(recipient) = recipient {
                backend.send_to_recipient(&recipient, &body).await
            } else {
                backend
                    .send_to_thread(&key, &body, confirm_group, &token)
                    .await
            };
            if let Some(app) = weak.upgrade() {
                {
                    let mut state = app.state.borrow_mut();
                    state.pending.retain(|pending| pending != &key);
                    if result.is_ok() && state.drafts.get(&key).is_some_and(|draft| draft == &body)
                    {
                        state.drafts.remove(&key);
                    }
                }
                match result {
                    Ok(()) => {
                        app.feedback
                            .set_text("The message was transferred to your iPhone.");
                        if app.state.borrow().selected.as_deref() == Some(key.as_str()) {
                            let buffer = app.editor.buffer();
                            let current = buffer
                                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                                .to_string();
                            if current == body {
                                buffer.set_text("");
                                app.state.borrow_mut().drafts.remove(&key);
                            }
                        }
                        if let Some(thread) = app
                            .state
                            .borrow_mut()
                            .threads
                            .iter_mut()
                            .find(|thread| thread.key == key && thread.local_only)
                        {
                            thread.messages.push(Message {
                                body: body.clone(),
                                outgoing: true,
                                read: true,
                                ..Message::default()
                            });
                        }
                    }
                    Err(error) if error.outcome_unknown => app.feedback.set_text(
                        "Your iPhone's send status is unknown. Your draft is kept. Check Messages on your iPhone before trying again.",
                    ),
                    Err(error) => app.feedback.set_text(&format!(
                        "Could not send. Your draft is kept; choose Send to retry. {}",
                        error.message
                    )),
                }
                app.refresh();
            }
        });
    }

    fn show_new_message_dialog(self: &Rc<Self>) {
        let dialog = gtk::Window::builder()
            .title("New Message")
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_width(440)
            .default_height(500)
            .build();
        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("New Message"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        let cancel = gtk::Button::with_label("Cancel");
        cancel.set_action_name(Some("window.close"));
        header.pack_start(&cancel);
        dialog.set_titlebar(Some(&header));

        let query = gtk::SearchEntry::builder()
            .placeholder_text("Name, phone number, or email")
            .build();
        mseries_pantheon_kit::set_accessible_label(&query, "To");
        let hint = gtk::Label::builder()
            .label("Choose a contact or enter a complete address.")
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("dim-label");
        let results = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .css_classes(["navigation-sidebar"])
            .build();
        let results_scroll = gtk::ScrolledWindow::builder()
            .child(&results)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let start = gtk::Button::with_label("Start Conversation");
        start.add_css_class("suggested-action");
        start.set_sensitive(false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        mseries_pantheon_kit::set_box_spacing_text_units(&content, 0.65);
        mseries_pantheon_kit::set_margins_text_units(&content, 1.0, 1.0, 1.0, 1.0);
        content.append(&query);
        content.append(&hint);
        content.append(&results_scroll);
        content.append(&start);
        dialog.set_child(Some(&content));
        dialog.set_default_widget(Some(&start));

        let mut recents = self
            .state
            .borrow()
            .threads
            .iter()
            .filter(|thread| !thread.local_only && !thread.is_group && thread.recipients.len() == 1)
            .map(|thread| {
                (
                    thread.last_ts.clone(),
                    ContactMatch {
                        name: thread.name.clone(),
                        address: thread.recipients[0].clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        recents.sort_by(|left, right| right.0.cmp(&left.0));
        let recents = recents
            .into_iter()
            .take(8)
            .map(|(_, contact)| contact)
            .collect::<Vec<_>>();
        let matches = Rc::new(RefCell::new(recents.clone()));
        let current = Rc::new(RefCell::new(None::<ContactMatch>));
        let generation = Rc::new(Cell::new(0_u64));
        let alive = Rc::new(Cell::new(true));
        append_contact_rows(&results, &recents);
        hint.set_text(if recents.is_empty() {
            "Choose a contact or enter a complete address."
        } else {
            "Recent conversations"
        });

        {
            let current = current.clone();
            let matches = matches.clone();
            let query = query.clone();
            let start = start.clone();
            results.connect_row_selected(move |_, row| {
                *current.borrow_mut() =
                    row.and_then(|row| matches.borrow().get(row.index() as usize).cloned());
                start.set_sensitive(
                    normalize_recipient(&query.text()).is_some() || current.borrow().is_some(),
                );
            });
        }
        {
            let app = Rc::downgrade(self);
            let dialog = dialog.downgrade();
            let matches = matches.clone();
            results.connect_row_activated(move |_, row| {
                let Some(app) = app.upgrade() else { return };
                let Some(contact) = matches.borrow().get(row.index() as usize).cloned() else {
                    return;
                };
                app.start_direct_conversation(contact.name, contact.address);
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
                app.editor.grab_focus();
            });
        }
        {
            let app = Rc::downgrade(self);
            let dialog = dialog.downgrade();
            let query = query.clone();
            let current = current.clone();
            start.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                let contact = normalize_recipient(&query.text())
                    .map(|address| ContactMatch {
                        name: String::new(),
                        address,
                    })
                    .or_else(|| current.borrow().clone());
                if let Some(contact) = contact {
                    app.start_direct_conversation(contact.name, contact.address);
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                    app.editor.grab_focus();
                }
            });
        }
        {
            let app = Rc::downgrade(self);
            let dialog = dialog.downgrade();
            let query_for_activate = query.clone();
            let current = current.clone();
            query.connect_activate(move |_| {
                let Some(app) = app.upgrade() else { return };
                let contact = normalize_recipient(&query_for_activate.text())
                    .map(|address| ContactMatch {
                        name: String::new(),
                        address,
                    })
                    .or_else(|| current.borrow().clone());
                if let Some(contact) = contact {
                    app.start_direct_conversation(contact.name, contact.address);
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                    app.editor.grab_focus();
                }
            });
        }
        {
            let app = Rc::downgrade(self);
            let results = results.clone();
            let matches = matches.clone();
            let current = current.clone();
            let hint = hint.clone();
            let start = start.clone();
            let generation = generation.clone();
            let alive = alive.clone();
            query.connect_search_changed(move |entry| {
                let next = generation.get().wrapping_add(1);
                generation.set(next);
                *current.borrow_mut() = None;
                let text = entry.text().to_string();
                let valid = normalize_recipient(&text).is_some();
                if text.chars().count() < 2 {
                    while let Some(child) = results.first_child() {
                        results.remove(&child);
                    }
                    *matches.borrow_mut() = recents.clone();
                    append_contact_rows(&results, &recents);
                    hint.set_text(if recents.is_empty() {
                        "Choose a contact or enter a complete address."
                    } else {
                        "Recent conversations"
                    });
                    start.set_sensitive(valid);
                    return;
                }
                while let Some(child) = results.first_child() {
                    results.remove(&child);
                }
                *matches.borrow_mut() = Vec::new();
                hint.set_text("Searching contacts…");
                start.set_sensitive(valid);
                let Some(app) = app.upgrade() else { return };
                let backend = app.backend.clone();
                let results = results.clone();
                let matches = matches.clone();
                let hint = hint.clone();
                let start = start.clone();
                let generation = generation.clone();
                let alive = alive.clone();
                glib::MainContext::default().spawn_local(async move {
                    let result = backend.find_contacts(&text).await;
                    if !alive.get() || generation.get() != next {
                        return;
                    }
                    let (contacts, search_failed) = match result {
                        Ok(contacts) => (contacts.into_iter().take(30).collect::<Vec<_>>(), false),
                        Err(error) => {
                            hint.set_text(&format!("Could not search contacts: {error}"));
                            (Vec::new(), true)
                        }
                    };
                    while let Some(child) = results.first_child() {
                        results.remove(&child);
                    }
                    *matches.borrow_mut() = contacts.clone();
                    append_contact_rows(&results, &contacts);
                    if !contacts.is_empty() {
                        hint.set_text("Choose the address you want to message.");
                    } else if !search_failed {
                        hint.set_text(
                            "No matching contacts. A complete number or email works too.",
                        );
                    }
                    start.set_sensitive(valid);
                });
            });
        }
        {
            let generation = generation.clone();
            let alive = alive.clone();
            dialog.connect_close_request(move |_| {
                alive.set(false);
                generation.set(generation.get().wrapping_add(1));
                glib::Propagation::Proceed
            });
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let results = results.clone();
            let dialog = dialog.downgrade();
            let query = query.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Down && query.has_focus() {
                    if let Some(row) = results.row_at_index(0) {
                        results.select_row(Some(&row));
                        row.grab_focus();
                        return glib::Propagation::Stop;
                    }
                } else if key == gtk::gdk::Key::Escape {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        dialog.add_controller(keys);
        dialog.present();
        query.grab_focus();
    }

    fn start_direct_conversation(self: &Rc<Self>, name: String, address: String) {
        let Some(address) = normalize_recipient(&address) else {
            self.feedback
                .set_text("Enter a complete phone number or email address.");
            return;
        };
        let existing = self
            .state
            .borrow()
            .threads
            .iter()
            .find(|thread| {
                !thread.is_group
                    && thread.recipients.iter().any(|recipient| {
                        normalize_recipient(recipient).as_deref() == Some(address.as_str())
                    })
            })
            .map(|thread| thread.key.clone());
        if let Some(key) = existing {
            self.state.borrow_mut().selected = Some(key.clone());
            self.search.set_text("");
            self.rebuild_sidebar();
            self.select_thread(&key);
            self.editor.grab_focus();
            return;
        }
        let key = format!("local:{address}");
        {
            let mut state = self.state.borrow_mut();
            state.selected = Some(key.clone());
            if !state.threads.iter().any(|thread| thread.key == key) {
                state.threads.push(Thread {
                    key: key.clone(),
                    name: if name.trim().is_empty() {
                        address.clone()
                    } else {
                        name
                    },
                    recipients: vec![address],
                    reply_ready: true,
                    local_only: true,
                    ..Thread::default()
                });
            }
        }
        self.search.set_text("");
        self.rebuild_sidebar();
        self.select_thread(&key);
        self.editor.grab_focus();
    }
}

fn append_contact_rows(list: &gtk::ListBox, contacts: &[ContactMatch]) {
    for contact in contacts {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        mseries_pantheon_kit::set_margins_text_units(&content, 0.45, 0.65, 0.45, 0.65);
        let display_name = if contact.name.is_empty() {
            &contact.address
        } else {
            &contact.name
        };
        let name = gtk::Label::builder()
            .label(display_name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        name.add_css_class("heading");
        content.append(&name);
        if !contact.name.is_empty() {
            let address = gtk::Label::builder()
                .label(&contact.address)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            address.add_css_class("dim-label");
            content.append(&address);
        }
        row.set_child(Some(&content));
        let accessible = if contact.name.is_empty() {
            contact.address.clone()
        } else {
            format!("{}, {}", contact.name, contact.address)
        };
        mseries_pantheon_kit::set_accessible_label(&row, &accessible);
        list.append(&row);
    }
}

fn message_quote(message: &Message, is_group: bool) -> String {
    let mut text = message
        .visible_body()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        text = "Attachment".to_string();
    }
    if text.chars().count() > 80 {
        text = text
            .chars()
            .take(79)
            .collect::<String>()
            .trim_end()
            .to_string();
        text.push('…');
    }
    let quote = format!("“{text}”");
    if is_group && !message.outgoing && !message.sender.is_empty() {
        format!("{}: {quote}", message.sender)
    } else {
        quote
    }
}

fn normalize_recipient(input: &str) -> Option<String> {
    let candidate = input.trim();
    let bytes = candidate.as_bytes();
    let phone = !bytes.is_empty()
        && (bytes[0] != b'+' || bytes.len() > 1)
        && bytes.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_digit() || (index == 0 && *byte == b'+') || b" ()-.".contains(byte)
        });
    if phone {
        let digits = candidate.bytes().filter(u8::is_ascii_digit).count();
        let plus = candidate.starts_with('+');
        let remainder = if plus { &bytes[1..] } else { bytes };
        if (3..=20).contains(&digits)
            && remainder.len() >= 3
            && remainder.len() <= 30
            && remainder[0].is_ascii_digit()
        {
            let digits = candidate
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>();
            return Some(format!("{}{}", if plus { "+" } else { "" }, digits));
        }
    }

    if candidate.len() > 254 || candidate.matches('@').count() != 1 || !candidate.is_ascii() {
        return None;
    }
    let (local, domain) = candidate.split_once('@')?;
    if local.is_empty()
        || local.len() > 64
        || domain.is_empty()
        || domain.len() > 253
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~.-".contains(&byte))
    {
        return None;
    }
    for label in domain.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return None;
        }
    }
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}
