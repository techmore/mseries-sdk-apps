use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// A logical window size class. GTK handles conversion to physical pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutClass {
    Compact,
    Regular,
}

/// Return a length expressed in multiples of the widget's current text line height.
///
/// This text-relative layout unit follows the effective font metrics. It is
/// distinct from CSS `em`; do not multiply it by `scale_factor`, because GTK
/// performs device-pixel scaling itself.
pub fn text_unit<W: IsA<gtk::Widget>>(widget: &W, multiplier: f64) -> i32 {
    let metrics = widget.pango_context().metrics(None, None);
    let line = metrics.ascent().saturating_add(metrics.descent());
    (f64::from(line) / f64::from(gtk::pango::SCALE) * multiplier)
        .round()
        .max(0.0) as i32
}

/// Set a box's spacing in current text-line units and track font/DPI changes.
pub fn set_box_spacing_text_units(container: &gtk::Box, multiplier: f64) {
    let weak = container.downgrade();
    let update: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(container) = weak.upgrade() {
            container.set_spacing(text_unit(&container, multiplier));
        }
    });
    update();
    watch_font_changes(update);
}

/// Set per-edge margins in multiples of the current text line height and track live
/// font/DPI changes. This is intended for a small number of persistent layout
/// containers, not for rapidly-created rows; use the CSS spacing classes for
/// repeated row content.
pub fn set_margins_text_units<W: IsA<gtk::Widget> + Clone + 'static>(
    widget: &W,
    top: f64,
    end: f64,
    bottom: f64,
    start: f64,
) {
    let widget: gtk::Widget = widget.clone().upcast();
    let weak = widget.downgrade();
    let update: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(widget) = weak.upgrade() {
            widget.set_margin_top(text_unit(&widget, top));
            widget.set_margin_end(text_unit(&widget, end));
            widget.set_margin_bottom(text_unit(&widget, bottom));
            widget.set_margin_start(text_unit(&widget, start));
        }
    });
    update();
    watch_font_changes(update);
}

/// Reclassify a widget as its allocated width or desktop text scale changes.
/// The callback fires only when the class changes; a matching CSS class is
/// maintained on the widget for style-only adaptations.
pub fn watch_responsive_width<W, F>(widget: &W, breakpoint_text_units: f64, on_change: F)
where
    W: IsA<gtk::Widget> + Clone + 'static,
    F: Fn(LayoutClass) + 'static,
{
    let widget: gtk::Widget = widget.clone().upcast();
    let weak = widget.downgrade();
    let previous = Rc::new(Cell::new(None));
    let on_change = Rc::new(on_change);
    let update: Rc<dyn Fn()> = Rc::new(move || {
        let Some(widget) = weak.upgrade() else { return };
        let width = widget.width();
        if width <= 0 {
            return;
        }

        let class = if width < text_unit(&widget, breakpoint_text_units) {
            LayoutClass::Compact
        } else {
            LayoutClass::Regular
        };
        if previous.get() == Some(class) {
            return;
        }
        previous.set(Some(class));
        match class {
            LayoutClass::Compact => widget.add_css_class("pantheon-compact"),
            LayoutClass::Regular => widget.remove_css_class("pantheon-compact"),
        }
        on_change(class);
    });

    let update_map = update.clone();
    widget.connect_map(move |_| {
        let update = update_map.clone();
        gtk::glib::idle_add_local_once(move || update());
    });
    let update_realize = update.clone();
    widget.connect_realize(move |widget| {
        let Some(surface) = widget.native().and_then(|native| native.surface()) else {
            return;
        };
        let update_surface_layout = update_realize.clone();
        surface.connect_layout(move |_, _, _| {
            let update = update_surface_layout.clone();
            gtk::glib::idle_add_local_once(move || update());
        });
    });
    watch_font_changes(update);
}

/// Set the accessible name of a control whose icon or visual treatment has no
/// visible text label.
pub fn set_accessible_label<W: IsA<gtk::Widget> + IsA<gtk::Accessible>>(widget: &W, label: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(label)]);
}

/// Install the SDK's shared geometry classes on a display. Colors and control
/// appearance continue to come from the user's GTK/Granite theme.
pub fn install_css(display: &gtk::gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../data/pantheon-kit.css"));
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn watch_font_changes(update: Rc<dyn Fn()>) {
    if let Some(settings) = gtk::Settings::default() {
        for property in ["gtk-font-name", "gtk-xft-dpi"] {
            let update = update.clone();
            settings.connect_notify_local(Some(property), move |_, _| update());
        }
    }
}
