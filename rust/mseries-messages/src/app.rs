use crate::backend::Backend;
use crate::model::Thread;
use granite::prelude::PlaceholderExt;
use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

#[derive(Default)]
struct State {
    threads: Vec<Thread>,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    pending: Vec<String>,
}

pub struct App {
    backend: Backend,
    history_subscription: RefCell<Option<gio::SignalSubscription>>,
    window: gtk::ApplicationWindow,
    status: gtk::Label,
    feedback: gtk::Label,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
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
        app.connect_signals(&refresh);
        app.connect_responsive_layout();
        app.watch_history();
        app.window.present();
        app.refresh();
        Some(app)
    }

    fn connect_signals(self: &Rc<Self>, refresh: &gtk::Button) {
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.refresh();
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
                        let ready = status
                            .get("map")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let detail = status
                            .get("connectivity_detail")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        app.status.set_text(if ready {
                            "Connected to BlueFerry · iPhone Messages available"
                        } else if detail.is_empty() {
                            "BlueFerry is connected · iPhone Messages are unavailable"
                        } else {
                            detail
                        });
                        app.state.borrow_mut().threads = threads
                            .into_iter()
                            .filter(|thread| !thread.key.is_empty())
                            .collect();
                        app.rebuild_sidebar();
                    }
                    Err(error) => app.status.set_text(&error),
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
            row.set_child(Some(&copy));
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
        if let Some(thread) = thread.filter(|thread| thread.unread() > 0) {
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

    fn render_conversation(&self, thread: Option<Thread>) {
        while let Some(child) = self.messages.first_child() {
            self.messages.remove(&child);
        }
        if let Some(thread) = thread {
            self.conversation_title.set_text(&thread.name);
            self.star_button.set_sensitive(true);
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
                let text = gtk::Label::builder()
                    .label(&message.body)
                    .xalign(0.0)
                    .wrap(true)
                    .selectable(true)
                    .max_width_chars(72)
                    .build();
                text.add_css_class("card");
                text.add_css_class("pantheon-surface");
                bubble.append(&text);
                let timestamp = gtk::Label::builder()
                    .label(&message.timestamp)
                    .xalign(if message.outgoing { 1.0 } else { 0.0 })
                    .build();
                timestamp.add_css_class("caption");
                timestamp.add_css_class("dim-label");
                bubble.append(&timestamp);
                self.messages.append(&bubble);
            }
            let permitted =
                thread.reply_ready && !thread.participants_required && !thread.roster_changed;
            self.editor.set_sensitive(permitted);
            self.send_button
                .set_sensitive(permitted && !self.state.borrow().pending.contains(&thread.key));
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
        if !thread.reply_ready || thread.participants_required || thread.roster_changed {
            self.status.set_text("BlueFerry has blocked replies to this conversation. Review its status before sending.");
            return;
        }
        if thread.is_group && !thread.group_confirmed {
            self.confirm_group(thread, body);
        } else {
            self.dispatch_send(thread.key, body, false, thread.confirmation_token);
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
                    app.dispatch_send(key.clone(), body.clone(), true, token.clone());
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
        self.feedback.set_text("Sending through BlueFerry…");
        let backend = self.backend.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = backend.send(&key, &body, confirm_group, &token).await;
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
                    Ok(()) => app.feedback.set_text("Message sent through your iPhone."),
                    Err(error) => app.feedback.set_text(&format!(
                        "Could not send. Your draft is kept; choose Send to retry. {error}"
                    )),
                }
                app.refresh();
            }
        });
    }
}
