use crate::media::{self, LinkPreview, PreviewKind};
use granite::prelude::PlaceholderExt;
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{
    BlueFerryMessagesClient as Backend, BlueFerryStatus, ContactMatch, Message, MessagePosition,
    Thread, format_message_timestamp, layout_messages,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const MAX_OUTBOX_MESSAGES: usize = 128;

#[derive(Default)]
struct State {
    threads: Vec<Thread>,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    pending: Vec<String>,
    outbox: Vec<OutboxMessage>,
    next_outbox_id: u64,
    map_ready: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboxStatus {
    Sending,
    Transferred,
    OutcomeUnknown,
}

#[derive(Clone, Debug)]
struct OutboxMessage {
    id: u64,
    key: String,
    body: String,
    timestamp: String,
    baseline: usize,
    status: OutboxStatus,
}

struct RenderedFrame {
    texture: gtk::gdk::MemoryTexture,
    delay: Duration,
}

struct RenderedPreview {
    title: String,
    kind: PreviewKind,
    source_size: (u32, u32),
    size: (u32, u32),
    frames: Vec<RenderedFrame>,
}

impl RenderedPreview {
    fn new(preview: LinkPreview) -> Rc<Self> {
        let (width, height) = preview.size;
        let frames = preview
            .frames
            .into_iter()
            .map(|frame| RenderedFrame {
                texture: gtk::gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(frame.rgba),
                    width as usize * 4,
                ),
                delay: frame.delay,
            })
            .collect();
        Rc::new(Self {
            title: preview.title,
            kind: preview.kind,
            source_size: preview.source_size,
            size: preview.size,
            frames,
        })
    }

    fn memory_bytes(&self) -> usize {
        self.frames
            .len()
            .saturating_mul(self.size.0 as usize)
            .saturating_mul(self.size.1 as usize)
            .saturating_mul(4)
    }
}

#[derive(Clone)]
struct MediaCard {
    root: gtk::Box,
    status: gtk::Label,
    load: gtk::Button,
    original: gtk::LinkButton,
    content: gtk::Box,
}

impl MediaCard {
    fn new(url: &str, outgoing: bool) -> Self {
        let youtube = media::is_youtube_url(url);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("card");
        root.add_css_class("pantheon-surface");
        root.add_css_class("pantheon-link-preview");
        root.set_halign(if outgoing {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        let status = gtk::Label::builder()
            .label(if youtube { "YouTube" } else { "Image" })
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .build();
        status.add_css_class("dim-label");
        let load = gtk::Button::with_label("Load Preview");
        load.set_halign(gtk::Align::Start);
        mseries_pantheon_kit::set_accessible_label(&load, "Load link preview");
        let original = gtk::LinkButton::with_label(
            url,
            if youtube {
                "Open YouTube"
            } else {
                "Open Image"
            },
        );
        mseries_pantheon_kit::set_accessible_label(
            &original,
            if youtube {
                "Open YouTube link"
            } else {
                "Open image link"
            },
        );
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.append(&status);
        root.append(&load);
        root.append(&original);
        Self {
            root,
            status,
            load,
            original,
            content,
        }
    }

    fn show_error(&self) {
        if self.root.root().is_none() {
            return;
        }
        self.status
            .set_text("Preview unavailable · open the link to view it.");
        self.status.add_css_class("dim-label");
        self.load.set_sensitive(true);
    }

    fn show_preview(&self, preview: Rc<RenderedPreview>) {
        if self.root.root().is_none() {
            return;
        }
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.status.set_visible(false);
        self.load.set_visible(false);
        let (width, height) = preview.size;
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .build();
        picture.set_size_request(width as i32, height as i32);
        picture.set_paintable(preview.frames.first().map(|frame| &frame.texture));
        mseries_pantheon_kit::set_accessible_label(&picture, &preview.title);
        self.content.append(&picture);
        let title = gtk::Label::builder()
            .label(&preview.title)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(36)
            .build();
        title.add_css_class("heading");
        self.content.append(&title);
        let (source_width, source_height) = preview.source_size;
        let dimensions = if preview.frames.len() > 1 {
            format!(
                "{source_width} × {source_height} · {} frames",
                preview.frames.len()
            )
        } else {
            format!("{source_width} × {source_height}")
        };
        let dimensions = gtk::Label::builder().label(&dimensions).xalign(0.0).build();
        dimensions.add_css_class("caption");
        dimensions.add_css_class("dim-label");
        self.content.append(&dimensions);
        self.original
            .set_label(if preview.kind == PreviewKind::Youtube {
                "Watch on YouTube"
            } else {
                "Open Original"
            });
        mseries_pantheon_kit::set_accessible_label(
            &self.original,
            if preview.kind == PreviewKind::Youtube {
                "Watch video on YouTube"
            } else {
                "Open original image"
            },
        );
        self.root.remove(&self.original);
        self.root.append(&self.content);
        self.root.append(&self.original);
        if preview.frames.len() > 1
            && gtk::Settings::default()
                .is_some_and(|settings| settings.property::<bool>("gtk-enable-animations"))
        {
            let first_delay = preview.frames[0].delay;
            schedule_preview_animation(picture, preview, 0, first_delay);
        }
    }
}

fn schedule_preview_animation(
    picture: gtk::Picture,
    preview: Rc<RenderedPreview>,
    index: usize,
    delay: Duration,
) {
    glib::timeout_add_local_once(delay, move || {
        if picture.root().is_none() || preview.frames.is_empty() {
            return;
        }
        let mapped = picture.is_mapped();
        let next = if mapped {
            (index + 1) % preview.frames.len()
        } else {
            index
        };
        if mapped {
            picture.set_paintable(Some(&preview.frames[next].texture));
        }
        let next_delay = if mapped {
            preview.frames[next].delay
        } else {
            Duration::from_millis(500)
        };
        schedule_preview_animation(picture, preview, next, next_delay);
    });
}

struct MediaState {
    jobs: Option<Sender<String>>,
    results: Receiver<(String, Result<LinkPreview, String>)>,
    pending: HashMap<String, Vec<MediaCard>>,
    cache: VecDeque<(String, Rc<RenderedPreview>)>,
    result_poll: Option<glib::SourceId>,
    cancelled: Arc<AtomicBool>,
}

impl MediaState {
    fn new() -> Self {
        let (job_sender, job_receiver) = mpsc::channel::<String>();
        let (result_sender, result_receiver) = mpsc::channel();
        let job_receiver = Arc::new(Mutex::new(job_receiver));
        let cancelled = Arc::new(AtomicBool::new(false));
        for _ in 0..2 {
            let jobs = Arc::clone(&job_receiver);
            let results = result_sender.clone();
            let cancelled = Arc::clone(&cancelled);
            thread::Builder::new()
                .name("mseries-message-preview".into())
                .spawn(move || {
                    loop {
                        let job = match jobs
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .recv()
                        {
                            Ok(job) => job,
                            Err(_) => break,
                        };
                        if cancelled.load(Ordering::Relaxed) {
                            break;
                        }
                        let result = media::load_preview(&job);
                        if results.send((job, result)).is_err() {
                            break;
                        }
                    }
                })
                .expect("could not start Messages preview worker");
        }
        Self {
            jobs: Some(job_sender),
            results: result_receiver,
            pending: HashMap::new(),
            cache: VecDeque::new(),
            result_poll: None,
            cancelled,
        }
    }

    fn take_cached(&mut self, url: &str) -> Option<Rc<RenderedPreview>> {
        let index = self.cache.iter().position(|(cached, _)| cached == url)?;
        let entry = self.cache.remove(index)?;
        let preview = Rc::clone(&entry.1);
        self.cache.push_back(entry);
        Some(preview)
    }

    fn cache(&mut self, url: String, preview: Rc<RenderedPreview>) {
        self.cache.retain(|(cached, _)| cached != &url);
        self.cache.push_back((url, preview));
        while self.cache.len() > 8
            || self
                .cache
                .iter()
                .map(|(_, item)| item.memory_bytes())
                .sum::<usize>()
                > 24 * 1024 * 1024
        {
            self.cache.pop_front();
        }
    }
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
    message_scroll: gtk::ScrolledWindow,
    editor: gtk::TextView,
    send_button: gtk::Button,
    copy_addresses_button: gtk::Button,
    star_button: gtk::ToggleButton,
    syncing_star: Cell<bool>,
    state: RefCell<State>,
    media: RefCell<MediaState>,
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
                let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                mseries_pantheon_kit::set_box_spacing_text_units(&content, 0.5);
                content.append(&placeholder);
                let retry = gtk::Button::with_label("Retry");
                retry.add_css_class("suggested-action");
                retry.set_halign(gtk::Align::Center);
                content.append(&retry);
                window.set_child(Some(&content));
                let retry_window = window.clone();
                let retry_application = application.clone();
                retry.connect_clicked(move |_| {
                    retry_window.close();
                    let application = retry_application.clone();
                    glib::idle_add_local_once(move || application.activate());
                });
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
        let shortcuts_button = gtk::Button::with_label("Keyboard Shortcuts");
        shortcuts_button.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&shortcuts_button, "Show keyboard shortcuts");
        let menu = gtk::Popover::new();
        menu.set_child(Some(&shortcuts_button));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Menu")
            .build();
        menu_button.set_popover(Some(&menu));
        mseries_pantheon_kit::set_accessible_label(&menu_button, "Menu");
        header.pack_end(&menu_button);

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
        mseries_pantheon_kit::set_accessible_label(&search, "Search conversations");
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
        mseries_pantheon_kit::set_accessible_label(&list, "Conversations");
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
        mseries_pantheon_kit::set_accessible_label(&star_button, "Favorite conversation");
        let copy_addresses_button = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_addresses_button.set_tooltip_text(Some("Copy recipient addresses"));
        copy_addresses_button.set_sensitive(false);
        mseries_pantheon_kit::set_accessible_label(
            &copy_addresses_button,
            "Copy recipient addresses",
        );
        let conversation_header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        mseries_pantheon_kit::set_box_spacing_text_units(&conversation_header, 0.6);
        mseries_pantheon_kit::set_margins_text_units(&conversation_header, 0.65, 0.75, 0.65, 1.0);
        let back_button = gtk::Button::with_label("Back");
        back_button.set_visible(false);
        mseries_pantheon_kit::set_accessible_label(&back_button, "Back to conversations");
        conversation_header.append(&back_button);
        conversation_header.append(&conversation_title);
        conversation_header.append(&copy_addresses_button);
        conversation_header.append(&star_button);

        let messages = gtk::Box::new(gtk::Orientation::Vertical, 0);
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
        mseries_pantheon_kit::set_accessible_label(&editor, "Message composer");
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
        send_button.set_tooltip_text(Some("Send message (Ctrl+Enter)"));
        mseries_pantheon_kit::set_accessible_label(&send_button, "Send message");
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
            message_scroll,
            editor,
            send_button,
            copy_addresses_button,
            star_button,
            syncing_star: Cell::new(false),
            state: RefCell::new(State::default()),
            media: RefCell::new(MediaState::new()),
        });
        app.connect_signals(
            &refresh,
            &new_message,
            &menu,
            &shortcuts_button,
            &app.copy_addresses_button,
        );
        app.connect_responsive_layout();
        app.watch_history();
        app.window.present();
        app.refresh();
        Some(app)
    }

    fn connect_signals(
        self: &Rc<Self>,
        refresh: &gtk::Button,
        new_message: &gtk::Button,
        menu: &gtk::Popover,
        shortcuts: &gtk::Button,
        copy_addresses: &gtk::Button,
    ) {
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.stop_media_workers();
            }
            glib::Propagation::Proceed
        });

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
        let menu = menu.clone();
        shortcuts.connect_clicked(move |_| {
            menu.popdown();
            if let Some(app) = weak.upgrade() {
                app.show_shortcuts();
            }
        });

        let weak = Rc::downgrade(self);
        copy_addresses.connect_clicked(move |button| {
            let Some(app) = weak.upgrade() else { return };
            let recipients = {
                let state = app.state.borrow();
                state
                    .selected
                    .as_ref()
                    .and_then(|selected| {
                        state.threads.iter().find(|thread| &thread.key == selected)
                    })
                    .map(|thread| thread.recipients.clone())
                    .unwrap_or_default()
            };
            if recipients.is_empty() {
                return;
            }
            button.clipboard().set_text(&recipients.join(", "));
            app.feedback.set_text(if recipients.len() == 1 {
                "Recipient address copied."
            } else {
                "Recipient addresses copied."
            });
        });

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if state.contains(gtk::gdk::ModifierType::ALT_MASK)
                && key == gtk::gdk::Key::Left
                && app.state.borrow().selected.is_some()
            {
                app.go_back_to_conversations();
                return glib::Propagation::Stop;
            }
            let control = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            if !control && key == gtk::gdk::Key::F5 {
                app.refresh();
                return glib::Propagation::Stop;
            }
            match (control, key) {
                (true, gtk::gdk::Key::f) => {
                    if app.window.has_css_class("pantheon-compact")
                        && app.state.borrow().selected.is_some()
                    {
                        app.show_conversation_list();
                    }
                    app.search.grab_focus();
                    glib::Propagation::Stop
                }
                (true, gtk::gdk::Key::r) => {
                    app.refresh();
                    glib::Propagation::Stop
                }
                (true, gtk::gdk::Key::n) => {
                    app.show_new_message_dialog();
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::Escape)
                    if app.search.has_focus() && !app.search.text().is_empty() =>
                {
                    app.search.set_text("");
                    glib::Propagation::Stop
                }
                (false, gtk::gdk::Key::Escape)
                    if app.window.has_css_class("pantheon-compact")
                        && app.state.borrow().selected.is_some() =>
                {
                    app.go_back_to_conversations();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);

        let composer_keys = gtk::EventControllerKey::new();
        composer_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        composer_keys.connect_key_pressed(move |_, key, _, state| {
            let is_send_shortcut = state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter);
            if !is_send_shortcut {
                return glib::Propagation::Proceed;
            }
            if let Some(app) = weak.upgrade()
                && app.send_button.is_sensitive()
            {
                app.send_current();
            }
            glib::Propagation::Stop
        });
        self.editor.add_controller(composer_keys);

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
                app.go_back_to_conversations();
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

    fn go_back_to_conversations(self: &Rc<Self>) {
        self.state.borrow_mut().selected = None;
        self.list.unselect_all();
        self.render_conversation(None);
        self.show_conversation_list();
        self.sync_favorite_selection();
    }

    fn show_shortcuts(self: &Rc<Self>) {
        let dialog = gtk::Window::builder()
            .title("Keyboard Shortcuts")
            .transient_for(&self.window)
            .modal(true)
            .destroy_with_parent(true)
            .default_width(380)
            .build();
        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("Keyboard Shortcuts"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        dialog.set_titlebar(Some(&header));

        let shortcuts = [
            ("New message", "Ctrl+N"),
            ("Search conversations", "Ctrl+F"),
            ("Refresh conversations", "Ctrl+R or F5"),
            ("Back to conversations", "Alt+Left"),
            ("Send message", "Ctrl+Enter"),
            ("Clear search / back in compact view", "Escape"),
        ];
        let rows = gtk::Grid::builder()
            .row_spacing(12)
            .column_spacing(28)
            .margin_start(20)
            .margin_end(20)
            .margin_top(16)
            .margin_bottom(20)
            .build();
        for (index, (description, shortcut)) in shortcuts.iter().enumerate() {
            let description = gtk::Label::builder()
                .label(*description)
                .xalign(0.0)
                .hexpand(true)
                .build();
            let shortcut = gtk::Label::builder().label(*shortcut).xalign(1.0).build();
            shortcut.add_css_class("dim-label");
            rows.attach(&description, 0, index as i32, 1, 1);
            rows.attach(&shortcut, 1, index as i32, 1, 1);
        }
        dialog.set_child(Some(&rows));
        dialog.present();
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
                                    state.drafts.entry(server_key.clone()).or_insert(draft);
                                }
                                for entry in &mut state.outbox {
                                    if entry.key == local.key {
                                        entry.key.clone_from(&server_key);
                                    }
                                }
                            } else {
                                keep_local.push(local);
                            }
                        }
                        server_threads.extend(keep_local);
                        state.threads = server_threads;
                        reconcile_outbox(&mut state);
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
        let filter = glib::casefold(self.search.text());
        let (threads, selected) = {
            let state = self.state.borrow();
            let threads = state
                .threads
                .iter()
                .filter(|thread| filter.is_empty() || thread.search_index.contains(filter.as_str()))
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
            let timestamp = if thread.is_group {
                "Group conversation".to_string()
            } else {
                format_message_timestamp(&thread.last_ts)
            };
            let meta = gtk::Label::builder()
                .label(&timestamp)
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
            let recipients = thread.recipients.join(", ");
            let accessible_name = if recipients.is_empty() {
                format!("{} conversation", thread.name)
            } else {
                format!("{} conversation with {}", thread.name, recipients)
            };
            mseries_pantheon_kit::set_accessible_label(&row, &accessible_name);
            self.attach_thread_actions(&row, thread);
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

    fn attach_thread_actions(self: &Rc<Self>, row: &gtk::ListBoxRow, thread: &Thread) {
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        mseries_pantheon_kit::set_margins_text_units(&actions, 0.45, 0.45, 0.45, 0.45);

        if thread.unread() > 0 && !thread.local_only {
            let mark_read = gtk::Button::with_label("Mark as Read");
            mark_read.add_css_class("flat");
            mseries_pantheon_kit::set_accessible_label(&mark_read, "Mark conversation as read");
            let weak_app = Rc::downgrade(self);
            let key = thread.key.clone();
            let mark_popover = popover.downgrade();
            mark_read.connect_clicked(move |_| {
                if let Some(popover) = mark_popover.upgrade() {
                    popover.popdown();
                }
                let Some(app) = weak_app.upgrade() else {
                    return;
                };
                let backend = app.backend.clone();
                let weak_app = Rc::downgrade(&app);
                let key = key.clone();
                glib::MainContext::default().spawn_local(async move {
                    match backend.mark_read(&key).await {
                        Ok(()) => {
                            if let Some(app) = weak_app.upgrade() {
                                app.feedback.set_text("Conversation marked as read.");
                                app.refresh();
                            }
                        }
                        Err(error) => {
                            if let Some(app) = weak_app.upgrade() {
                                app.status.set_text(&format!(
                                    "Could not mark conversation read: {error}"
                                ));
                            }
                        }
                    }
                });
            });
            actions.append(&mark_read);
        }

        if !thread.recipients.is_empty() {
            let label = if thread.recipients.len() == 1 {
                "Copy Address"
            } else {
                "Copy Addresses"
            };
            let copy = gtk::Button::with_label(label);
            copy.add_css_class("flat");
            mseries_pantheon_kit::set_accessible_label(&copy, label);
            let recipients = thread.recipients.join(", ");
            let feedback = if thread.recipients.len() == 1 {
                "Recipient address copied."
            } else {
                "Recipient addresses copied."
            };
            let weak_app = Rc::downgrade(self);
            let copy_popover = popover.downgrade();
            copy.connect_clicked(move |_| {
                if let Some(popover) = copy_popover.upgrade() {
                    popover.popdown();
                }
                if let Some(app) = weak_app.upgrade() {
                    gtk::prelude::WidgetExt::display(&app.window)
                        .clipboard()
                        .set_text(&recipients);
                    app.feedback.set_text(feedback);
                }
            });
            actions.append(&copy);
        }

        if actions.first_child().is_none() {
            return;
        }
        popover.set_child(Some(&actions));
        popover.set_parent(row);

        let right_click = gtk::GestureClick::builder().button(3).build();
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let right_popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            right_popover
                .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            right_popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        row.add_controller(right_click);

        let long_press = gtk::GestureLongPress::new();
        long_press.set_propagation_phase(gtk::PropagationPhase::Capture);
        let long_popover = popover.clone();
        long_press.connect_pressed(move |gesture, x, y| {
            long_popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            long_popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        row.add_controller(long_press);
        row.connect_destroy(move |_| popover.unparent());
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
        self.copy_addresses_button.set_sensitive(
            thread
                .as_ref()
                .is_some_and(|thread| !thread.recipients.is_empty()),
        );
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
            let pending_entries = self
                .state
                .borrow()
                .outbox
                .iter()
                .filter(|entry| entry.key == thread.key)
                .cloned()
                .collect::<Vec<_>>();
            if thread.messages.is_empty() && pending_entries.is_empty() {
                let empty = gtk::Label::new(Some("No messages in this conversation yet."));
                empty.add_css_class("dim-label");
                self.messages.append(&empty);
            }
            let outbox_messages = pending_entries
                .iter()
                .map(|entry| Message {
                    body: entry.body.clone(),
                    timestamp: entry.timestamp.clone(),
                    outgoing: true,
                    read: true,
                    ..Message::default()
                })
                .collect::<Vec<_>>();
            let mut visible_messages = thread.messages.iter().collect::<Vec<_>>();
            visible_messages.extend(outbox_messages.iter());
            let mut outbox_statuses = vec![None; thread.messages.len()];
            outbox_statuses.extend(pending_entries.iter().map(|entry| Some(entry.status)));
            let message_layout = layout_messages(visible_messages.iter().copied(), thread.is_group);
            for ((message, layout), outbox_status) in visible_messages
                .into_iter()
                .zip(message_layout)
                .zip(outbox_statuses)
            {
                if let Some(heading) = layout.date_heading.as_deref() {
                    let stamp = gtk::Label::builder()
                        .label(heading)
                        .halign(gtk::Align::Center)
                        .build();
                    stamp.add_css_class("caption");
                    stamp.add_css_class("dim-label");
                    stamp.add_css_class("pantheon-message-stamp");
                    mseries_pantheon_kit::set_accessible_label(&stamp, heading);
                    self.messages.append(&stamp);
                }
                let bubble = gtk::Box::new(gtk::Orientation::Vertical, 4);
                bubble.add_css_class(if message.outgoing {
                    "pantheon-message-outgoing"
                } else {
                    "pantheon-message-incoming"
                });
                bubble.add_css_class(match layout.position {
                    MessagePosition::Single => "pantheon-message-run-single",
                    MessagePosition::First => "pantheon-message-run-first",
                    MessagePosition::Middle => "pantheon-message-run-middle",
                    MessagePosition::Last => "pantheon-message-run-last",
                });
                bubble.set_halign(if message.outgoing {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                });
                if layout.show_sender {
                    let sender = gtk::Label::builder()
                        .label(&message.sender)
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .max_width_chars(32)
                        .build();
                    sender.add_css_class("caption");
                    sender.add_css_class("dim-label");
                    sender.add_css_class("pantheon-message-sender");
                    bubble.append(&sender);
                }
                let visible_body = message.visible_body();
                let mut media_cards = Vec::new();
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
                for url in media::media_links(&visible_body) {
                    let card = MediaCard::new(&url, message.outgoing);
                    media_cards.push((url, card));
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
                if let Some(timestamp) = layout.timestamp_caption.as_deref() {
                    let timestamp = gtk::Label::builder()
                        .label(timestamp)
                        .xalign(if message.outgoing { 1.0 } else { 0.0 })
                        .build();
                    timestamp.add_css_class("caption");
                    timestamp.add_css_class("dim-label");
                    bubble.append(&timestamp);
                }
                if let Some(outbox_status) = outbox_status {
                    let text = match outbox_status {
                        OutboxStatus::Sending => "Sending…",
                        OutboxStatus::Transferred => "Transferred to your iPhone",
                        OutboxStatus::OutcomeUnknown => {
                            "Status unknown · Check your iPhone before trying again"
                        }
                    };
                    let status = gtk::Label::builder()
                        .label(text)
                        .xalign(1.0)
                        .wrap(true)
                        .build();
                    status.add_css_class("caption");
                    status.add_css_class(if outbox_status == OutboxStatus::OutcomeUnknown {
                        "warning"
                    } else {
                        "dim-label"
                    });
                    mseries_pantheon_kit::set_accessible_label(&status, text);
                    bubble.append(&status);
                } else {
                    self.attach_message_actions(&bubble, message, thread.is_group);
                }
                self.messages.append(&bubble);
                for (url, card) in media_cards {
                    self.messages.append(&card.root);
                    self.request_media_preview(url, card, message.outgoing);
                }
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

    fn request_media_preview(self: &Rc<Self>, url: String, card: MediaCard, automatic: bool) {
        if let Some(preview) = self.media.borrow_mut().take_cached(&url) {
            card.show_preview(preview);
            return;
        }

        if let Some(cards) = self.media.borrow_mut().pending.get_mut(&url) {
            card.load.set_sensitive(false);
            cards.push(card);
            return;
        }

        let weak = Rc::downgrade(self);
        let requested_url = url.clone();
        let requested_card = card.clone();
        card.load.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.start_media_preview(requested_url.clone(), requested_card.clone());
            }
        });
        if automatic {
            self.start_media_preview(url, card);
        }
    }

    fn start_media_preview(self: &Rc<Self>, url: String, card: MediaCard) {
        if let Some(preview) = self.media.borrow_mut().take_cached(&url) {
            card.show_preview(preview);
            return;
        }
        let mut media = self.media.borrow_mut();
        if let Some(cards) = media.pending.get_mut(&url) {
            card.load.set_sensitive(false);
            cards.push(card);
            return;
        }
        if media.pending.len() >= 6 {
            card.status
                .set_text("Other previews are loading. Try again shortly.");
            return;
        }
        card.load.set_sensitive(false);
        card.status.set_text("Loading preview…");
        media.pending.insert(url.clone(), vec![card.clone()]);
        let submitted = media
            .jobs
            .as_ref()
            .is_some_and(|jobs| jobs.send(url.clone()).is_ok());
        if !submitted {
            media.pending.remove(&url);
            drop(media);
            card.show_error();
            return;
        }
        drop(media);
        self.ensure_media_poll();
    }

    fn ensure_media_poll(self: &Rc<Self>) {
        if self.media.borrow().result_poll.is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.drain_media_results();
            let mut media = app.media.borrow_mut();
            if media.pending.is_empty() {
                media.result_poll.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.media.borrow_mut().result_poll = Some(source);
    }

    fn drain_media_results(&self) {
        let mut ready = Vec::new();
        {
            let media = self.media.borrow();
            while let Ok(result) = media.results.try_recv() {
                ready.push(result);
            }
        }
        for (url, result) in ready {
            let cards = self
                .media
                .borrow_mut()
                .pending
                .remove(&url)
                .unwrap_or_default();
            if let Ok(preview) = result {
                let preview = RenderedPreview::new(preview);
                self.media
                    .borrow_mut()
                    .cache(url.clone(), Rc::clone(&preview));
                for card in cards {
                    card.show_preview(Rc::clone(&preview));
                }
            } else {
                for card in cards {
                    card.show_error();
                }
            }
        }
    }

    fn stop_media_workers(&self) {
        let mut media = self.media.borrow_mut();
        if let Some(source) = media.result_poll.take() {
            source.remove();
        }
        media.cancelled.store(true, Ordering::Relaxed);
        media.pending.clear();
        media.cache.clear();
        media.jobs.take();
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
        if self.state.borrow().outbox.len() >= MAX_OUTBOX_MESSAGES {
            self.feedback.set_text(
                "Recent messages are still waiting for history. Refresh before sending more.",
            );
            return;
        }
        let (outbox_id, selected_thread) = {
            let mut state = self.state.borrow_mut();
            let Some(thread) = state
                .threads
                .iter()
                .find(|thread| thread.key == key)
                .cloned()
            else {
                return;
            };
            let normalized = normalize_message_body(&body);
            let baseline = outgoing_echo_count(&thread.messages, &body)
                + state
                    .outbox
                    .iter()
                    .filter(|entry| {
                        entry.key == key && normalize_message_body(&entry.body) == normalized
                    })
                    .count();
            let outbox_id = state.next_outbox_id;
            state.next_outbox_id = state.next_outbox_id.wrapping_add(1);
            state.outbox.push(OutboxMessage {
                id: outbox_id,
                key: key.clone(),
                body: body.clone(),
                timestamp: now_iso8601(),
                baseline,
                status: OutboxStatus::Sending,
            });
            state.pending.push(key.clone());
            state
                .drafts
                .entry(key.clone())
                .or_insert_with(|| body.clone());
            let selected_thread =
                (state.selected.as_deref() == Some(key.as_str())).then_some(thread);
            (outbox_id, selected_thread)
        };
        self.send_button.set_sensitive(false);
        self.feedback.set_text("Sending to your iPhone…");
        if let Some(thread) = selected_thread {
            self.render_conversation(Some(thread));
            self.scroll_messages_to_bottom();
        }
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
                    match &result {
                        Ok(()) => {
                            if let Some(entry) = state
                                .outbox
                                .iter_mut()
                                .find(|entry| entry.id == outbox_id)
                            {
                                entry.status = OutboxStatus::Transferred;
                            }
                        }
                        Err(error) if error.outcome_unknown => {
                            if let Some(entry) = state
                                .outbox
                                .iter_mut()
                                .find(|entry| entry.id == outbox_id)
                            {
                                entry.status = OutboxStatus::OutcomeUnknown;
                            }
                        }
                        Err(_) => state.outbox.retain(|entry| entry.id != outbox_id),
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
                            thread.refresh_search_index();
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
                let selected_thread = {
                    let state = app.state.borrow();
                    if state.selected.as_deref() == Some(key.as_str()) {
                        state
                            .threads
                            .iter()
                            .find(|thread| thread.key == key)
                            .cloned()
                    } else {
                        None
                    }
                };
                if let Some(thread) = selected_thread {
                    app.render_conversation(Some(thread));
                    app.scroll_messages_to_bottom();
                }
                app.refresh();
            }
        });
    }

    fn scroll_messages_to_bottom(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                let adjustment = app.message_scroll.vadjustment();
                let bottom = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
                adjustment.set_value(bottom);
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
                let mut thread = Thread {
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
                };
                thread.refresh_search_index();
                state.threads.push(thread);
            }
        }
        self.search.set_text("");
        self.rebuild_sidebar();
        self.select_thread(&key);
        self.editor.grab_focus();
    }
}

fn now_iso8601() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|timestamp| timestamp.format_iso8601().ok())
        .map(|timestamp| timestamp.to_string())
        .unwrap_or_default()
}

fn normalize_message_body(body: &str) -> String {
    body.replace("\r\n", "\n").trim().to_string()
}

fn outgoing_echo_count(messages: &[Message], body: &str) -> usize {
    let wanted = normalize_message_body(body);
    messages
        .iter()
        .filter(|message| {
            message.outgoing && normalize_message_body(&message.body).as_str() == wanted
        })
        .count()
}

fn reconcile_outbox(state: &mut State) {
    let echoed = state
        .outbox
        .iter()
        .filter_map(|entry| {
            state
                .threads
                .iter()
                .find(|thread| thread.key == entry.key)
                .filter(|thread| {
                    outgoing_echo_count(&thread.messages, &entry.body) > entry.baseline
                })
                .map(|_| entry.id)
        })
        .collect::<HashSet<_>>();
    state.outbox.retain(|entry| !echoed.contains(&entry.id));
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
