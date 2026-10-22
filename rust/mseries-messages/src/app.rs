use crate::media::{self, LinkPreview, PreviewKind};
use crate::preferences;
use granite::prelude::PlaceholderExt;
use gtk::{gio, glib, prelude::*};
use mseries_pantheon_kit::{
    BlueFerryMessagesClient as Backend, BlueFerryStatus, ContactMatch, DockBadge, ImportedPinState,
    Message, MessagePosition, PhonePinStore, PinSnapshot, Polling, RecentMessage, ResumeWatcher,
    Thread, format_message_timestamp, is_valid_phone_profile_id, layout_messages,
    merge_recent_inbox, recipients_share_inbox_identity, suggested_pin_matches,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

const MAX_OUTBOX_MESSAGES: usize = 128;
const RECENT_INBOX_FOLDER: &str = "telecom/msg/INBOX";
const RECENT_INBOX_LIMIT: u32 = 200;
const RECENT_INBOX_MAX_AGE: Duration = Duration::from_secs(300);

fn ensure_refresh_phone_unchanged(
    before: &BlueFerryStatus,
    after: &BlueFerryStatus,
) -> Result<(), String> {
    if before.phone_profile_id != after.phone_profile_id || before.map != after.map {
        Err("The iPhone connection changed during refresh. Retry to load its conversations.".into())
    } else {
        Ok(())
    }
}

fn recent_inbox_due(
    map_ready: bool,
    was_map_ready: bool,
    has_snapshot: bool,
    force_requested: bool,
    last_success: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    if !map_ready {
        return false;
    }
    if force_requested || !was_map_ready || !has_snapshot {
        return true;
    }
    last_success.is_none_or(|last| {
        now.duration_since(last)
            .unwrap_or_default()
            .ge(&RECENT_INBOX_MAX_AGE)
    })
}

fn launch_phone_setup(feedback: &gtk::Label) {
    let Some(home) = std::env::var_os("HOME") else {
        feedback.set_text("Could not open Connect iPhone: HOME is not set.");
        feedback.add_css_class("warning");
        return;
    };
    let program = PathBuf::from(home).join(".local/bin/mseries-phone-setup");
    launch_helper(program.as_os_str(), "Connect iPhone", feedback);
}

fn launch_advanced_blueferry(feedback: &gtk::Label) {
    launch_helper(OsStr::new("blueferry-gtk"), "Advanced BlueFerry", feedback);
}

fn launch_helper(program: &OsStr, name: &str, feedback: &gtk::Label) {
    match Command::new(program).spawn() {
        Ok(mut child) => {
            thread::spawn(move || {
                let _ = child.wait();
            });
            feedback.remove_css_class("warning");
            feedback.set_text(&format!("Opened {name}."));
        }
        Err(error) => {
            feedback.add_css_class("warning");
            feedback.set_text(&format!("Could not open {name}: {error}"));
        }
    }
}

#[derive(Default)]
struct RefreshGate {
    in_flight: Cell<bool>,
    requested_again: Cell<bool>,
}

impl RefreshGate {
    /// Start one refresh, or coalesce another request behind the active one.
    fn request(&self) -> bool {
        if self.in_flight.replace(true) {
            self.requested_again.set(true);
            false
        } else {
            true
        }
    }

    /// Finish the current refresh and report whether one follow-up was queued.
    fn finish(&self) -> bool {
        self.in_flight.set(false);
        self.requested_again.replace(false)
    }
}

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

fn switch_phone_state(
    current: &mut State,
    parked: &mut HashMap<String, State>,
    previous_profile: Option<&str>,
    next_profile: Option<&str>,
) {
    let mut previous = std::mem::take(current);
    previous.selected = None;
    previous.pending.clear();
    previous.map_ready = false;
    previous.threads.retain(|thread| !thread.inbox_only);
    for entry in &mut previous.outbox {
        if entry.status == OutboxStatus::Sending {
            entry.status = OutboxStatus::OutcomeUnknown;
        }
    }
    if let Some(profile) = previous_profile {
        parked.insert(profile.to_string(), previous);
    }
    *current = next_profile
        .and_then(|profile| parked.remove(profile))
        .unwrap_or_default();
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
    dock_badge: DockBadge,
    history_subscription: RefCell<Option<gio::SignalSubscription>>,
    window: gtk::ApplicationWindow,
    status: gtk::Label,
    feedback: gtk::Label,
    pin_note: gtk::Label,
    pin_import_button: gtk::Button,
    pin_review_button: gtk::Button,
    auto_refresh_switch: gtk::Switch,
    enter_sends: Cell<bool>,
    enter_armed: Cell<bool>,
    enter_generation: Cell<u64>,
    refresh_poller: RefCell<Option<Polling>>,
    resume_watcher: RefCell<Option<ResumeWatcher>>,
    recent_inbox_snapshot: RefCell<Option<Vec<RecentMessage>>>,
    recent_inbox_last_success: Cell<Option<SystemTime>>,
    recent_inbox_force_generation: Cell<u64>,
    recent_inbox_satisfied_generation: Cell<u64>,
    recent_inbox_warning: RefCell<Option<String>>,
    pin_profile_id: RefCell<Option<String>>,
    pin_state: RefCell<ImportedPinState>,
    pin_load_generation: Cell<u64>,
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
    parked_phone_states: RefCell<HashMap<String, State>>,
    backend_available: Cell<bool>,
    refresh_gate: RefreshGate,
    media: RefCell<MediaState>,
}

impl App {
    pub fn build(application: &gtk::Application) -> Option<Rc<Self>> {
        let enter_sends = preferences::load_enter_sends();
        let auto_refresh = preferences::load_auto_refresh();
        let dock_badge = DockBadge::new("io.elementary.mseries.MessagesRust");
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
                placeholder.set_icon(&gio::ThemedIcon::new("mail-message-new-symbolic"));
                let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                mseries_pantheon_kit::set_box_spacing_text_units(&content, 0.5);
                content.append(&placeholder);
                let helper_feedback = gtk::Label::builder().xalign(0.0).wrap(true).build();
                helper_feedback.add_css_class("dim-label");
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                mseries_pantheon_kit::set_box_spacing_text_units(&actions, 0.5);
                let setup = gtk::Button::with_label("Connect iPhone…");
                mseries_pantheon_kit::set_accessible_label(&setup, "Connect iPhone");
                let setup_feedback = helper_feedback.clone();
                setup.connect_clicked(move |_| launch_phone_setup(&setup_feedback));
                let advanced = gtk::Button::with_label("Advanced BlueFerry…");
                mseries_pantheon_kit::set_accessible_label(&advanced, "Open Advanced BlueFerry");
                let advanced_feedback = helper_feedback.clone();
                advanced.connect_clicked(move |_| launch_advanced_blueferry(&advanced_feedback));
                let retry = gtk::Button::with_label("Retry");
                retry.add_css_class("suggested-action");
                mseries_pantheon_kit::set_accessible_label(&retry, "Retry BlueFerry connection");
                actions.append(&setup);
                actions.append(&advanced);
                actions.append(&retry);
                content.append(&actions);
                content.append(&helper_feedback);
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
        let pin_import_button = gtk::Button::with_label("Import iPhone Pins…");
        pin_import_button.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&pin_import_button, "Import iPhone pins");
        let pin_review_button = gtk::Button::with_label("Review Imported Pins…");
        pin_review_button.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&pin_review_button, "Review imported pins");
        pin_review_button.set_sensitive(false);
        let setup_button = gtk::Button::with_label("Connect iPhone…");
        setup_button.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&setup_button, "Connect iPhone");
        let advanced_button = gtk::Button::with_label("Advanced BlueFerry…");
        advanced_button.add_css_class("flat");
        mseries_pantheon_kit::set_accessible_label(&advanced_button, "Open Advanced BlueFerry");
        let auto_refresh_switch = gtk::Switch::builder()
            .active(auto_refresh)
            .valign(gtk::Align::Center)
            .build();
        mseries_pantheon_kit::set_accessible_label(
            &auto_refresh_switch,
            "Refresh conversations automatically every 30 seconds",
        );
        let refresh_description = gtk::Label::builder()
            .label("Refresh every 30 seconds while this window is open")
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(34)
            .build();
        refresh_description.add_css_class("dim-label");
        refresh_description.add_css_class("caption");
        let refresh_label = gtk::Label::builder()
            .label("Automatic refresh")
            .xalign(0.0)
            .build();
        let refresh_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        refresh_text.append(&refresh_label);
        refresh_text.append(&refresh_description);
        let refresh_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        refresh_row.set_valign(gtk::Align::Center);
        refresh_row.append(&refresh_text);
        refresh_row.append(&auto_refresh_switch);
        let enter_send_switch = gtk::Switch::builder()
            .active(enter_sends)
            .valign(gtk::Align::Center)
            .build();
        mseries_pantheon_kit::set_accessible_label(
            &enter_send_switch,
            "Send messages when Enter is pressed",
        );
        let enter_send_label = gtk::Label::builder()
            .label("Send with Enter")
            .xalign(0.0)
            .build();
        let enter_send_description = gtk::Label::builder()
            .label("Turn off to use Ctrl+Enter and keep Enter for new lines")
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(34)
            .build();
        enter_send_label.add_css_class("heading");
        enter_send_description.add_css_class("dim-label");
        enter_send_description.add_css_class("caption");
        let enter_send_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        enter_send_text.append(&enter_send_label);
        enter_send_text.append(&enter_send_description);
        let enter_send_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        enter_send_row.set_valign(gtk::Align::Center);
        enter_send_row.append(&enter_send_text);
        enter_send_row.append(&enter_send_switch);
        let menu_actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        mseries_pantheon_kit::set_margins_text_units(&menu_actions, 0.35, 0.35, 0.35, 0.35);
        menu_actions.append(&pin_import_button);
        menu_actions.append(&pin_review_button);
        menu_actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_actions.append(&setup_button);
        menu_actions.append(&advanced_button);
        menu_actions.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu_actions.append(&refresh_row);
        menu_actions.append(&enter_send_row);
        menu_actions.append(&shortcuts_button);
        let menu = gtk::Popover::new();
        menu.set_child(Some(&menu_actions));
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
        let pin_note = gtk::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(true)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .visible(false)
            .build();
        pin_note.add_css_class("caption");
        pin_note.add_css_class("dim-label");
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
        sidebar.append(&pin_note);
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
        send_button.set_tooltip_text(Some(if enter_sends {
            "Send message (Enter; Ctrl+Enter also works)"
        } else {
            "Send message (Ctrl+Enter)"
        }));
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
            dock_badge,
            history_subscription: RefCell::new(None),
            window,
            status,
            feedback,
            pin_note,
            pin_import_button,
            pin_review_button,
            auto_refresh_switch,
            enter_sends: Cell::new(enter_sends),
            enter_armed: Cell::new(false),
            enter_generation: Cell::new(0),
            refresh_poller: RefCell::new(None),
            resume_watcher: RefCell::new(None),
            recent_inbox_snapshot: RefCell::new(None),
            recent_inbox_last_success: Cell::new(None),
            recent_inbox_force_generation: Cell::new(0),
            recent_inbox_satisfied_generation: Cell::new(0),
            recent_inbox_warning: RefCell::new(None),
            pin_profile_id: RefCell::new(None),
            pin_state: RefCell::new(ImportedPinState::default()),
            pin_load_generation: Cell::new(0),
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
            parked_phone_states: RefCell::new(HashMap::new()),
            backend_available: Cell::new(true),
            refresh_gate: RefreshGate::default(),
            media: RefCell::new(MediaState::new()),
        });
        app.connect_signals(
            &refresh,
            &new_message,
            &menu,
            &shortcuts_button,
            &enter_send_switch,
        );
        app.connect_helper_buttons(&menu, &setup_button, &advanced_button);
        app.connect_responsive_layout();
        app.watch_history();
        app.install_refresh_controls();
        app.window.present();
        app.refresh_recent();
        Some(app)
    }

    fn connect_signals(
        self: &Rc<Self>,
        refresh: &gtk::Button,
        new_message: &gtk::Button,
        menu: &gtk::Popover,
        shortcuts: &gtk::Button,
        enter_send_switch: &gtk::Switch,
    ) {
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                if let Some(poller) = app.refresh_poller.borrow().as_ref() {
                    poller.set_enabled(false);
                }
                app.resume_watcher.borrow_mut().take();
                app.history_subscription.borrow_mut().take();
                app.stop_media_workers();
                app.enter_armed.set(false);
                app.enter_generation
                    .set(app.enter_generation.get().wrapping_add(1));
            }
            glib::Propagation::Proceed
        });

        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.refresh_recent();
            }
        });

        let weak = Rc::downgrade(self);
        new_message.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.show_new_message_dialog();
            }
        });

        let weak = Rc::downgrade(self);
        let shortcuts_menu = menu.clone();
        shortcuts.connect_clicked(move |_| {
            shortcuts_menu.popdown();
            if let Some(app) = weak.upgrade() {
                app.show_shortcuts();
            }
        });

        let weak = Rc::downgrade(self);
        let import_menu = menu.clone();
        self.pin_import_button.connect_clicked(move |_| {
            import_menu.popdown();
            if let Some(app) = weak.upgrade() {
                app.import_phone_pins();
            }
        });

        let weak = Rc::downgrade(self);
        let review_menu = menu.clone();
        self.pin_review_button.connect_clicked(move |_| {
            review_menu.popdown();
            if let Some(app) = weak.upgrade() {
                app.review_phone_pins();
            }
        });

        let weak = Rc::downgrade(self);
        enter_send_switch.connect_state_set(move |_, active| {
            if let Some(app) = weak.upgrade() {
                app.enter_sends.set(active);
                app.enter_armed.set(false);
                app.enter_generation
                    .set(app.enter_generation.get().wrapping_add(1));
                app.update_send_shortcut();
                if let Err(error) = preferences::save_enter_sends(active) {
                    let message = format!(
                        "Could not save the Enter-to-send preference. It applies for this session only. {error}"
                    );
                    app.feedback.set_text(&message);
                    app.feedback.add_css_class("warning");
                    app.feedback.set_tooltip_text(Some(&error.to_string()));
                } else {
                    app.feedback.remove_css_class("warning");
                    app.feedback.set_text("");
                    app.feedback.set_tooltip_text(None);
                }
            }
            glib::Propagation::Proceed
        });

        let weak = Rc::downgrade(self);
        self.copy_addresses_button.connect_clicked(move |button| {
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
                app.refresh_recent();
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
                    app.refresh_recent();
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
                if is_plain_enter(key, state)
                    && let Some(app) = weak.upgrade()
                    && app.enter_sends.get()
                {
                    app.arm_enter();
                }
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

        let buffer = self.editor.buffer();
        let weak = Rc::downgrade(self);
        buffer.connect_insert_text(move |buffer, _, text| {
            let Some(app) = weak.upgrade() else { return };
            let armed = app.enter_armed.replace(false);
            if !armed {
                return;
            }
            app.enter_generation
                .set(app.enter_generation.get().wrapping_add(1));
            if !should_send_after_enter_insert(armed, app.enter_sends.get(), text) {
                return;
            }
            buffer.stop_signal_emission_by_name("insert-text");
            let weak = Rc::downgrade(&app);
            glib::idle_add_local_once(move || {
                if let Some(app) = weak.upgrade()
                    && app.send_button.is_sensitive()
                {
                    app.send_current();
                }
            });
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

    fn connect_helper_buttons(
        self: &Rc<Self>,
        menu: &gtk::Popover,
        setup: &gtk::Button,
        advanced: &gtk::Button,
    ) {
        let weak = Rc::downgrade(self);
        let setup_menu = menu.clone();
        setup.connect_clicked(move |_| {
            setup_menu.popdown();
            if let Some(app) = weak.upgrade() {
                launch_phone_setup(&app.feedback);
            }
        });

        let weak = Rc::downgrade(self);
        let advanced_menu = menu.clone();
        advanced.connect_clicked(move |_| {
            advanced_menu.popdown();
            if let Some(app) = weak.upgrade() {
                launch_advanced_blueferry(&app.feedback);
            }
        });
    }

    fn install_refresh_controls(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let poller = Polling::new(Duration::from_secs(30), move || {
            if let Some(app) = weak.upgrade() {
                app.refresh();
            }
        });
        poller.bind_to_widget_mapped(&self.window);
        let poller_for_switch = poller.clone();
        let weak = Rc::downgrade(self);
        self.auto_refresh_switch
            .connect_state_set(move |_, active| {
                poller_for_switch.set_enabled(active);
                if let Some(app) = weak.upgrade() {
                    if let Err(error) = preferences::save_auto_refresh(active) {
                        app.feedback.set_text(&format!(
                            "Could not save automatic refresh preference: {error}"
                        ));
                        app.feedback.add_css_class("warning");
                    } else {
                        app.feedback.remove_css_class("warning");
                        app.feedback.set_text(if active {
                            "Automatic refresh enabled."
                        } else {
                            "Automatic refresh paused."
                        });
                    }
                }
                glib::Propagation::Proceed
            });
        poller.set_enabled(self.auto_refresh_switch.is_active());
        self.refresh_poller.replace(Some(poller));

        let weak = Rc::downgrade(self);
        self.resume_watcher
            .replace(Some(ResumeWatcher::new(move || {
                if let Some(app) = weak.upgrade() {
                    app.refresh_recent();
                }
            })));
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

        let send_shortcut = if self.enter_sends.get() {
            "Enter (Ctrl+Enter also works)"
        } else {
            "Ctrl+Enter"
        };
        let shortcuts = [
            ("New message", "Ctrl+N"),
            ("Search conversations", "Ctrl+F"),
            ("Refresh conversations", "Ctrl+R or F5"),
            ("Back to conversations", "Alt+Left"),
            ("Send message", send_shortcut),
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

    fn arm_enter(self: &Rc<Self>) {
        let generation = self.enter_generation.get().wrapping_add(1);
        self.enter_generation.set(generation);
        self.enter_armed.set(true);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(80), move || {
            if let Some(app) = weak.upgrade()
                && app.enter_generation.get() == generation
            {
                app.enter_armed.set(false);
            }
        });
    }

    fn update_send_shortcut(&self) {
        let state = self.state.borrow();
        let alpha_sender = state.selected.as_deref().and_then(|key| {
            state
                .threads
                .iter()
                .find(|thread| thread.key == key)
                .and_then(|thread| thread.sender_alphanumeric.as_ref())
        });
        self.send_button
            .set_tooltip_text(Some(if alpha_sender.is_some() {
                "Replies are unavailable for this alphanumeric sender"
            } else if self.enter_sends.get() {
                "Send message (Enter; Ctrl+Enter also works)"
            } else {
                "Send message (Ctrl+Enter)"
            }));
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
        self.refresh_with_recent(false);
    }

    fn refresh_recent(self: &Rc<Self>) {
        self.refresh_with_recent(true);
    }

    fn refresh_with_recent(self: &Rc<Self>, request_recent: bool) {
        if request_recent {
            self.recent_inbox_force_generation
                .set(self.recent_inbox_force_generation.get().saturating_add(1));
        }
        if !self.refresh_gate.request() {
            return;
        }
        let force_generation = self.recent_inbox_force_generation.get();
        let satisfied_generation = self.recent_inbox_satisfied_generation.get();
        let previous_phone_profile = self.pin_profile_id.borrow().clone();
        let was_map_ready = self.state.borrow().map_ready;
        let has_recent_snapshot = self.recent_inbox_snapshot.borrow().is_some();
        let recent_last_success = self.recent_inbox_last_success.get();
        self.status.set_text("Refreshing conversations…");
        let weak = Rc::downgrade(self);
        let backend = self.backend.clone();
        glib::MainContext::default().spawn_local(async move {
            let outcome = async {
                let status = backend.status().await?;
                let should_fetch_recent = recent_inbox_due(
                    status.map,
                    was_map_ready,
                    has_recent_snapshot,
                    force_generation > satisfied_generation
                        || status.phone_profile_id.as_ref() != previous_phone_profile.as_ref(),
                    recent_last_success,
                    SystemTime::now(),
                );
                let recent = if should_fetch_recent {
                    Some(
                        backend
                            .recent_inbox(RECENT_INBOX_FOLDER, RECENT_INBOX_LIMIT)
                            .await,
                    )
                } else {
                    None
                };
                let threads = backend.threads().await?;
                let final_status = backend.status().await?;
                ensure_refresh_phone_unchanged(&status, &final_status)?;
                Ok::<_, String>((final_status, threads, recent))
            }
            .await;
            if let Some(app) = weak.upgrade() {
                let refresh_again = app.refresh_gate.finish();
                match outcome {
                    Ok((status, threads, recent)) => {
                        let phone_changed = app.pin_profile_id.borrow().as_ref()
                            != status.phone_profile_id.as_ref();
                        if phone_changed {
                            // Revisions are scoped to a phone, not globally unique.
                            // Force the existing cache/generation invalidation below.
                            app.contact_photo_revision.replace(None);
                            app.recent_inbox_snapshot.replace(None);
                            app.recent_inbox_last_success.set(None);
                            app.recent_inbox_warning.replace(None);
                            switch_phone_state(
                                &mut app.state.borrow_mut(),
                                &mut app.parked_phone_states.borrow_mut(),
                                app.pin_profile_id.borrow().as_deref(),
                                status.phone_profile_id.as_deref(),
                            );
                            // No selection is active while clearing the previous editor text.
                            app.editor.buffer().set_text("");
                        }
                        app.backend_available.set(true);
                        app.update_contact_photo_revision(&status);
                        match recent {
                            Some(Ok(snapshot)) => {
                                app.recent_inbox_snapshot.replace(Some(snapshot));
                                app.recent_inbox_last_success.set(Some(SystemTime::now()));
                                app.recent_inbox_satisfied_generation
                                    .set(force_generation);
                                app.recent_inbox_warning.replace(None);
                            }
                            Some(Err(error)) => {
                                app.recent_inbox_warning.replace(Some(error));
                            }
                            None => {}
                        }
                        let ready = status.map;
                        let detail = status.connectivity_detail.as_deref().unwrap_or("");
                        app.status.remove_css_class("warning");
                        app.status.set_tooltip_text(None);
                        if ready
                            && let Some(error) = app.recent_inbox_warning.borrow().as_deref()
                        {
                            app.status.set_text(
                                "Could not refresh the iPhone inbox. Showing messages already received.",
                            );
                            app.status.set_tooltip_text(Some(error));
                            app.status.add_css_class("warning");
                        } else {
                            app.status.set_text(if ready {
                                "Connected to BlueFerry · iPhone Messages available"
                            } else if detail.is_empty() {
                                "BlueFerry is connected · iPhone Messages are unavailable"
                            } else {
                                detail
                            });
                        }
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
                            let matching_servers = server_threads
                                .iter()
                                .enumerate()
                                .filter(|(_, server)| {
                                    !server.is_group
                                        && server.recipients.len() == 1
                                        && server.recipients.iter().any(|recipient| {
                                            recipients_share_inbox_identity(recipient, address)
                                        })
                                })
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>();
                            if matching_servers.len() == 1 {
                                let server = &mut server_threads[matching_servers[0]];
                                let server_key = server.key.clone();
                                if local.inbox_only {
                                    let known_handles = server
                                        .messages
                                        .iter()
                                        .filter_map(|message| {
                                            (!message.handle.is_empty())
                                                .then_some(message.handle.clone())
                                        })
                                        .collect::<HashSet<_>>();
                                    server.messages.extend(
                                        local
                                            .messages
                                            .iter()
                                            .filter(|message| {
                                                message.phone_list
                                                    && (message.handle.is_empty()
                                                        || !known_handles.contains(
                                                            &message.handle,
                                                        ))
                                            })
                                            .cloned(),
                                    );
                                    if let Some(sender) = local.sender_alphanumeric.as_deref() {
                                        server.sender_alphanumeric = Some(sender.to_string());
                                        server.name = sender.to_string();
                                        server.reply_ready = false;
                                    }
                                    server.messages.sort_by(|left, right| {
                                        left.timestamp.cmp(&right.timestamp)
                                    });
                                    if let Some(last) = server.messages.last() {
                                        server.last_ts.clone_from(&last.timestamp);
                                    }
                                    server.refresh_search_index();
                                }
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
                        let snapshot = app.recent_inbox_snapshot.borrow();
                        state.threads = if let Some(snapshot) = snapshot.as_deref() {
                            merge_recent_inbox(server_threads, snapshot)
                        } else {
                            server_threads
                        };
                        let unread_count = state.threads.iter().fold(0_u64, |total, thread| {
                            total.saturating_add(thread.unread() as u64)
                        });
                        app.dock_badge.set_count(unread_count);
                        reconcile_outbox(&mut state);
                        drop(state);
                        app.sync_phone_pin_profile(status.phone_profile_id.clone());
                        app.rebuild_sidebar();
                        app.load_contact_photos();
                    }
                    Err(error) => {
                        app.state.borrow_mut().map_ready = false;
                        app.backend_available.set(false);
                        app.send_button.set_sensitive(false);
                        app.status.remove_css_class("warning");
                        app.status.set_text(&backend_error_summary(&error));
                        app.status.set_tooltip_text(Some(&error));
                        app.status.add_css_class("warning");
                        app.rebuild_sidebar();
                    }
                }
                if refresh_again {
                    app.refresh();
                }
            }
        });
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        let filter = glib::casefold(self.search.text());
        let (threads, selected) = {
            let state = self.state.borrow();
            let pin_order = self.pin_state.borrow().ordered_thread_keys(&state.threads);
            let rank = pin_order
                .iter()
                .enumerate()
                .map(|(index, key)| (key.as_str(), index))
                .collect::<HashMap<_, _>>();
            let mut threads = state
                .threads
                .iter()
                .filter(|thread| filter.is_empty() || thread.search_index.contains(filter.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            threads
                .sort_by_key(|thread| rank.get(thread.key.as_str()).copied().unwrap_or(usize::MAX));
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
        if let Some(index) = selected_index {
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
            let phone_generation = self.pin_load_generation.get();
            let mark_popover = popover.downgrade();
            mark_read.connect_clicked(move |_| {
                if let Some(popover) = mark_popover.upgrade() {
                    popover.popdown();
                }
                let Some(app) = weak_app.upgrade() else {
                    return;
                };
                if app.pin_load_generation.get() != phone_generation {
                    return;
                }
                let backend = app.backend.clone();
                let weak_app = Rc::downgrade(&app);
                let key = key.clone();
                glib::MainContext::default().spawn_local(async move {
                    match backend.mark_read(&key).await {
                        Ok(()) => {
                            if let Some(app) = weak_app.upgrade()
                                && app.pin_load_generation.get() == phone_generation
                            {
                                app.feedback.set_text("Conversation marked as read.");
                                app.refresh();
                            }
                        }
                        Err(error) => {
                            if let Some(app) = weak_app.upgrade()
                                && app.pin_load_generation.get() == phone_generation
                            {
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

    fn sync_phone_pin_profile(self: &Rc<Self>, reported: Option<String>) {
        let profile = reported.filter(|value| is_valid_phone_profile_id(value));
        if self.pin_profile_id.borrow().as_ref() == profile.as_ref() {
            return;
        }
        *self.pin_profile_id.borrow_mut() = profile.clone();
        *self.pin_state.borrow_mut() = ImportedPinState::default();
        self.pin_load_generation
            .set(self.pin_load_generation.get().wrapping_add(1));
        let generation = self.pin_load_generation.get();
        self.pin_review_button.set_sensitive(false);
        self.update_pin_note();
        let Some(profile) = profile else {
            return;
        };

        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = PhonePinStore::load(&profile).await;
            let Some(app) = weak.upgrade() else { return };
            if app.pin_load_generation.get() != generation
                || app.pin_profile_id.borrow().as_deref() != Some(profile.as_str())
            {
                return;
            }
            match result {
                Ok(state) => {
                    *app.pin_state.borrow_mut() = state;
                    app.update_pin_note();
                    app.rebuild_sidebar();
                }
                Err(error) => {
                    app.pin_note.set_text(&format!(
                        "Could not open imported pins from the desktop keyring: {error}"
                    ));
                    app.pin_note.set_visible(true);
                }
            }
        });
    }

    fn update_pin_note(&self) {
        let state = self.pin_state.borrow();
        let Some(snapshot) = state.snapshot.as_ref() else {
            self.pin_note.set_text("");
            self.pin_note.set_tooltip_text(None);
            self.pin_note.set_visible(false);
            self.pin_review_button.set_sensitive(false);
            return;
        };
        let current_threads = self.state.borrow();
        let thread_keys = current_threads
            .threads
            .iter()
            .map(|thread| thread.key.as_str())
            .collect::<HashSet<_>>();
        let missing = state
            .suggested_matches(snapshot, &current_threads.threads)
            .iter()
            .filter(|choice| {
                choice
                    .as_ref()
                    .is_none_or(|key| !thread_keys.contains(key.as_str()))
            })
            .count();
        drop(current_threads);
        let stamp = format_message_timestamp(&snapshot.captured_at);
        let note = if snapshot.pins.is_empty() {
            format!("No Apple conversation pins · {stamp}")
        } else {
            let mut note = format!("Pins imported from Mac · {stamp}");
            if missing > 0 {
                note.push_str(&format!(" · {missing} need matching"));
            }
            note
        };
        self.pin_note.set_text(&note);
        let mut tooltip = String::from(
            "This is an ordered Mac snapshot. Changes on the iPhone require a new export; review unmatched conversations here.",
        );
        if let Some(modified) = snapshot.source_modified_at.as_deref() {
            tooltip.push_str(&format!(
                "\nApple pin settings last changed: {}",
                format_message_timestamp(modified)
            ));
        }
        self.pin_note.set_tooltip_text(Some(&tooltip));
        self.pin_note.set_visible(true);
        self.pin_review_button.set_sensitive(true);
    }

    fn import_phone_pins(self: &Rc<Self>) {
        if self.pin_profile_id.borrow().is_none() {
            self.status
                .set_text("Update BlueFerry and connect your iPhone before importing phone pins.");
            return;
        }
        let weak = Rc::downgrade(self);
        let parent = self.window.clone();
        glib::MainContext::default().spawn_local(async move {
            let dialog = gtk::FileDialog::builder()
                .title("Import iPhone Pins")
                .modal(true)
                .build();
            let file = match dialog.open_future(Some(&parent)).await {
                Ok(file) => file,
                Err(_) => return,
            };
            let Some(path) = file.path() else {
                if let Some(app) = weak.upgrade() {
                    app.status.set_text("Choose a local Apple pin export file.");
                }
                return;
            };
            match PinSnapshot::read(&path) {
                Ok(snapshot) => {
                    if let Some(app) = weak.upgrade() {
                        app.show_pin_review(snapshot);
                    }
                }
                Err(error) => {
                    if let Some(app) = weak.upgrade() {
                        app.status.set_text(&format!(
                            "Could not read that pin export. Use Export-Message-Pins on your Mac. {error}"
                        ));
                    }
                }
            }
        });
    }

    fn review_phone_pins(self: &Rc<Self>) {
        let snapshot = self.pin_state.borrow().snapshot.clone();
        if let Some(snapshot) = snapshot {
            self.show_pin_review(snapshot);
        } else {
            self.import_phone_pins();
        }
    }

    fn show_pin_review(self: &Rc<Self>, snapshot: PinSnapshot) {
        let Some(profile) = self.pin_profile_id.borrow().clone() else {
            self.status
                .set_text("Update BlueFerry and connect your iPhone before importing phone pins.");
            return;
        };
        let threads = self.state.borrow().threads.clone();
        let previous = self.pin_state.borrow().clone();
        let choices = if previous.snapshot.as_ref() == Some(&snapshot) {
            previous.choices.clone()
        } else {
            suggested_pin_matches(&snapshot, &threads, Some(&previous))
        };
        let mut candidates = threads.clone();
        candidates.sort_by_key(|thread| thread.name.to_lowercase());
        let keys = candidates
            .iter()
            .map(|thread| thread.key.clone())
            .collect::<Vec<_>>();
        let mut labels = vec!["Not matched yet".to_string()];
        labels.extend(candidates.iter().map(|thread| {
            if thread.recipients.is_empty() {
                thread.name.clone()
            } else {
                format!("{} · {}", thread.name, thread.recipients.join(", "))
            }
        }));
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();

        let dialog = gtk::Window::builder()
            .title("Import iPhone Pins")
            .transient_for(&self.window)
            .destroy_with_parent(true)
            .modal(true)
            .default_width(500)
            .default_height(560)
            .build();
        let header = gtk::HeaderBar::new();
        let title = gtk::Label::new(Some("Import iPhone Pins"));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        dialog.set_titlebar(Some(&header));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        mseries_pantheon_kit::set_margins_text_units(&content, 0.8, 0.9, 0.8, 0.9);
        let explanation = gtk::Label::builder()
            .label("Match Apple’s ordered pin list with conversations on this computer. Groups need your selection. This saves a local snapshot and does not change your iPhone.")
            .xalign(0.0)
            .wrap(true)
            .build();
        explanation.add_css_class("dim-label");
        content.append(&explanation);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let mut dropdowns = Vec::with_capacity(snapshot.pins.len());
        for (index, pin) in snapshot.pins.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 5);
            row.add_css_class("card");
            mseries_pantheon_kit::set_margins_text_units(&row, 0.45, 0.5, 0.45, 0.5);
            let identifier = gtk::Label::builder()
                .label(format!("{}. {}", index + 1, pin.apple_identifier))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .max_width_chars(48)
                .tooltip_text(&pin.apple_identifier)
                .build();
            mseries_pantheon_kit::set_accessible_label(
                &identifier,
                &format!("Apple pin {}: {}", index + 1, pin.apple_identifier),
            );
            row.append(&identifier);
            let dropdown = gtk::DropDown::from_strings(&label_refs);
            let selected = choices
                .get(index)
                .and_then(Option::as_ref)
                .and_then(|key| keys.iter().position(|candidate| candidate == key))
                .map_or(0, |index| index as u32 + 1);
            dropdown.set_selected(selected);
            mseries_pantheon_kit::set_accessible_label(
                &dropdown,
                &format!("Conversation for pin {}", index + 1),
            );
            row.append(&dropdown);
            rows.append(&row);
            dropdowns.push(dropdown);
        }
        if snapshot.pins.is_empty() {
            let empty = gtk::Label::builder()
                .label("Apple Messages exported an empty pin list. Importing it clears this computer’s imported pin order.")
                .xalign(0.0)
                .wrap(true)
                .build();
            empty.add_css_class("dim-label");
            rows.append(&empty);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&rows)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        content.append(&scroll);
        let error = gtk::Label::builder()
            .label("")
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        error.add_css_class("error");
        content.append(&error);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label("Cancel");
        let apply = gtk::Button::with_label("Import Pins");
        apply.add_css_class("suggested-action");
        actions.append(&cancel);
        actions.append(&apply);
        content.append(&actions);
        dialog.set_child(Some(&content));
        dialog.set_default_widget(Some(&apply));
        let close_dialog = dialog.clone();
        cancel.connect_clicked(move |_| close_dialog.close());

        let weak = Rc::downgrade(self);
        let snapshot_for_save = snapshot.clone();
        let profile_for_save = profile.clone();
        let dropdowns = Rc::new(dropdowns);
        let keys = Rc::new(keys);
        let error_label = error.clone();
        let apply_button = apply.clone();
        let dialog_for_save = dialog.clone();
        apply.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else { return };
            let choices = dropdowns
                .iter()
                .map(|dropdown| {
                    let selected = dropdown.selected();
                    selected
                        .checked_sub(1)
                        .and_then(|index| keys.get(index as usize))
                        .cloned()
                })
                .collect::<Vec<_>>();
            let state = match ImportedPinState::reviewed(
                snapshot_for_save.clone(),
                choices,
                &threads,
            ) {
                Ok(state) => state,
                Err(message) => {
                    error_label.set_text(&message);
                    error_label.set_visible(true);
                    return;
                }
            };
            apply_button.set_sensitive(false);
            let backend = app.backend.clone();
            let weak_app = Rc::downgrade(&app);
            let profile = profile_for_save.clone();
            let error_label_async = error_label.clone();
            let apply_button_async = apply_button.clone();
            let dialog_async = dialog_for_save.clone();
            glib::MainContext::default().spawn_local(async move {
                let result = async {
                    let current = backend.status().await?;
                    if current.phone_profile_id.as_deref() != Some(profile.as_str()) {
                        return Err("The connected phone changed. Refresh and review the pin import again.".to_string());
                    }
                    PhonePinStore::save(&profile, &state).await
                }
                .await;
                let Some(app) = weak_app.upgrade() else { return };
                if app.pin_profile_id.borrow().as_deref() != Some(profile.as_str()) {
                    return;
                }
                match result {
                    Ok(()) => {
                        *app.pin_state.borrow_mut() = state;
                        app.update_pin_note();
                        app.rebuild_sidebar();
                        dialog_async.close();
                    }
                    Err(message) => {
                        error_label_async.set_text(&format!(
                            "Could not save pins in the desktop keyring. Your previous pins are kept. {message}"
                        ));
                        error_label_async.set_visible(true);
                        apply_button_async.set_sensitive(true);
                    }
                }
            });
        });
        dialog.present();
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
            icon.set_overflow(gtk::Overflow::Hidden);
            icon.add_css_class("pantheon-avatar");
            return icon.upcast();
        };
        picture.set_size_request(size, size);
        picture.set_overflow(gtk::Overflow::Hidden);
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
            let phone_generation = self.pin_load_generation.get();
            let backend = self.backend.clone();
            let weak = Rc::downgrade(self);
            let key = thread.key.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(error) = backend.mark_read(&key).await
                    && let Some(app) = weak.upgrade()
                    && app.pin_load_generation.get() == phone_generation
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
            let permitted =
                thread.reply_ready && !thread.participants_required && !thread.roster_changed;
            self.update_send_shortcut();
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
            let backend_available = self.backend_available.get();
            self.conversation_title.set_text(if backend_available {
                "Select a conversation"
            } else {
                "Connect iPhone to get started"
            });
            self.star_button.set_sensitive(false);
            self.editor.set_sensitive(false);
            self.send_button.set_sensitive(false);
            let empty = granite::Placeholder::new(if backend_available {
                "Your conversations appear here"
            } else {
                "BlueFerry is unavailable"
            });
            empty.set_description(if backend_available {
                "Choose a conversation from the list to read and reply."
            } else {
                "Set up your iPhone connection, open BlueFerry, or retry."
            });
            empty.set_icon(&gio::ThemedIcon::new("mail-message-new-symbolic"));
            if !backend_available {
                let setup = empty.append_button(
                    &gio::ThemedIcon::new("phone-symbolic"),
                    "Connect iPhone…",
                    "Open Phone Setup",
                );
                mseries_pantheon_kit::set_accessible_label(&setup, "Connect iPhone");
                let feedback = self.feedback.clone();
                setup.connect_clicked(move |_| launch_phone_setup(&feedback));

                let advanced = empty.append_button(
                    &gio::ThemedIcon::new("preferences-system-symbolic"),
                    "Advanced BlueFerry…",
                    "Open BlueFerry connection settings",
                );
                mseries_pantheon_kit::set_accessible_label(&advanced, "Open Advanced BlueFerry");
                let feedback = self.feedback.clone();
                advanced.connect_clicked(move |_| launch_advanced_blueferry(&feedback));

                let retry = empty.append_button(
                    &gio::ThemedIcon::new("view-refresh-symbolic"),
                    "Retry",
                    "Check the BlueFerry connection again",
                );
                mseries_pantheon_kit::set_accessible_label(&retry, "Retry BlueFerry connection");
                let weak = Rc::downgrade(self);
                retry.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.refresh_recent();
                    }
                });
            }
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
        let phone_generation = self.pin_load_generation.get();
        let backend = self.backend.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = backend.set_starred(&key, starred).await;
            if let Some(app) = weak.upgrade() {
                if app.pin_load_generation.get() != phone_generation {
                    return;
                }
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
        if !thread.reply_ready {
            self.status.set_text("BlueFerry has blocked replies to this conversation. Review its status before sending.");
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
        if thread.participants_required || thread.roster_changed {
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
        let phone_generation = self.pin_load_generation.get();
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
                    if app.pin_load_generation.get() != phone_generation {
                        app.feedback.set_text(
                            "The connected phone changed. Review the conversation before sending.",
                        );
                        return;
                    }
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
        let phone_generation = self.pin_load_generation.get();
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
                if app.pin_load_generation.get() != phone_generation {
                    return;
                }
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
        let phone_generation = self.pin_load_generation.get();
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
                if app.pin_load_generation.get() != phone_generation {
                    app.feedback
                        .set_text("The connected phone changed. Open a new message again.");
                    return;
                }
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
                if app.pin_load_generation.get() != phone_generation {
                    app.feedback
                        .set_text("The connected phone changed. Open a new message again.");
                    return;
                }
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
                if app.pin_load_generation.get() != phone_generation {
                    app.feedback
                        .set_text("The connected phone changed. Open a new message again.");
                    return;
                }
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
                if app.pin_load_generation.get() != phone_generation {
                    app.feedback
                        .set_text("The connected phone changed. Open a new message again.");
                    return;
                }
                let backend = app.backend.clone();
                let results = results.clone();
                let matches = matches.clone();
                let hint = hint.clone();
                let start = start.clone();
                let generation = generation.clone();
                let alive = alive.clone();
                glib::MainContext::default().spawn_local(async move {
                    let result = backend.find_contacts(&text).await;
                    if !alive.get()
                        || generation.get() != next
                        || app.pin_load_generation.get() != phone_generation
                    {
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

fn backend_error_summary(error: &str) -> String {
    if error.contains("org.freedesktop.DBus.Error.ServiceUnknown")
        || error.contains("org.freedesktop.DBus.Error.NameHasNoOwner")
    {
        "BlueFerry could not be reached. Check its connection in Phone Setup, then retry."
            .to_owned()
    } else {
        error.to_owned()
    }
}

fn normalize_message_body(body: &str) -> String {
    body.replace("\r\n", "\n").trim().to_string()
}

fn should_send_after_enter_insert(armed: bool, enter_sends: bool, inserted_text: &str) -> bool {
    armed && enter_sends && inserted_text == "\n"
}

fn is_plain_enter(key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
    matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
        && !state.intersects(
            gtk::gdk::ModifierType::SHIFT_MASK
                | gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK
                | gtk::gdk::ModifierType::HYPER_MASK
                | gtk::gdk::ModifierType::META_MASK,
        )
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

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires an isolated Xvfb display, D-Bus session and config directories"]
    fn offline_gui_disables_sending_and_preserves_drafts_on_back() {
        use gtk::prelude::*;
        assert_eq!(
            std::env::var("MSERIES_MESSAGES_ISOLATED_GUI").as_deref(),
            Ok("1")
        );
        gtk::init().expect("isolated GTK display");
        granite::init();
        let application = gtk::Application::builder()
            .application_id("io.elementary.mseries.MessagesRust.Test")
            .build();
        application
            .register(None::<&gtk::gio::Cancellable>)
            .unwrap();
        let app = super::App::build(&application).expect("private session bus");
        let wait_for_refresh = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let context = gtk::glib::MainContext::default();
            while app.refresh_gate.in_flight.get() {
                assert!(std::time::Instant::now() < deadline, "refresh timed out");
                context.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        wait_for_refresh();
        assert!(!app.backend_available.get());
        assert!(!app.state.borrow().map_ready);
        assert!(app.status.has_css_class("warning"));
        assert_eq!(
            app.conversation_title.text(),
            "Connect iPhone to get started"
        );
        assert!(!app.send_button.is_sensitive());
        assert!(!app.editor.is_sensitive());
        app.state
            .borrow_mut()
            .drafts
            .insert("test-thread".into(), "unsent draft".into());
        app.state.borrow_mut().selected = Some("test-thread".into());
        app.go_back_to_conversations();
        assert!(app.state.borrow().selected.is_none());
        assert_eq!(app.state.borrow().drafts["test-thread"], "unsent draft");
        app.refresh_recent();
        wait_for_refresh();
        assert!(!app.backend_available.get());
        assert!(!app.send_button.is_sensitive());
        assert_eq!(app.state.borrow().drafts["test-thread"], "unsent draft");
        // Publish only read methods on this test's private bus. Sends are unsupported.
        use gtk::glib::variant::ToVariant;
        let bus = gtk::gio::bus_get_sync(gtk::gio::BusType::Session, gtk::gio::Cancellable::NONE)
            .unwrap();
        bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&("io.weirdware.BlueFerry", 0_u32).to_variant()),
            None,
            gtk::gio::DBusCallFlags::NONE,
            1000,
            gtk::gio::Cancellable::NONE,
        )
        .unwrap();
        let node = gtk::gio::DBusNodeInfo::for_xml(r#"<node><interface name="io.weirdware.BlueFerry.Messages1">
          <method name="SendToThreadChecked"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="b" direction="in"/><arg type="s" direction="in"/></method>
          <method name="GetStatus"><arg type="s" direction="out"/></method>
          <method name="ListThreads"><arg type="u" direction="in"/><arg type="s" direction="out"/></method>
          <method name="ListRecent"><arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="out"/></method>
        </interface></node>"#).unwrap();
        let fixture_profile = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let reported_profile = fixture_profile.clone();
        let api_version = std::rc::Rc::new(std::cell::Cell::new(2_u64));
        let fixture_api_version = api_version.clone();
        let send_mode = std::rc::Rc::new(std::cell::Cell::new(0_u8));
        let fixture_send_mode = send_mode.clone();
        let switch_during_inbox = std::rc::Rc::new(std::cell::Cell::new(false));
        let switch_request = switch_during_inbox.clone();
        let registration = bus
            .register_object("/io/weirdware/BlueFerry", &node.interfaces()[0])
            .method_call(move |_, _, _, _, method, _, invocation| {
                if method == "SendToThreadChecked" {
                    match fixture_send_mode.get() {
                        0 => invocation.return_value(Some(&().to_variant())),
                        1 => invocation.return_dbus_error("io.weirdware.BlueFerry.Error.Failed", "Synthetic known failure"),
                        _ => invocation.return_dbus_error("io.weirdware.BlueFerry.Error.SendOutcomeUnknown", "Synthetic uncertain transfer"),
                    }
                    return;
                }
                let json = match method {
                    "GetStatus" => serde_json::json!({"api_version":fixture_api_version.get(),"map":true,"phone_profile_id":*reported_profile.borrow()}).to_string(),
                    "ListThreads" => serde_json::json!([{
                        "key":"shared-key", "name":"Synthetic Contact",
                        "recipients":["+12025550123"], "reply_ready":true,
                        "messages":[{"handle":"fixture-1", "body":"Synthetic history message", "read":true}]
                    }]).to_string(),
                    "ListRecent" => {
                        if switch_request.replace(false) {
                            *reported_profile.borrow_mut() = Some("87654321-1234-4234-8234-123456789abc".into());
                        }
                        "[]".to_string()
                    },
                    _ => panic!("unexpected fixture method: {method}"),
                };
                invocation.return_value(Some(&(json,).to_variant()));
            })
            .build()
            .unwrap();
        app.refresh_recent();
        wait_for_refresh();
        assert!(app.backend_available.get());
        assert!(app.state.borrow().map_ready);
        assert!(!app.status.has_css_class("warning"));
        assert!(!app.send_button.is_sensitive()); // no conversation selected
        assert_eq!(app.state.borrow().drafts["test-thread"], "unsent draft");
        assert!(app.state.borrow().selected.is_none());
        app.select_thread("shared-key");
        assert_eq!(app.conversation_title.text(), "Synthetic Contact");
        assert!(app.editor.is_sensitive());
        assert!(app.send_button.is_sensitive());
        assert!(app.messages.first_child().is_some());
        app.editor.buffer().set_text("Composer draft");
        assert_eq!(app.state.borrow().drafts["shared-key"], "Composer draft");
        app.go_back_to_conversations();
        app.select_thread("shared-key");
        let buffer = app.editor.buffer();
        assert_eq!(
            buffer.text(&buffer.start_iter(), &buffer.end_iter(), false),
            "Composer draft"
        );
        app.go_back_to_conversations();
        let phone_a = "12345678-1234-4234-8234-123456789abc";
        let phone_b = "87654321-1234-4234-8234-123456789abc";
        *fixture_profile.borrow_mut() = Some(phone_a.into());
        app.refresh_recent();
        wait_for_refresh();
        app.state
            .borrow_mut()
            .drafts
            .insert("shared-key".into(), "A draft".into());
        *fixture_profile.borrow_mut() = Some(phone_b.into());
        app.refresh_recent();
        wait_for_refresh();
        assert!(app.backend_available.get());
        assert!(!app.state.borrow().drafts.contains_key("shared-key"));
        app.state
            .borrow_mut()
            .drafts
            .insert("shared-key".into(), "B draft".into());
        *fixture_profile.borrow_mut() = Some(phone_a.into());
        app.refresh_recent();
        wait_for_refresh();
        assert_eq!(app.state.borrow().drafts["shared-key"], "A draft");
        assert_eq!(
            app.parked_phone_states.borrow()[phone_b].drafts["shared-key"],
            "B draft"
        );
        app.select_thread("shared-key");
        assert!(app.send_button.is_sensitive());
        switch_during_inbox.set(true);
        app.refresh_recent();
        wait_for_refresh();
        assert!(!app.backend_available.get());
        assert!(!app.state.borrow().map_ready);
        assert!(!app.send_button.is_sensitive());
        assert_eq!(app.pin_profile_id.borrow().as_deref(), Some(phone_a));
        assert_eq!(app.state.borrow().drafts["shared-key"], "A draft");
        app.refresh_recent();
        wait_for_refresh();
        assert!(app.backend_available.get());
        assert_eq!(app.pin_profile_id.borrow().as_deref(), Some(phone_b));
        assert_eq!(app.state.borrow().drafts["shared-key"], "B draft");
        app.select_thread("shared-key");
        let wait_for_send = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !app.state.borrow().pending.is_empty() || app.refresh_gate.in_flight.get() {
                assert!(std::time::Instant::now() < deadline, "send timed out");
                gtk::glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        app.editor.buffer().set_text("Synthetic transfer");
        app.send_current();
        wait_for_send();
        assert_eq!(app.state.borrow().outbox.len(), 1);
        assert_eq!(
            app.state.borrow().outbox[0].status,
            super::OutboxStatus::Transferred
        );
        assert!(!app.state.borrow().drafts.contains_key("shared-key"));
        send_mode.set(1);
        app.editor.buffer().set_text("Synthetic failed draft");
        app.send_current();
        wait_for_send();
        assert_eq!(app.state.borrow().outbox.len(), 1);
        assert_eq!(
            app.state.borrow().drafts["shared-key"],
            "Synthetic failed draft"
        );
        send_mode.set(2);
        app.editor.buffer().set_text("Synthetic uncertain draft");
        app.send_current();
        wait_for_send();
        assert_eq!(app.state.borrow().outbox.len(), 2);
        assert_eq!(
            app.state.borrow().outbox[1].status,
            super::OutboxStatus::OutcomeUnknown
        );
        assert_eq!(
            app.state.borrow().drafts["shared-key"],
            "Synthetic uncertain draft"
        );
        api_version.set(99);
        app.refresh_recent();
        wait_for_refresh();
        assert!(!app.backend_available.get());
        assert!(!app.state.borrow().map_ready);
        assert!(!app.send_button.is_sensitive());
        assert!(app.status.has_css_class("warning"));
        assert!(app.status.text().contains("API v2"));
        assert_eq!(
            app.state.borrow().drafts["shared-key"],
            "Synthetic uncertain draft"
        );
        api_version.set(2);
        app.refresh_recent();
        wait_for_refresh();
        assert!(app.backend_available.get());
        assert!(app.state.borrow().map_ready);
        assert!(!app.status.has_css_class("warning"));
        assert_eq!(
            app.state.borrow().drafts["shared-key"],
            "Synthetic uncertain draft"
        );
        bus.unregister_object(registration).unwrap();
        app.window.close();
    }

    #[test]
    fn refresh_rejects_phone_changes_and_disconnects() {
        let before = super::BlueFerryStatus {
            map: true,
            phone_profile_id: Some("A".into()),
            ..Default::default()
        };
        assert!(super::ensure_refresh_phone_unchanged(&before, &before).is_ok());
        for after in [
            super::BlueFerryStatus {
                phone_profile_id: Some("B".into()),
                ..before.clone()
            },
            super::BlueFerryStatus {
                phone_profile_id: None,
                ..before.clone()
            },
            super::BlueFerryStatus {
                map: false,
                ..before.clone()
            },
        ] {
            assert!(super::ensure_refresh_phone_unchanged(&before, &after).is_err());
        }
    }

    #[test]
    fn phone_switch_preserves_drafts_without_sharing_or_retrying_sends() {
        let mut state = super::State::default();
        state
            .drafts
            .insert("same-key".into(), "phone A draft".into());
        state.selected = Some("same-key".into());
        state.pending.push("same-key".into());
        state.map_ready = true;
        state.outbox.push(super::OutboxMessage {
            id: 1,
            key: "same-key".into(),
            body: "pending A".into(),
            timestamp: String::new(),
            baseline: 0,
            status: super::OutboxStatus::Sending,
        });
        let mut parked = std::collections::HashMap::new();
        super::switch_phone_state(&mut state, &mut parked, Some("A"), Some("B"));
        assert!(state.drafts.is_empty());
        assert!(state.outbox.is_empty());
        state
            .drafts
            .insert("same-key".into(), "phone B draft".into());
        super::switch_phone_state(&mut state, &mut parked, Some("B"), Some("A"));
        assert_eq!(state.drafts["same-key"], "phone A draft");
        assert_eq!(state.outbox[0].status, super::OutboxStatus::OutcomeUnknown);
        assert!(state.pending.is_empty());
        assert!(state.selected.is_none());
        assert!(!state.map_ready);
        assert_eq!(parked["B"].drafts["same-key"], "phone B draft");
    }

    #[test]
    fn unidentified_drafts_are_not_assigned_to_an_identified_phone() {
        let mut state = super::State::default();
        state.drafts.insert("key".into(), "unknown phone".into());
        let mut parked = std::collections::HashMap::new();
        super::switch_phone_state(&mut state, &mut parked, None, Some("A"));
        assert!(state.drafts.is_empty());
        assert!(parked.is_empty());
    }

    use super::*;

    #[test]
    fn backend_unavailable_errors_get_actionable_summary() {
        let error = "GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: no service";
        assert_eq!(
            backend_error_summary(error),
            "BlueFerry could not be reached. Check its connection in Phone Setup, then retry."
        );
    }

    #[test]
    fn other_backend_errors_remain_available_for_diagnosis() {
        let error = "The iPhone connection was lost";
        assert_eq!(backend_error_summary(error), error);
    }

    #[test]
    fn enter_sends_only_after_a_literal_newline_insert() {
        assert!(should_send_after_enter_insert(true, true, "\n"));
        assert!(!should_send_after_enter_insert(true, true, "composed text"));
        assert!(!should_send_after_enter_insert(true, false, "\n"));
        assert!(!should_send_after_enter_insert(false, true, "\n"));
    }

    #[test]
    fn enter_send_does_not_capture_modified_or_other_keys() {
        use gtk::gdk::{Key, ModifierType};
        assert!(is_plain_enter(Key::Return, ModifierType::empty()));
        assert!(is_plain_enter(Key::KP_Enter, ModifierType::empty()));
        assert!(!is_plain_enter(Key::Return, ModifierType::SHIFT_MASK));
        assert!(!is_plain_enter(Key::Return, ModifierType::CONTROL_MASK));
        assert!(!is_plain_enter(Key::a, ModifierType::empty()));
    }

    #[test]
    fn refresh_gate_coalesces_concurrent_refresh_requests() {
        let gate = RefreshGate::default();
        assert!(gate.request(), "the first refresh should start");
        assert!(!gate.request(), "parallel requests should be coalesced");
        assert!(!gate.request(), "only one follow-up should be queued");
        assert!(gate.finish(), "a coalesced request needs one follow-up");
        assert!(gate.request(), "the follow-up refresh should start");
        assert!(!gate.finish(), "no extra refresh should be queued");
    }

    #[test]
    fn refresh_gate_finishes_without_an_unrequested_follow_up() {
        let gate = RefreshGate::default();
        assert!(gate.request());
        assert!(!gate.finish());
        assert!(
            gate.request(),
            "a completed gate should allow a new refresh"
        );
    }

    #[test]
    fn recent_inbox_refresh_policy_catches_up_without_polling_every_status_tick() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let last = Some(now - Duration::from_secs(299));
        assert!(!recent_inbox_due(true, true, true, false, last, now));
        assert!(recent_inbox_due(
            true,
            true,
            true,
            false,
            Some(now - Duration::from_secs(300)),
            now
        ));
        assert!(recent_inbox_due(true, true, true, true, last, now));
        assert!(recent_inbox_due(true, false, true, false, last, now));
        assert!(recent_inbox_due(true, true, false, false, None, now));
        assert!(!recent_inbox_due(false, false, false, true, None, now));
    }

    #[test]
    fn normalizes_complete_phone_numbers_and_email_domains() {
        assert_eq!(
            normalize_recipient(" +1 (415) 555-2671 "),
            Some("+14155552671".to_string())
        );
        assert_eq!(
            normalize_recipient("415.555.2671"),
            Some("4155552671".to_string())
        );
        assert_eq!(
            normalize_recipient("Alice+work@EXAMPLE.COM"),
            Some("Alice+work@example.com".to_string())
        );
    }

    #[test]
    fn rejects_incomplete_or_malformed_recipients() {
        for invalid in [
            "",
            "+",
            "12",
            "123abc",
            "alice@",
            "@example.com",
            "alice..work@example.com",
            "alice@-example.com",
            "alice@example-.com",
            "alice@example..com",
            "alïce@example.com",
        ] {
            assert_eq!(normalize_recipient(invalid), None, "accepted {invalid:?}");
        }
    }

    #[test]
    fn outbox_waits_for_a_matching_outgoing_history_echo() {
        let mut state = State {
            threads: vec![Thread {
                key: "phone-thread".into(),
                messages: vec![Message {
                    body: "same text".into(),
                    outgoing: false,
                    ..Message::default()
                }],
                ..Thread::default()
            }],
            outbox: vec![OutboxMessage {
                id: 7,
                key: "phone-thread".into(),
                body: "  same text\r\n".into(),
                timestamp: "2026-10-22T12:00:00Z".into(),
                baseline: 0,
                status: OutboxStatus::Transferred,
            }],
            ..State::default()
        };

        reconcile_outbox(&mut state);
        assert_eq!(
            state.outbox.len(),
            1,
            "incoming text must not acknowledge a send"
        );

        state.threads[0].messages.push(Message {
            body: "same text".into(),
            outgoing: true,
            ..Message::default()
        });
        reconcile_outbox(&mut state);
        assert!(
            state.outbox.is_empty(),
            "normalized outgoing echo should acknowledge it"
        );
    }

    #[test]
    fn outbox_requires_an_echo_after_the_pre_send_baseline() {
        let prior = Message {
            body: "repeat".into(),
            outgoing: true,
            ..Message::default()
        };
        let mut state = State {
            threads: vec![Thread {
                key: "phone-thread".into(),
                messages: vec![prior],
                ..Thread::default()
            }],
            outbox: vec![OutboxMessage {
                id: 8,
                key: "phone-thread".into(),
                body: "repeat".into(),
                timestamp: "2026-10-22T12:00:00Z".into(),
                baseline: 1,
                status: OutboxStatus::OutcomeUnknown,
            }],
            ..State::default()
        };

        reconcile_outbox(&mut state);
        assert_eq!(
            state.outbox.len(),
            1,
            "an old matching message is not a new receipt"
        );

        state.threads[0].messages.push(Message {
            body: "repeat".into(),
            outgoing: true,
            ..Message::default()
        });
        reconcile_outbox(&mut state);
        assert!(state.outbox.is_empty());
    }
}
