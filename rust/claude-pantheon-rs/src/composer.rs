//! The composer: a text view that grows to eight lines (Enter sends, Shift+Enter breaks the line),
//! attachment chips from paste or drag-and-drop (uploaded through the server's attachment RPC), an
//! `@` file picker over the thread's project (fuzzy, served by `projects.searchEntries`), and a draft
//! kept per thread across restarts in `~/.cache/claude-pantheon-rs/drafts.json`.

use crate::chat::label;
use crate::t3::attach::{self, Attachment};
use gtk::{gdk, gio, glib, pango, prelude::*};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

const MAX_LINES: i32 = 8;

/// Bytes and a name for a pasted or dropped attachment.
pub type AttachHook = Rc<dyn Fn(Vec<u8>, String)>;
/// Project-relative paths answering a picker query.
pub type SearchReply = Rc<dyn Fn(Vec<String>)>;
pub type SearchHook = Rc<dyn Fn(String, SearchReply)>;

/// Called with true when a recording starts from the mic button and false when it stops.
pub type VoiceHook = Rc<dyn Fn(bool)>;

pub struct Composer {
    pub root: gtk::Box,
    pub view: gtk::TextView,
    scroll: gtk::ScrolledWindow,
    chips: gtk::FlowBox,
    attachments: RefCell<Vec<Attachment>>,
    picker: gtk::Popover,
    picker_entry: gtk::SearchEntry,
    picker_list: gtk::ListBox,
    picker_paths: RefCell<Vec<String>>,
    picker_at: Cell<i32>,
    on_send: RefCell<Option<Rc<dyn Fn()>>>,
    on_attach: RefCell<Option<AttachHook>>,
    on_search: RefCell<Option<SearchHook>>,
    on_voice: RefCell<Option<VoiceHook>>,
    pub voice_btn: gtk::ToggleButton,
    draft_key: RefCell<Option<String>>,
    draft_timer: RefCell<Option<glib::SourceId>>,
    drafts: RefCell<HashMap<String, String>>,
    status: gtk::Label,
    me: RefCell<Weak<Composer>>,
}

fn drafts_path() -> PathBuf {
    crate::index::cache_dir().join("drafts.json")
}

impl Composer {
    pub fn new() -> Rc<Composer> {
        let view = gtk::TextView::builder()
            .accessible_role(gtk::AccessibleRole::TextBox)
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(10)
            .right_margin(10)
            .build();
        view.add_css_class("composer-entry");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .hexpand(true)
            .build();
        scroll.add_css_class("frame");
        let chips = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(8)
            .row_spacing(4)
            .column_spacing(6)
            .build();
        chips.add_css_class("chips");
        chips.set_visible(false);
        let status = label("", &["caption", "dim-label"]);
        status.set_visible(false);
        status.set_hexpand(true);
        // Push-to-talk: press to record, press again to stop; the transcript lands at the cursor.
        let voice_btn = gtk::ToggleButton::new();
        voice_btn.set_icon_name("audio-input-microphone-symbolic");
        voice_btn.add_css_class("flat");
        voice_btn.add_css_class("circular");
        voice_btn.add_css_class("voice-btn");
        voice_btn.set_tooltip_text(Some("Voice input: press to record, press again to stop (Ctrl+Shift+V). The words go into the draft; nothing is sent."));
        voice_btn.update_property(&[gtk::accessible::Property::Label("Voice input")]);
        voice_btn.set_valign(gtk::Align::Center);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tools.append(&voice_btn);
        tools.append(&status);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.append(&chips);
        root.append(&scroll);
        root.append(&tools);

        // The @ picker: a popover anchored to the text view.
        let picker_entry = gtk::SearchEntry::builder().placeholder_text("File in this project…").build();
        picker_entry.update_property(&[gtk::accessible::Property::Label("File to attach")]);
        let picker_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).build();
        let picker_scroll = gtk::ScrolledWindow::builder()
            .child(&picker_list)
            .min_content_height(220)
            .max_content_height(320)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let picker_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        picker_box.set_width_request(420);
        picker_box.append(&picker_entry);
        picker_box.append(&picker_scroll);
        let picker = gtk::Popover::builder()
            .child(&picker_box)
            .autohide(true)
            .has_arrow(false)
            .position(gtk::PositionType::Top)
            .build();
        picker.set_parent(&view);

        let c = Rc::new(Composer {
            root,
            view,
            scroll,
            chips,
            attachments: RefCell::new(Vec::new()),
            picker,
            picker_entry,
            picker_list,
            picker_paths: RefCell::new(Vec::new()),
            picker_at: Cell::new(-1),
            on_send: RefCell::new(None),
            on_attach: RefCell::new(None),
            on_search: RefCell::new(None),
            on_voice: RefCell::new(None),
            voice_btn,
            draft_key: RefCell::new(None),
            draft_timer: RefCell::new(None),
            drafts: RefCell::new(
                std::fs::read(drafts_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(),
            ),
            status,
            me: RefCell::new(Weak::new()),
        });
        *c.me.borrow_mut() = Rc::downgrade(&c);
        c.grow_to_lines();
        let me = c.clone();
        c.voice_btn.connect_toggled(move |b| {
            let cb = me.on_voice.borrow().clone();
            if let Some(cb) = cb {
                cb(b.is_active());
            }
        });

        // Enter sends, Shift+Enter breaks the line, @ opens the picker, Escape closes it.
        let keys = gtk::EventControllerKey::new();
        let me = c.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter if !shift && !ctrl => {
                    if let Some(f) = me.on_send.borrow().as_ref() {
                        f();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::at => {
                    // Insert the @ and open the picker at it.
                    let buffer = me.view.buffer();
                    buffer.insert_at_cursor("@");
                    me.picker_at.set(buffer.cursor_position() - 1);
                    me.open_picker();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        c.view.add_controller(keys);

        // Paste: images and files from the clipboard become chips; text pastes as usual.
        let me = c.clone();
        c.view.connect_paste_clipboard(move |view| {
            let clipboard = view.clipboard();
            let formats = clipboard.formats();
            if formats.contains_type(gdk::Texture::static_type()) {
                let me = me.clone();
                clipboard.read_texture_async(gio::Cancellable::NONE, move |r| {
                    if let Ok(Some(texture)) = r {
                        let bytes = texture.save_to_png_bytes();
                        me.attach_bytes(
                            bytes.to_vec(),
                            format!(
                                "pasted-{}.png",
                                glib::DateTime::now_local()
                                    .and_then(|d| d.format("%H%M%S"))
                                    .map(|s| s.to_string())
                                    .unwrap_or_default()
                            ),
                        );
                    }
                });
                view.stop_signal_emission_by_name("paste-clipboard");
            } else if formats.contains_type(gdk::FileList::static_type()) {
                let me = me.clone();
                clipboard.read_value_async(
                    gdk::FileList::static_type(),
                    glib::Priority::DEFAULT,
                    gio::Cancellable::NONE,
                    move |r| {
                        if let Ok(v) = r
                            && let Ok(list) = v.get::<gdk::FileList>()
                        {
                            for f in list.files() {
                                me.attach_file(f);
                            }
                        }
                    },
                );
                view.stop_signal_emission_by_name("paste-clipboard");
            }
        });

        // Drag and drop of files and images.
        let drop = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        drop.set_types(&[gdk::FileList::static_type(), gdk::Texture::static_type()]);
        let me = c.clone();
        drop.connect_drop(move |_, value, _, _| {
            if let Ok(list) = value.get::<gdk::FileList>() {
                for f in list.files() {
                    me.attach_file(f);
                }
                true
            } else if let Ok(texture) = value.get::<gdk::Texture>() {
                me.attach_bytes(texture.save_to_png_bytes().to_vec(), "dropped.png".into());
                true
            } else {
                false
            }
        });
        c.root.add_controller(drop);

        // Draft: save a second after the last change.
        let me = c.clone();
        c.view.buffer().connect_changed(move |_| me.schedule_draft_save());

        // Picker wiring.
        let me = c.clone();
        c.picker_entry.connect_search_changed(move |e| me.search_picker(e.text().to_string()));
        let me = c.clone();
        c.picker_entry.connect_activate(move |_| me.pick_selected());
        let me = c.clone();
        c.picker_list.connect_row_activated(move |_, row| me.pick(row.index() as usize));
        let me = c.clone();
        c.picker.connect_closed(move |_| {
            me.view.grab_focus();
        });
        c
    }

    pub fn on_send(&self, f: Rc<dyn Fn()>) {
        *self.on_send.borrow_mut() = Some(f);
    }

    /// Called with the bytes and a name for every pasted or dropped attachment; the app uploads it
    /// and calls `add_attachment` when done.
    pub fn on_attach(&self, f: AttachHook) {
        *self.on_attach.borrow_mut() = Some(f);
    }

    /// Called with the picker query; the app answers through the callback with project-relative paths.
    pub fn on_search(&self, f: SearchHook) {
        *self.on_search.borrow_mut() = Some(f);
    }

    /// Eight lines of the view's font, then the view scrolls.
    fn grow_to_lines(&self) {
        let context = self.view.pango_context();
        let metrics = context.metrics(None, None);
        let line = (metrics.ascent() + metrics.descent()) as f64 / pango::SCALE as f64;
        let height = (line * MAX_LINES as f64 + 16.0).ceil() as i32;
        self.scroll.set_max_content_height(height.max(48));
    }

    pub fn text(&self) -> String {
        let b = self.view.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).to_string()
    }

    pub fn set_text(&self, text: &str) {
        self.view.buffer().set_text(text);
    }

    pub fn clear(&self) {
        self.set_text("");
        self.attachments.borrow_mut().clear();
        self.render_chips();
        if let Some(key) = self.draft_key.borrow().clone() {
            self.drafts.borrow_mut().remove(&key);
            self.save_drafts();
        }
    }

    pub fn attachments_json(&self) -> Vec<Value> {
        self.attachments.borrow().iter().map(Attachment::to_json).collect()
    }

    pub fn attachment_count(&self) -> usize {
        self.attachments.borrow().len()
    }

    /// Called with true when the mic button starts a recording and false when it stops.
    pub fn on_voice(&self, f: VoiceHook) {
        *self.on_voice.borrow_mut() = Some(f);
    }

    /// Reflects a recording that started or stopped elsewhere (the shortcut) without re-firing the hook.
    pub fn set_voice_active(&self, active: bool) {
        if self.voice_btn.is_active() != active {
            let hook = self.on_voice.borrow_mut().take();
            self.voice_btn.set_active(active);
            *self.on_voice.borrow_mut() = hook;
        }
        if active {
            self.voice_btn.add_css_class("recording");
        } else {
            self.voice_btn.remove_css_class("recording");
        }
    }

    /// The draft the composer is editing (a thread id, or None for a new thread).
    pub fn draft_key(&self) -> Option<String> {
        self.draft_key.borrow().clone()
    }

    /// Inserts text at the cursor with a space on either side where needed; never sends.
    pub fn insert_at_cursor(&self, text: &str) {
        let buffer = self.view.buffer();
        let mut iter = buffer.iter_at_mark(&buffer.get_insert());
        let before = buffer.text(&buffer.start_iter(), &iter, false).to_string();
        let after = buffer.text(&iter, &buffer.end_iter(), false).to_string();
        let mut piece = String::new();
        if !before.is_empty() && !before.ends_with(char::is_whitespace) {
            piece.push(' ');
        }
        piece.push_str(text);
        if !after.is_empty() && !after.starts_with(char::is_whitespace) {
            piece.push(' ');
        }
        buffer.insert(&mut iter, &piece);
        self.view.grab_focus();
        self.grow_to_lines();
    }

    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
        self.status.set_visible(!text.is_empty());
    }

    // ---- attachments ---------------------------------------------------------------------------

    fn attach_file(&self, file: gio::File) {
        let Some(path) = file.path() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        match std::fs::read(&path) {
            Ok(bytes) => self.attach_bytes(bytes, name),
            Err(e) => self.set_status(&format!("Could not read {name}: {e}")),
        }
    }

    fn attach_bytes(&self, bytes: Vec<u8>, name: String) {
        if self.attachments.borrow().len() >= attach::MAX_ATTACHMENTS {
            self.set_status(&format!("At most {} attachments per message.", attach::MAX_ATTACHMENTS));
            return;
        }
        self.set_status(&format!("Uploading {name}…"));
        if let Some(f) = self.on_attach.borrow().as_ref() {
            f(bytes, name);
        }
    }

    pub fn add_attachment(&self, a: Attachment) {
        self.set_status("");
        self.attachments.borrow_mut().push(a);
        self.render_chips();
    }

    fn render_chips(&self) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        let list = self.attachments.borrow();
        self.chips.set_visible(!list.is_empty());
        for (i, a) in list.iter().enumerate() {
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            chip.add_css_class("chip");
            chip.append(&gtk::Image::from_icon_name(if a.is_image {
                "image-x-generic-symbolic"
            } else {
                "text-x-generic-symbolic"
            }));
            let text = label(&format!("{} · {}", a.name, crate::look::size_text(a.size)), &["caption"]);
            chip.append(&text);
            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.update_property(&[gtk::accessible::Property::Label("Remove attachment")]);
            remove.set_tooltip_text(Some("Remove attachment"));
            remove.add_css_class("flat");
            remove.add_css_class("circular");
            remove.set_tooltip_text(Some("Remove attachment"));
            chip.append(&remove);
            self.chips.insert(&chip, -1);
            let idx = i;
            let me = self.me.borrow().clone();
            remove.connect_clicked(move |_| {
                let Some(me) = me.upgrade() else { return };
                if idx < me.attachments.borrow().len() {
                    me.attachments.borrow_mut().remove(idx);
                }
                me.render_chips();
            });
        }
    }

    // ---- @ picker --------------------------------------------------------------------------------

    fn open_picker(&self) {
        self.picker_entry.set_text("");
        self.search_picker(String::new());
        self.picker.popup();
        self.picker_entry.grab_focus();
    }

    fn search_picker(&self, query: String) {
        let Some(f) = self.on_search.borrow().clone() else { return };
        let list = self.picker_list.clone();
        let me = self.me.borrow().clone();
        f(
            query,
            Rc::new(move |results: Vec<String>| {
                let Some(me) = me.upgrade() else { return };
                while let Some(row) = list.row_at_index(0) {
                    list.remove(&row);
                }
                for p in &results {
                    let l = label(p, &[]);
                    l.set_ellipsize(pango::EllipsizeMode::Start);
                    l.set_margin_top(4);
                    l.set_margin_bottom(4);
                    l.set_margin_start(6);
                    let row = gtk::ListBoxRow::new();
                    row.set_child(Some(&l));
                    list.append(&row);
                }
                *me.picker_paths.borrow_mut() = results;
                if let Some(first) = list.row_at_index(0) {
                    list.select_row(Some(&first));
                }
            }),
        );
    }

    fn pick_selected(&self) {
        let i = self.picker_list.selected_row().map(|r| r.index()).unwrap_or(0).max(0) as usize;
        self.pick(i);
    }

    fn pick(&self, index: usize) {
        let Some(path) = self.picker_paths.borrow().get(index).cloned() else { return };
        self.picker.popdown();
        let buffer = self.view.buffer();
        // Replace the "@" typed to open the picker with "@path ".
        let at = self.picker_at.get();
        if at >= 0 {
            let mut start = buffer.iter_at_offset(at);
            let mut end = buffer.iter_at_offset(at + 1);
            if start.char() == '@' {
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &format!("@{path} "));
            } else {
                buffer.insert_at_cursor(&format!("@{path} "));
            }
        } else {
            buffer.insert_at_cursor(&format!("@{path} "));
        }
        self.picker_at.set(-1);
        self.view.grab_focus();
    }

    // ---- drafts ----------------------------------------------------------------------------------

    /// Switches the draft slot: saves the current text under the old key, restores the new one.
    pub fn set_draft_key(&self, key: Option<String>) {
        self.flush_draft();
        *self.draft_key.borrow_mut() = key.clone();
        let restored = key.and_then(|k| self.drafts.borrow().get(&k).cloned()).unwrap_or_default();
        if self.text() != restored {
            self.set_text(&restored);
        }
    }

    fn schedule_draft_save(&self) {
        if let Some(id) = self.draft_timer.borrow_mut().take() {
            id.remove();
        }
        let me = self.me.borrow().clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
            let Some(me) = me.upgrade() else { return };
            *me.draft_timer.borrow_mut() = None;
            me.flush_draft();
        });
        *self.draft_timer.borrow_mut() = Some(id);
    }

    fn flush_draft(&self) {
        let Some(key) = self.draft_key.borrow().clone() else { return };
        let text = self.text();
        let mut drafts = self.drafts.borrow_mut();
        let changed = if text.trim().is_empty() {
            drafts.remove(&key).is_some()
        } else {
            drafts.insert(key, text.clone()) != Some(text)
        };
        drop(drafts);
        if changed {
            self.save_drafts();
        }
    }

    fn save_drafts(&self) {
        let path = drafts_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, serde_json::to_vec(&*self.drafts.borrow()).unwrap_or_default());
    }

    pub fn draft_for(&self, key: &str) -> Option<String> {
        self.drafts.borrow().get(key).cloned()
    }
}
