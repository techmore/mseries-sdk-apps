//! Candidate reusable GTK status anchor. Host placement and profile-specific
//! installation are deliberately external to this widget.
use crate::usage_control::UsageControl;
use gtk::prelude::*;
use msdk::usage::{Snapshot, SnapshotError};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

struct Inner {
    button: gtk::Button,
    popup: gtk::Popover,
    control: UsageControl,
    anchor_motion: gtk::EventControllerMotion,
    popup_motion: gtk::EventControllerMotion,
    pinned: Cell<bool>,
    changing_mode: Cell<bool>,
    open_timer: RefCell<Option<gtk::glib::SourceId>>,
    close_timer: RefCell<Option<gtk::glib::SourceId>>,
}
impl Inner {
    fn cancel(timer: &RefCell<Option<gtk::glib::SourceId>>) {
        if let Some(source) = timer.borrow_mut().take() {
            source.remove();
        }
    }
    fn show(&self, interactive: bool) {
        Self::cancel(&self.close_timer);
        if self.popup.is_mapped() {
            if interactive {
                // GTK hides a visible popover when autohide changes. Close and
                // reopen it while retaining the same control/read lifecycle.
                self.changing_mode.set(true);
                self.control.preserve_during_unmap(true);
                self.popup.popdown();
                self.popup.set_autohide(true);
                self.pinned.set(true);
                self.popup.popup();
                self.control.preserve_during_unmap(false);
                self.changing_mode.set(false);
                if !self.popup.is_mapped() {
                    self.closed();
                }
            }
            return;
        }
        self.pinned.set(interactive);
        self.popup.set_autohide(interactive);
        let _ = self.control.open();
        self.popup.popup();
    }
    fn schedule_open(this: &Rc<Self>) {
        Self::cancel(&this.close_timer);
        if this.open_timer.borrow().is_some() || this.popup.is_mapped() {
            return;
        }
        let weak = Rc::downgrade(this);
        let source = gtk::glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(inner) = weak.upgrade() {
                inner.open_timer.borrow_mut().take();
                inner.show(false);
            }
        });
        *this.open_timer.borrow_mut() = Some(source);
    }
    fn schedule_close(this: &Rc<Self>) {
        Self::cancel(&this.open_timer);
        Self::cancel(&this.close_timer);
        if this.pinned.get() {
            return;
        }
        let weak = Rc::downgrade(this);
        let source = gtk::glib::timeout_add_local(Duration::from_millis(450), move || {
            let Some(inner) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            // Crossing events can report a leave while the device is still on
            // the popover's native surface. Check the surface itself and keep
            // this UI-only timer alive until the pointer actually leaves.
            let on_popup_surface = inner.popup.surface().is_some_and(|surface| {
                inner
                    .popup
                    .display()
                    .default_seat()
                    .and_then(|seat| seat.pointer())
                    .is_some_and(|pointer| {
                        pointer.surface_at_position().0.as_ref() == Some(&surface)
                    })
            });
            if inner.anchor_motion.contains_pointer()
                || inner.popup_motion.contains_pointer()
                || on_popup_surface
            {
                return gtk::glib::ControlFlow::Continue;
            }
            inner.close_timer.borrow_mut().take();
            inner.popup.popdown();
            gtk::glib::ControlFlow::Break
        });
        *this.close_timer.borrow_mut() = Some(source);
    }
    fn closed(&self) {
        if std::env::var_os("MSDK_ANCHOR_TRACE").is_some() {
            eprintln!("anchor trace: popup closed");
        }
        if self.changing_mode.get() {
            return;
        }
        self.pinned.set(false);
        Self::cancel(&self.open_timer);
        Self::cancel(&self.close_timer);
        self.control.close();
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        Self::cancel(&self.open_timer);
        Self::cancel(&self.close_timer);
        self.control.close();
        if self.popup.parent().is_some() {
            self.popup.unparent();
        }
    }
}

/// Host-owned status anchor. The profile must place `button()` in its actual
/// status area and call neither open nor refresh while it is closed. Dropping
/// this object releases timers, popup and owned read; no background polling.
pub struct UsageAnchor {
    inner: Rc<Inner>,
}
impl UsageAnchor {
    pub fn new(
        initial: &Snapshot,
        now: i64,
        executable: PathBuf,
        max_age_seconds: u64,
        position: gtk::PositionType,
    ) -> Result<Self, SnapshotError> {
        let control = UsageControl::new(initial, now, executable, max_age_seconds)?;
        let button = gtk::Button::with_label("Usage");
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Usage status"));
        let popup = gtk::Popover::builder()
            .position(position)
            .autohide(false)
            .has_arrow(false)
            .child(control.widget())
            .build();
        popup.set_parent(&button);
        let inner = Rc::new(Inner {
            button,
            popup,
            control,
            anchor_motion: gtk::EventControllerMotion::new(),
            popup_motion: gtk::EventControllerMotion::new(),
            pinned: Cell::new(false),
            changing_mode: Cell::new(false),
            open_timer: RefCell::new(None),
            close_timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        inner.button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                Inner::cancel(&inner.open_timer);
                if inner.popup.is_mapped() && inner.pinned.get() {
                    inner.popup.popdown();
                } else {
                    inner.show(true);
                }
            }
        });
        let motion = inner.anchor_motion.clone();
        let weak = Rc::downgrade(&inner);
        motion.connect_enter(move |_, _, _| {
            if std::env::var_os("MSDK_ANCHOR_TRACE").is_some() {
                eprintln!("anchor trace: anchor enter");
            }
            if let Some(inner) = weak.upgrade() {
                Inner::schedule_open(&inner);
            }
        });
        let weak = Rc::downgrade(&inner);
        motion.connect_leave(move |_| {
            if std::env::var_os("MSDK_ANCHOR_TRACE").is_some() {
                eprintln!("anchor trace: anchor leave");
            }
            if let Some(inner) = weak.upgrade() {
                Inner::schedule_close(&inner);
            }
        });
        inner.button.add_controller(motion);
        let motion = inner.popup_motion.clone();
        let weak = Rc::downgrade(&inner);
        motion.connect_enter(move |_, _, _| {
            if std::env::var_os("MSDK_ANCHOR_TRACE").is_some() {
                eprintln!("anchor trace: popup enter");
            }
            if let Some(inner) = weak.upgrade() {
                Inner::cancel(&inner.close_timer);
            }
        });
        let weak = Rc::downgrade(&inner);
        motion.connect_leave(move |_| {
            if std::env::var_os("MSDK_ANCHOR_TRACE").is_some() {
                eprintln!("anchor trace: popup leave");
            }
            if let Some(inner) = weak.upgrade() {
                Inner::schedule_close(&inner);
            }
        });
        inner.popup.add_controller(motion);
        let weak = Rc::downgrade(&inner);
        inner.popup.connect_closed(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.closed();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(inner) = weak.upgrade() {
                    inner.popup.popdown();
                    inner.button.grab_focus();
                }
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        inner.popup.add_controller(keys);
        let weak = Rc::downgrade(&inner);
        inner.button.connect_unmap(move |_| {
            if let Some(inner) = weak.upgrade() {
                Inner::cancel(&inner.open_timer);
                Inner::cancel(&inner.close_timer);
                inner.popup.popdown();
                inner.closed();
            }
        });
        Ok(Self { inner })
    }
    pub fn button(&self) -> &gtk::Button {
        &self.inner.button
    }
    /// The host owns placement; exposing the popup also permits host-specific
    /// positioning and isolated interaction inspection.
    pub fn popover(&self) -> &gtk::Popover {
        &self.inner.popup
    }
}
