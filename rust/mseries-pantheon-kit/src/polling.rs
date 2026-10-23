//! Explicit, UI-thread polling for GTK applications.
//!
//! Pollers start disabled. Apps can bind them to widget mapping so hidden pages
//! stop doing periodic work automatically.

use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

struct Inner {
    interval: Cell<Duration>,
    source: RefCell<Option<glib::SourceId>>,
    generation: Cell<u64>,
    enabled: Cell<bool>,
    mapped: Cell<bool>,
    bound_to_widget: Cell<bool>,
    callback: RefCell<Box<dyn FnMut()>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(source) = self.source.get_mut().take() {
            source.remove();
        }
    }
}

/// A controllable periodic callback attached to the GLib main loop.
///
/// A new poller is disabled. Call [`set_enabled`](Self::set_enabled) to start
/// and stop it, or [`bind_to_widget_mapped`](Self::bind_to_widget_mapped) to
/// run it only while a GTK widget is mapped. Callbacks must be short and
/// nonblocking; move expensive work to an async task or worker and update GTK
/// widgets on the main thread.
#[derive(Clone)]
pub struct Polling(Rc<Inner>);

impl Polling {
    /// Create a disabled poller. `interval` must be nonzero.
    pub fn new(interval: Duration, callback: impl FnMut() + 'static) -> Self {
        assert!(!interval.is_zero(), "polling interval must be nonzero");
        Self(Rc::new(Inner {
            interval: Cell::new(interval),
            source: RefCell::new(None),
            generation: Cell::new(0),
            enabled: Cell::new(false),
            mapped: Cell::new(true),
            bound_to_widget: Cell::new(false),
            callback: RefCell::new(Box::new(callback)),
        }))
    }

    /// Enable or disable periodic work. Enabling waits for the next interval;
    /// it does not trigger an immediate callback.
    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.set(enabled);
        reconcile(&self.0);
    }

    /// Whether periodic work is enabled by the app or user.
    pub fn is_enabled(&self) -> bool {
        self.0.enabled.get()
    }

    /// Whether a GLib timer is currently registered. A poller can be enabled
    /// but not running while its bound widget is unmapped.
    pub fn is_running(&self) -> bool {
        self.0.source.borrow().is_some()
    }

    /// Change the interval. An enabled poller restarts its timer at the new
    /// interval; a disabled poller remains disabled.
    pub fn set_interval(&self, interval: Duration) {
        assert!(!interval.is_zero(), "polling interval must be nonzero");
        if self.0.interval.replace(interval) != interval && self.is_running() {
            stop(&self.0);
            start(&self.0);
        }
    }

    /// Run one callback now without changing the enabled state.
    pub fn poll_now(&self) {
        (self.0.callback.borrow_mut())();
    }

    /// Enable polling while `widget` is mapped and disable it when unmapped.
    ///
    /// The signal handlers hold only a weak reference to this poller. Keep a
    /// `Polling` handle for as long as the widget should control it.
    pub fn bind_to_widget_mapped<W>(&self, widget: &W)
    where
        W: IsA<gtk::Widget> + Clone + 'static,
    {
        self.0.bound_to_widget.set(true);
        self.0.mapped.set(widget.is_mapped());
        reconcile(&self.0);

        let weak = Rc::downgrade(&self.0);
        widget.connect_map(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.mapped.set(true);
                reconcile(&inner);
            }
        });

        let weak = Rc::downgrade(&self.0);
        widget.connect_unmap(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.mapped.set(false);
                reconcile(&inner);
            }
        });
    }
}

fn reconcile(inner: &Rc<Inner>) {
    let should_run = inner.enabled.get() && (!inner.bound_to_widget.get() || inner.mapped.get());
    if should_run == inner.source.borrow().is_some() {
        return;
    }

    if should_run {
        start(inner);
    } else {
        stop(inner);
    }
}

fn stop(inner: &Rc<Inner>) {
    inner.generation.set(inner.generation.get().wrapping_add(1));
    if let Some(source) = inner.source.borrow_mut().take() {
        source.remove();
    }
}

fn start(inner: &Rc<Inner>) {
    inner.generation.set(inner.generation.get().wrapping_add(1));
    let generation = inner.generation.get();
    let weak = Rc::downgrade(inner);
    let source = glib::timeout_add_local(inner.interval.get(), move || {
        let Some(inner) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if inner.generation.get() != generation {
            return glib::ControlFlow::Break;
        }

        (inner.callback.borrow_mut())();
        if inner.generation.get() == generation {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    inner.source.replace(Some(source));
}
