//! Image Studio's visual language: one scoped stylesheet plus the few widget
//! recipes every page shares. Colors come from the elementary stylesheet's named
//! colors so the app follows the desktop accent and light/dark preference.

use gtk::prelude::*;

const CSS: &str = r#"
.mseries-image-studio { font-size: 1.05em; }

/* ---------- Window chrome ---------- */
.mseries-image-studio .is-title-icon { opacity: 0.7; }
.mseries-image-studio .is-title-text { font-weight: 700; }
.mseries-image-studio .is-title-sub { font-size: 0.82em; opacity: 0.6; }

/* ---------- Sidebar (source list) ---------- */
.mseries-image-studio .is-sidebar { background: alpha(@fg_color, 0.035); border-right: 1px solid alpha(@fg_color, 0.09); }
.mseries-image-studio .is-sidebar list.navigation-sidebar { background: transparent; padding: 0 8px; }
.mseries-image-studio .is-sidebar row.is-nav-row { border-radius: 8px; margin: 1px 0; min-height: 30px; }
.mseries-image-studio .is-sidebar row.is-nav-row image { opacity: 0.75; }
.mseries-image-studio .is-sidebar row.is-nav-row:selected { background: alpha(@accent_color, 0.22); }
.mseries-image-studio .is-sidebar row.is-nav-row:selected image { color: @accent_color; opacity: 1; }
.mseries-image-studio .is-sidebar row.is-nav-row:selected label { opacity: 1; font-weight: 700; }
.mseries-image-studio .is-sidebar row.is-nav-header { background: transparent; opacity: 1; min-height: 0; }
.mseries-image-studio .is-section-caption { font-size: 0.78em; font-weight: 700; letter-spacing: 0.06em; opacity: 0.55; }
.mseries-image-studio .is-sidebar button.is-quick { border-radius: 8px; padding: 6px 8px; }
.mseries-image-studio .is-sidebar button.is-quick:hover { background: alpha(@fg_color, 0.07); }

/* ---------- Cards, chips, dots ---------- */
.mseries-image-studio .is-card { background: alpha(@fg_color, 0.045); border: 1px solid alpha(@fg_color, 0.10); border-radius: 12px; }
.mseries-image-studio .is-card.is-raised { background: @base_color; box-shadow: 0 1px 2px alpha(black, 0.18), 0 6px 18px alpha(black, 0.12); }
.mseries-image-studio .is-card.is-selectable { transition: background 120ms ease, border-color 120ms ease; }
.mseries-image-studio .is-card.is-selectable:hover { background: alpha(@fg_color, 0.075); }
.mseries-image-studio .is-card.is-selectable.is-selected { border-color: @accent_color; background: alpha(@accent_color, 0.14); }
.mseries-image-studio .is-card-title { font-weight: 700; }
.mseries-image-studio .is-dim { opacity: 0.62; }
.mseries-image-studio .is-caption { font-size: 0.86em; opacity: 0.72; }
.mseries-image-studio .is-page-title { font-size: 1.7em; font-weight: 800; letter-spacing: -0.01em; }
.mseries-image-studio .is-page-sub { opacity: 0.68; }

.mseries-image-studio .is-chip { background: alpha(@fg_color, 0.08); border-radius: 999px; padding: 2px 10px; font-size: 0.84em; }
.mseries-image-studio .is-chip.is-accent { background: alpha(@accent_color, 0.24); font-weight: 600; }
.mseries-image-studio .is-chip.is-ok { background: alpha(@LIME_500, 0.30); font-weight: 600; }
.mseries-image-studio .is-chip.is-warn { background: alpha(@ORANGE_500, 0.26); font-weight: 600; }
.mseries-image-studio .is-chip.is-error { background: alpha(@STRAWBERRY_500, 0.28); font-weight: 600; }
.mseries-image-studio .is-chip image { opacity: 0.85; }
.mseries-image-studio .is-chip.is-ok image { color: @LIME_700; opacity: 1; }

.mseries-image-studio .is-dot { min-width: 9px; min-height: 9px; border-radius: 99px; background: alpha(@fg_color, 0.35); }
.mseries-image-studio .is-dot.is-ok { background: @LIME_500; box-shadow: 0 0 0 3px alpha(@LIME_500, 0.26); }
.mseries-image-studio .is-dot.is-warn { background: @ORANGE_300; box-shadow: 0 0 0 3px alpha(@ORANGE_300, 0.28); }
.mseries-image-studio .is-dot.is-busy { background: @accent_color; box-shadow: 0 0 0 3px alpha(@accent_color, 0.25); }

/* ---------- Device meter ---------- */
.mseries-image-studio levelbar.is-meter trough { min-height: 6px; border-radius: 99px; background: alpha(@fg_color, 0.12); border: none; padding: 0; }
.mseries-image-studio levelbar.is-meter block { min-height: 6px; border-radius: 99px; border: none; }
.mseries-image-studio levelbar.is-meter block.filled { background: @accent_color; }
.mseries-image-studio levelbar.is-meter block.low { background: @ORANGE_300; }
.mseries-image-studio levelbar.is-meter block.empty { background: transparent; }

/* ---------- Canvas ---------- */
.mseries-image-studio .is-stage-clip { border-radius: 16px; }
.mseries-image-studio .is-stage {
  background-color: mix(@bg_color, black, 0.30);
  background-image: radial-gradient(circle at center, alpha(@fg_color, 0.13) 0px, alpha(@fg_color, 0.13) 1px, transparent 1.6px);
  background-size: 22px 22px;
  border: 1px solid alpha(@fg_color, 0.10);
  border-radius: 16px;
}
.mseries-image-studio picture.is-stage-frame { border-radius: 10px; box-shadow: 0 2px 4px alpha(black, 0.35), 0 18px 48px alpha(black, 0.45); }
.mseries-image-studio .is-empty-icon { opacity: 0.32; }
.mseries-image-studio .is-empty-title { font-size: 1.25em; font-weight: 700; }

/* Glass HUD that sits over images: always dark so it reads on any picture. */
.mseries-image-studio .is-hud { background: alpha(#17181c, 0.78); color: #f4f4f6; border: 1px solid alpha(white, 0.13); border-radius: 999px; padding: 5px 12px; box-shadow: 0 4px 14px alpha(black, 0.35); }
.mseries-image-studio .is-hud label, .mseries-image-studio .is-hud image { color: #f4f4f6; }
.mseries-image-studio .is-hud .is-dim { opacity: 0.7; }
.mseries-image-studio .is-hud-rail { background: alpha(#17181c, 0.78); border: 1px solid alpha(white, 0.13); border-radius: 14px; padding: 6px; box-shadow: 0 4px 14px alpha(black, 0.35); }
.mseries-image-studio progressbar.is-stage-progress trough { min-height: 4px; min-width: 0; margin: 0; border: none; background: alpha(black, 0.28); border-radius: 0; }
.mseries-image-studio progressbar.is-stage-progress progress { min-height: 4px; min-width: 0; margin: 0; border: none; background: @accent_color; border-radius: 0; }

/* History filmstrip */
.mseries-image-studio button.is-film { padding: 0; border-radius: 9px; min-width: 52px; min-height: 52px; border: 2px solid transparent; background: alpha(white, 0.06); }
.mseries-image-studio button.is-film picture { border-radius: 7px; }
.mseries-image-studio button.is-film:hover { border-color: alpha(white, 0.5); }
.mseries-image-studio button.is-film.is-current { border-color: @accent_color; }

/* ---------- Prompt dock ---------- */
.mseries-image-studio .is-dock { background: @base_color; border: 1px solid alpha(@fg_color, 0.12); border-radius: 18px; box-shadow: 0 2px 4px alpha(black, 0.16), 0 14px 36px alpha(black, 0.26); }
.mseries-image-studio .is-dock textview, .mseries-image-studio .is-dock textview text { background: transparent; }
.mseries-image-studio .is-dock .is-prompt textview { font-size: 1.08em; }
.mseries-image-studio .is-dock-divider { background: alpha(@fg_color, 0.09); min-height: 1px; }
.mseries-image-studio button.is-generate { border-radius: 999px; padding: 8px 20px; font-weight: 700; min-height: 22px; }
.mseries-image-studio button.is-pill { border-radius: 999px; padding: 4px 12px; font-size: 0.92em; }
.mseries-image-studio dropdown.is-pill > button { border-radius: 999px; padding: 2px 8px; min-height: 26px; }

/* ---------- Inspector ---------- */
.mseries-image-studio .is-inspector { background: alpha(@fg_color, 0.035); border-left: 1px solid alpha(@fg_color, 0.09); }
.mseries-image-studio .is-inspector-section { background: alpha(@fg_color, 0.05); border: 1px solid alpha(@fg_color, 0.09); border-radius: 12px; }
.mseries-image-studio .is-inspector-section > button.is-section-head { background: transparent; border: none; box-shadow: none; padding: 9px 12px; border-radius: 12px; }
.mseries-image-studio .is-inspector-section > button.is-section-head:hover { background: alpha(@fg_color, 0.06); }
.mseries-image-studio .is-section-head image.is-chevron { opacity: 0.5; transition: transform 140ms ease; }
.mseries-image-studio .is-field-label { font-size: 0.88em; opacity: 0.75; }
.mseries-image-studio .is-inspector scale trough { min-height: 5px; }
.mseries-image-studio .is-inspector spinbutton { min-width: 0; }
.mseries-image-studio .is-ref-thumb { border-radius: 10px; border: 1px solid alpha(@fg_color, 0.14); background: alpha(@fg_color, 0.06); min-width: 64px; min-height: 64px; }

/* ---------- Library ---------- */
.mseries-image-studio flowbox.is-gallery { background: transparent; }
.mseries-image-studio flowbox.is-gallery > flowboxchild { padding: 6px; border-radius: 16px; background: transparent; }
.mseries-image-studio flowbox.is-gallery > flowboxchild:selected { background: alpha(@accent_color, 0.22); }
.mseries-image-studio .is-tile { border-radius: 12px; background: alpha(@fg_color, 0.06); box-shadow: 0 1px 3px alpha(black, 0.3); }
.mseries-image-studio .is-tile picture { border-radius: 12px; }
.mseries-image-studio .is-tile-caption { background: linear-gradient(to top, alpha(black, 0.72), alpha(black, 0)); border-radius: 0 0 12px 12px; padding: 22px 10px 8px; color: white; }
.mseries-image-studio .is-tile-caption label { color: white; }
.mseries-image-studio .is-detail { background: alpha(@fg_color, 0.035); border-left: 1px solid alpha(@fg_color, 0.09); }
.mseries-image-studio picture.is-detail-image { border-radius: 12px; box-shadow: 0 2px 4px alpha(black, 0.3), 0 12px 32px alpha(black, 0.3); }
.mseries-image-studio .is-kv-key { font-size: 0.84em; opacity: 0.6; }
.mseries-image-studio .is-kv-val { font-weight: 600; }

/* ---------- Lists / rows inside pages ---------- */
.mseries-image-studio list.is-recipes { background: transparent; }
.mseries-image-studio list.is-recipes > row { background: transparent; padding: 4px 0; }
.mseries-image-studio .is-row-card { background: alpha(@fg_color, 0.045); border: 1px solid alpha(@fg_color, 0.10); border-radius: 12px; padding: 12px; }
.mseries-image-studio .is-mono { font-family: monospace; font-size: 0.92em; }
.mseries-image-studio .is-code { background: alpha(@fg_color, 0.05); border: 1px solid alpha(@fg_color, 0.10); border-radius: 12px; }
.mseries-image-studio .is-code textview, .mseries-image-studio .is-code textview text { background: transparent; }
.mseries-image-studio .is-field { background: alpha(@fg_color, 0.045); border: 1px solid alpha(@fg_color, 0.10); border-radius: 10px; }
.mseries-image-studio .is-field textview, .mseries-image-studio .is-field textview text { background: transparent; }
.mseries-image-studio .is-stat-value { font-size: 1.5em; font-weight: 800; letter-spacing: -0.01em; }

.mseries-image-studio.pantheon-compact .is-stage { border-radius: 12px; }
"#;

pub fn install(window: &gtk::ApplicationWindow) {
    let css = gtk::CssProvider::new();
    css.load_from_string(CSS);
    let display = gtk::prelude::RootExt::display(window);
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Large page title with an optional subtitle, aligned to the page gutter.
pub fn page_header(title: &str, subtitle: &str) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["is-page-title"])
        .build();
    title.set_accessible_role(gtk::AccessibleRole::Heading);
    header.append(&title);
    if !subtitle.is_empty() {
        header.append(
            &gtk::Label::builder()
                .label(subtitle)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["is-page-sub"])
                .build(),
        );
    }
    header
}

/// A scrolling page body with the standard gutter. Returns (scroller, content box).
pub fn page_body() -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_top(22);
    content.set_margin_bottom(24);
    content.set_margin_start(26);
    content.set_margin_end(26);
    let clamp = gtk::Box::new(gtk::Orientation::Vertical, 0);
    clamp.set_halign(gtk::Align::Fill);
    clamp.append(&content);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    (scroller, content)
}

pub fn card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("is-card");
    card.set_margin_top(0);
    card
}

/// Card with its own padding, so callers just append content.
pub fn padded_card(padding: i32) -> gtk::Box {
    let card = card();
    card.set_margin_top(0);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 10);
    inner.set_margin_top(padding);
    inner.set_margin_bottom(padding);
    inner.set_margin_start(padding + 2);
    inner.set_margin_end(padding + 2);
    card.append(&inner);
    // Callers append to the inner box via `card_content`.
    card
}

/// The padded inner box of a `padded_card`.
pub fn card_content(card: &gtk::Box) -> gtk::Box {
    card.first_child()
        .and_downcast::<gtk::Box>()
        .expect("padded_card content")
}

pub fn chip(text: &str, tone: &str, icon: Option<&str>) -> gtk::Box {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    chip.add_css_class("is-chip");
    chip.set_halign(gtk::Align::Start);
    if !tone.is_empty() {
        chip.add_css_class(tone);
    }
    if let Some(icon) = icon {
        let image = gtk::Image::from_icon_name(icon);
        image.set_pixel_size(12);
        chip.append(&image);
    }
    chip.append(&gtk::Label::new(Some(text)));
    chip
}

pub fn dot(tone: &str) -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("is-dot");
    if !tone.is_empty() {
        dot.add_css_class(tone);
    }
    dot.set_valign(gtk::Align::Center);
    dot.set_halign(gtk::Align::Center);
    dot
}

/// Replace a status class on a widget (is-ok / is-warn / is-busy / "").
pub fn set_tone(widget: &impl IsA<gtk::Widget>, tone: &str) {
    for class in ["is-ok", "is-warn", "is-busy", "is-error"] {
        widget.remove_css_class(class);
    }
    if !tone.is_empty() {
        widget.add_css_class(tone);
    }
}

pub fn caption(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["is-caption"])
        .build()
}

pub fn section_caption(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text.to_uppercase())
        .xalign(0.0)
        .css_classes(["is-section-caption"])
        .build()
}

/// An icon-and-text button used for quiet actions.
pub fn icon_button(icon: &str, label: &str, classes: &[&str]) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.set_halign(gtk::Align::Center);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    for class in classes {
        button.add_css_class(class);
    }
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Label above its control, matching the inspector's rhythm.
pub fn field(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .css_classes(["is-field-label"])
            .build(),
    );
    row.append(control);
    row
}

/// Slider that shares the spin button's adjustment, with the number beside it.
pub fn slider_row(label: &str, spin: &gtk::SpinButton) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["is-field-label"])
        .build();
    spin.set_width_chars(5);
    spin.set_valign(gtk::Align::Center);
    top.append(&name);
    top.append(spin);
    let scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&spin.adjustment()));
    scale.set_draw_value(false);
    scale.set_digits(spin.digits() as i32);
    scale.set_hexpand(true);
    scale.update_property(&[gtk::accessible::Property::Label(label)]);
    row.append(&top);
    row.append(&scale);
    row
}

/// Collapsible inspector section: header button + revealer. Returns (section, body).
pub fn collapsible(title: &str, icon: &str, expanded: bool) -> (gtk::Box, gtk::Box) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
    section.add_css_class("is-inspector-section");
    let head = gtk::Button::new();
    head.add_css_class("is-section-head");
    head.add_css_class("flat");
    let head_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = gtk::Image::from_icon_name(icon);
    icon.add_css_class("is-dim");
    let name = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["is-card-title"])
        .build();
    let chevron = gtk::Image::from_icon_name(if expanded {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    chevron.add_css_class("is-chevron");
    head_row.append(&icon);
    head_row.append(&name);
    head_row.append(&chevron);
    head.set_child(Some(&head_row));
    head.update_property(&[gtk::accessible::Property::Label(title)]);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_bottom(12);
    let revealer = gtk::Revealer::builder()
        .child(&body)
        .reveal_child(expanded)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(160)
        .build();
    section.append(&head);
    section.append(&revealer);
    let revealer_for_click = revealer.clone();
    head.connect_clicked(move |_| {
        let open = !revealer_for_click.reveals_child();
        revealer_for_click.set_reveal_child(open);
        chevron.set_icon_name(Some(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    });
    (section, body)
}

/// Remove all children of a box-like container.
pub fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Lay chips out left to right, wrapping to a new row once `budget` characters are used.
/// GTK's FlowBox aligns children in columns, which stretches short chips; this keeps
/// each chip at its natural width.
pub fn chip_rows(items: &[(String, &str, Option<&str>)], budget: usize) -> gtk::Box {
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let mut row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let mut used = 0;
    for (text, tone, icon) in items {
        let cost = text.chars().count() + if icon.is_some() { 3 } else { 0 } + 2;
        if used > 0 && used + cost > budget {
            rows.append(&row);
            row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            used = 0;
        }
        row.append(&chip(text, tone, *icon));
        used += cost;
    }
    if used > 0 {
        rows.append(&row);
    }
    rows
}
