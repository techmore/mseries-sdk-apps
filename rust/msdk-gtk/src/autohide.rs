//! Auto-hiding sidebar: an OPTIONAL feature. A `Split(autohide: true)` shows only its content; the sidebar slides in over the
//! content after the pointer rests on the left edge for 0.5 s (or keyboard focus enters it) and slides away shortly after the
//! pointer leaves. Settings are not part of it: they live in the title bar's options menu.
//!
//! TO REMOVE THE FEATURE: delete this file; delete `pub mod autohide;` in lib.rs; delete the `autohide` branch of the `Split`
//! arm in render.rs; delete `opt("autohide", ...)` from the `Split` props in msdk/src/vocab.rs; delete `features` from
//! msdk/data/tokens.json and `feature()` from msdk/src/tokens.rs. To switch it off without removing code, set
//! `features.sidebar_autohide` to false in tokens.json: `autohide: true` is then ignored and the normal split is used.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

pub const SHOW_DELAY_MS: u64 = 500;
pub const HIDE_DELAY_MS: u64 = 350;

/// Runs `f` after `delay` unless `generation` changed meanwhile (arming again or cancelling bumps it).
fn arm(generation: &Rc<Cell<u64>>, delay: u64, f: impl FnOnce() + 'static) {
    let now = generation.get() + 1;
    generation.set(now);
    let g = generation.clone();
    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        if g.get() == now {
            f();
        }
    });
}

fn cancel(generation: &Rc<Cell<u64>>) {
    generation.set(generation.get() + 1);
}

/// Content fills the width; the sidebar is an overlay that reveals on hover.
pub fn build(sidebar: &gtk::Box, content: &gtk::Box) -> gtk::Widget {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(content));

    sidebar.add_css_class("msdk-floating-sidebar");
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideRight)
        .transition_duration(180)
        .reveal_child(false)
        .halign(gtk::Align::Start)
        .vexpand(true)
        .build();
    revealer.set_child(Some(sidebar));
    overlay.add_overlay(&revealer);

    // A thin strip on the left edge senses the pointer; it is narrow enough not to get in the way of the content.
    let strip = gtk::Box::new(gtk::Orientation::Vertical, 0);
    strip.set_size_request(8, -1);
    strip.set_halign(gtk::Align::Start);
    strip.set_vexpand(true);
    strip.add_css_class("msdk-hover-edge");
    overlay.add_overlay(&strip);

    let generation = Rc::new(Cell::new(0u64));

    let enter_strip = gtk::EventControllerMotion::new();
    let (g, r) = (generation.clone(), revealer.clone());
    enter_strip.connect_enter(move |_, _, _| {
        let r = r.clone();
        arm(&g, SHOW_DELAY_MS, move || r.set_reveal_child(true));
    });
    let g = generation.clone();
    enter_strip.connect_leave(move |_| cancel(&g));
    strip.add_controller(enter_strip);

    // Leaving the revealed sidebar hides it again after a short grace period; coming back cancels that.
    let over_sidebar = gtk::EventControllerMotion::new();
    let g = generation.clone();
    over_sidebar.connect_enter(move |_, _, _| cancel(&g));
    let (g, r) = (generation.clone(), revealer.clone());
    over_sidebar.connect_leave(move |_| {
        let r = r.clone();
        arm(&g, HIDE_DELAY_MS, move || r.set_reveal_child(false));
    });
    revealer.add_controller(over_sidebar);

    // Keyboard users are never locked out: focus inside the sidebar reveals it and keeps it open.
    let focus = gtk::EventControllerFocus::new();
    let (g, r) = (generation.clone(), revealer.clone());
    focus.connect_enter(move |_| {
        cancel(&g);
        r.set_reveal_child(true);
    });
    let (g, r) = (generation.clone(), revealer.clone());
    focus.connect_leave(move |_| {
        let r = r.clone();
        arm(&g, HIDE_DELAY_MS, move || r.set_reveal_child(false));
    });
    sidebar.add_controller(focus);

    overlay.upcast()
}
