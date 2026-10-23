//! Builds GTK4 widgets from an `msdk` component tree. Each built node returns refreshers that re-read the store.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use serde_json::{Value, json};

use crate::logic::{Store, display, interpolate, truthy};

pub type Refresh = Rc<dyn Fn(&Store)>;

/// What the widgets need from the host: send messages, read and write state, ask for confirmation.
pub trait Dispatcher {
    fn send(&self, msg: &str, payload: Option<Value>);
    fn get(&self, key: &str) -> Option<Value>;
    fn set(&self, key: &str, v: Value);
    fn confirm(&self, text: &str, then: Box<dyn FnOnce()>);
}

pub type D = Rc<dyn Dispatcher>;

/// One rendered list row and its refreshers.
type Kid = (gtk::Widget, Vec<Refresh>);

pub struct Built {
    pub widget: gtk::Widget,
    pub refresh: Vec<Refresh>,
}

/// The row a template is evaluated in (inside a `List` row) and its key.
#[derive(Clone, Default)]
struct Row {
    value: Option<Value>,
    key: Option<String>,
}

fn s<'a>(n: &'a Value, k: &str) -> Option<&'a str> {
    n.get(k).and_then(Value::as_str)
}

fn lookup(store: &Store, row: &Row, key: &str) -> Option<Value> {
    row.value
        .as_ref()
        .and_then(|r| r.get(key))
        .or_else(|| store.get(key))
        .cloned()
}

fn align_of(name: &str) -> gtk::Align {
    match name {
        "start" => gtk::Align::Start,
        "center" => gtk::Align::Center,
        "end" => gtk::Align::End,
        _ => gtk::Align::Fill,
    }
}

/// The monochrome (`-symbolic`) variant of an icon name when the theme has one; app icons keep their colour elsewhere.
fn symbolic(name: &str) -> String {
    if name.ends_with("-symbolic") || name.contains('{') {
        return name.to_string();
    }
    let candidate = format!("{name}-symbolic");
    match gtk::gdk::Display::default() {
        Some(display) if gtk::IconTheme::for_display(&display).has_icon(&candidate) => candidate,
        _ => name.to_string(),
    }
}

/// Applies a child's layout props inside a box. `align` is on the parent's cross axis, `grow` takes free space on its main
/// axis, `width` is a minimum width. Compact controls (buttons, tabs, dropdowns) default to their natural size, not stretched.
fn place(w: &gtk::Widget, node: &Value, parent_vertical: bool) {
    let compact = matches!(
        s(node, "type"),
        Some("Button" | "IconButton" | "Tabs" | "Dropdown")
    );
    let align = s(node, "align")
        .map(align_of)
        .or(compact.then_some(if parent_vertical {
            gtk::Align::Start
        } else {
            gtk::Align::Center
        }));
    if let Some(a) = align {
        if parent_vertical {
            w.set_halign(a);
        } else {
            w.set_valign(a);
        }
    }
    if node.get("grow").and_then(Value::as_bool) == Some(true) {
        if parent_vertical {
            w.set_vexpand(true);
        } else {
            w.set_hexpand(true);
        }
    }
    if let Some(width) = node.get("width").and_then(Value::as_f64) {
        w.set_size_request(width as i32, -1);
    }
}

pub fn build_node(node: &Value, d: &D) -> Built {
    build(node, d, &Row::default())
}

fn build(node: &Value, d: &D, row: &Row) -> Built {
    let ty = s(node, "type").unwrap_or("");
    let mut refresh: Vec<Refresh> = Vec::new();
    let widget: gtk::Widget = match ty {
        "Box" => {
            let vertical = s(node, "direction") != Some("row");
            let spacing = msdk::tokens::spacing_px(s(node, "spacing").unwrap_or("small"));
            let b = gtk::Box::new(
                if vertical {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                },
                spacing,
            );
            for c in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let ch = build(c, d, row);
                place(&ch.widget, c, vertical);
                b.append(&ch.widget);
                refresh.extend(ch.refresh);
            }
            b.upcast()
        }
        "Text" => {
            let lines = node.get("lines").and_then(Value::as_u64).unwrap_or(0) as i32;
            let l = gtk::Label::builder().xalign(0.0).wrap(lines != 1).build();
            if lines > 0 {
                // A fixed number of lines, ending in an ellipsis: titles in rows stay on one line.
                l.set_lines(lines);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            l.add_css_class(&format!("msdk-{}", s(node, "text_style").unwrap_or("body")));
            let (tpl, row) = (s(node, "text").unwrap_or("").to_string(), row.clone());
            let l2 = l.clone();
            refresh.push(Rc::new(move |st| {
                l2.set_text(&interpolate(&tpl, st, row.value.as_ref()))
            }));
            l.upcast()
        }
        "Entry" => entry(node, d, &mut refresh),
        "Button" => {
            let b = gtk::Button::new();
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.set_halign(gtk::Align::Center);
            if let Some(icon) = s(node, "icon") {
                content.append(&gtk::Image::from_icon_name(&symbolic(icon)));
            }
            content.append(&gtk::Label::new(s(node, "label")));
            b.set_child(Some(&content));
            style_button(&b, s(node, "button_style"));
            b.set_valign(gtk::Align::Center);
            wire_click(&b, node, d, row);
            a11y(&b, s(node, "label"));
            b.upcast()
        }
        "IconButton" => icon_button(node, d, row, &mut refresh),
        "Switch" => {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let l = gtk::Label::builder()
                .label(s(node, "label").unwrap_or(""))
                .xalign(0.0)
                .hexpand(true)
                .build();
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            a11y(&sw, s(node, "label"));
            b.append(&l);
            b.append(&sw);
            let bind = s(node, "bind").unwrap_or("").to_string();
            let on = s(node, "on_change").map(String::from);
            let (dd, bk) = (d.clone(), bind.clone());
            sw.connect_active_notify(move |w| {
                if dd.get(&bk).and_then(|v| v.as_bool()) == Some(w.is_active()) {
                    return;
                }
                dd.set(&bk, json!(w.is_active()));
                if let Some(m) = &on {
                    dd.send(m, Some(json!(w.is_active())));
                }
            });
            let sw2 = sw.clone();
            refresh.push(Rc::new(move |st| {
                let want = truthy(st, &bind);
                if sw2.is_active() != want {
                    sw2.set_active(want);
                }
            }));
            b.upcast()
        }
        "Dropdown" => {
            let opts: Vec<String> = node
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            let refs: Vec<&str> = opts.iter().map(String::as_str).collect();
            let dd = gtk::DropDown::from_strings(&refs);
            a11y(&dd, s(node, "label"));
            let (bind, on) = (
                s(node, "bind").unwrap_or("").to_string(),
                s(node, "on_change").map(String::from),
            );
            let (disp, opts2, bk) = (d.clone(), opts.clone(), bind.clone());
            dd.connect_selected_notify(move |w| {
                let Some(v) = opts2.get(w.selected() as usize) else {
                    return;
                };
                if disp
                    .get(&bk)
                    .and_then(|x| x.as_str().map(String::from))
                    .as_deref()
                    == Some(v)
                {
                    return;
                }
                disp.set(&bk, json!(v));
                if let Some(m) = &on {
                    disp.send(m, Some(json!(v)));
                }
            });
            let dd2 = dd.clone();
            refresh.push(Rc::new(move |st| {
                let cur = st.get(&bind).map(display).unwrap_or_default();
                if let Some(i) = opts.iter().position(|o| *o == cur)
                    && dd2.selected() != i as u32
                {
                    dd2.set_selected(i as u32);
                }
            }));
            dd.upcast()
        }
        "List" => list(node, d, row, &mut refresh),
        "Transcript" => transcript(node, d, row, &mut refresh),
        "Tabs" => {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row_box.add_css_class("msdk-tabs");
            let (bind, on) = (
                s(node, "bind").unwrap_or("").to_string(),
                s(node, "on_change").map(String::from),
            );
            let mut group: Option<gtk::ToggleButton> = None;
            let mut buttons: Vec<(String, gtk::ToggleButton)> = Vec::new();
            for opt in node
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let b = gtk::ToggleButton::with_label(opt);
                if let Some(g) = &group {
                    b.set_group(Some(g));
                } else {
                    group = Some(b.clone());
                }
                let (dd, bk, name, on) = (d.clone(), bind.clone(), opt.to_string(), on.clone());
                b.connect_toggled(move |w| {
                    if !w.is_active()
                        || dd
                            .get(&bk)
                            .and_then(|v| v.as_str().map(String::from))
                            .as_deref()
                            == Some(name.as_str())
                    {
                        return;
                    }
                    dd.set(&bk, json!(name));
                    if let Some(m) = &on {
                        dd.send(m, Some(json!(name)));
                    }
                });
                row_box.append(&b);
                buttons.push((opt.to_string(), b));
            }
            refresh.push(Rc::new(move |st| {
                let cur = st.get(&bind).map(display).unwrap_or_default();
                for (name, b) in &buttons {
                    if *name == cur && !b.is_active() {
                        b.set_active(true);
                    }
                }
            }));
            row_box.upcast()
        }
        "Stack" => {
            let stack = gtk::Stack::new();
            stack.set_transition_type(gtk::StackTransitionType::Crossfade);
            stack.set_vexpand(true);
            for c in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let ch = build(c, d, row);
                refresh.extend(ch.refresh);
                stack.add_named(&ch.widget, Some(s(c, "id").unwrap_or("")));
            }
            let bind = s(node, "bind").unwrap_or("").to_string();
            let st2 = stack.clone();
            refresh.push(Rc::new(move |st| {
                let cur = st.get(&bind).map(display).unwrap_or_default();
                if st2.child_by_name(&cur).is_some() {
                    st2.set_visible_child_name(&cur);
                }
            }));
            stack.upcast()
        }
        "Card" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            b.add_css_class("msdk-card");
            if let Some(t) = s(node, "title") {
                let l = gtk::Label::builder().xalign(0.0).build();
                l.add_css_class("msdk-heading");
                let (tpl, row) = (t.to_string(), row.clone());
                let l2 = l.clone();
                refresh.push(Rc::new(move |st| {
                    l2.set_text(&interpolate(&tpl, st, row.value.as_ref()))
                }));
                b.append(&l);
            }
            for c in node
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let ch = build(c, d, row);
                place(&ch.widget, c, true);
                b.append(&ch.widget);
                refresh.extend(ch.refresh);
            }
            b.upcast()
        }
        "EmptyState" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
            b.set_halign(gtk::Align::Center);
            b.set_valign(gtk::Align::Center);
            if let Some(i) = s(node, "icon") {
                let img = gtk::Image::from_icon_name(&symbolic(i));
                img.set_pixel_size(48);
                img.add_css_class("msdk-muted");
                b.append(&img);
            }
            let t = gtk::Label::new(s(node, "title"));
            t.add_css_class("msdk-heading");
            b.append(&t);
            if let Some(dsc) = s(node, "description") {
                let l = gtk::Label::builder().label(dsc).wrap(true).build();
                l.add_css_class("msdk-caption");
                b.append(&l);
            }
            if let Some(a) = node.get("action") {
                let ch = build(a, d, row);
                b.append(&ch.widget);
                refresh.extend(ch.refresh);
            }
            b.upcast()
        }
        "Progress" => {
            let p = gtk::ProgressBar::builder()
                .show_text(true)
                .text(s(node, "label").unwrap_or(""))
                .build();
            a11y(&p, s(node, "label"));
            let bind = s(node, "bind").unwrap_or("").to_string();
            let p2 = p.clone();
            refresh.push(Rc::new(move |st| {
                p2.set_fraction(
                    st.get(&bind)
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0),
                )
            }));
            p.upcast()
        }
        "Gauge" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let max = node.get("max").and_then(Value::as_f64).unwrap_or(1.0);
            let bar = gtk::LevelBar::for_interval(0.0, max);
            a11y(&bar, s(node, "label"));
            let cap = gtk::Label::builder().xalign(0.0).build();
            cap.add_css_class("msdk-caption");
            b.append(
                &gtk::Label::builder()
                    .label(s(node, "label").unwrap_or(""))
                    .xalign(0.0)
                    .build(),
            );
            b.append(&bar);
            b.append(&cap);
            let (bind, tpl, row) = (
                s(node, "bind").unwrap_or("").to_string(),
                s(node, "state_text").unwrap_or("").to_string(),
                row.clone(),
            );
            refresh.push(Rc::new(move |st| {
                bar.set_value(st.get(&bind).and_then(Value::as_f64).unwrap_or(0.0));
                cap.set_text(&interpolate(&tpl, st, row.value.as_ref()));
            }));
            b.upcast()
        }
        "Split" => {
            // Panes run edge to edge; each carries its own padding. The sidebar is tinted and has a divider.
            let (px, py) = (
                msdk::tokens::layout_px("content_padding_x"),
                msdk::tokens::layout_px("content_padding_y"),
            );
            let (mut side, mut main) = (None, None);
            for (slot, start) in [("sidebar", true), ("content", false)] {
                if let Some(n) = node.get(slot) {
                    let ch = build(n, d, row);
                    refresh.extend(ch.refresh);
                    let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    ch.widget.set_margin_start(px);
                    ch.widget.set_margin_end(px);
                    ch.widget.set_margin_top(py);
                    ch.widget.set_margin_bottom(py);
                    ch.widget.set_vexpand(true);
                    pane.append(&ch.widget);
                    pane.set_size_request(
                        if start {
                            msdk::tokens::layout_px("sidebar_width") - 20
                        } else {
                            -1
                        },
                        -1,
                    );
                    if start {
                        pane.add_css_class("msdk-sidebar");
                        side = Some(pane);
                    } else {
                        pane.set_hexpand(true);
                        main = Some(pane);
                    }
                }
            }
            match (side, main) {
                (Some(side), Some(main))
                    if node.get("autohide").and_then(Value::as_bool) == Some(true)
                        && msdk::tokens::feature("sidebar_autohide") =>
                {
                    crate::autohide::build(&side, &main)
                }
                (Some(side), Some(main)) => {
                    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
                    paned.set_start_child(Some(&side));
                    paned.set_resize_start_child(false);
                    paned.set_shrink_start_child(false);
                    paned.set_end_child(Some(&main));
                    paned.set_position(msdk::tokens::layout_px("sidebar_width"));
                    paned.upcast()
                }
                _ => gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
            }
        }
        _ => {
            let l = gtk::Label::new(Some(&format!(
                "[{}: {} not rendered]",
                s(node, "name").unwrap_or(ty),
                s(node, "reason").unwrap_or("unsupported")
            )));
            l.add_css_class("msdk-muted");
            l.upcast()
        }
    };
    if let Some(t) = s(node, "tooltip") {
        widget.set_tooltip_text(Some(t));
    }
    if let Some(key) = s(node, "visible_if") {
        let (w, key, row) = (widget.clone(), key.to_string(), row.clone());
        refresh.push(Rc::new(move |st| {
            w.set_visible(
                lookup(st, &row, &key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
        }));
    }
    Built { widget, refresh }
}

fn a11y(w: &impl IsA<gtk::Accessible>, label: Option<&str>) {
    if let Some(l) = label {
        w.update_property(&[gtk::accessible::Property::Label(l)]);
    }
}

fn style_button(b: &gtk::Button, style: Option<&str>) {
    b.add_css_class("msdk-control");
    match style {
        Some("suggested") => b.add_css_class("suggested-action"),
        Some("destructive") => b.add_css_class("destructive-action"),
        Some("flat") => b.add_css_class("flat"),
        _ => {}
    }
}

/// Click -> message. Inside a list row the row's key is the payload; a `confirm` text asks first.
fn wire_click(b: &gtk::Button, node: &Value, d: &D, row: &Row) {
    let Some(msg) = s(node, "on").map(String::from) else {
        return;
    };
    let (d, confirm, key) = (
        d.clone(),
        s(node, "confirm").map(String::from),
        row.key.clone(),
    );
    b.connect_clicked(move |_| {
        let payload = key.clone().map(|k| json!(k));
        match &confirm {
            Some(text) => {
                let (d2, m, p) = (d.clone(), msg.clone(), payload.clone());
                d.confirm(text, Box::new(move || d2.send(&m, p)));
            }
            None => d.send(&msg, payload),
        }
    });
}

fn icon_button(node: &Value, d: &D, row: &Row, refresh: &mut Vec<Refresh>) -> gtk::Widget {
    let btn = gtk::Button::new();
    btn.add_css_class("flat");
    btn.add_css_class("msdk-control");
    let img = gtk::Image::new();
    img.set_pixel_size(40);
    btn.set_child(Some(&img));
    wire_click(&btn, node, d, row);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&btn));
    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indicator.add_css_class("msdk-indicator");
    indicator.set_halign(gtk::Align::Center);
    indicator.set_valign(gtk::Align::End);
    indicator.set_can_target(false);
    overlay.add_overlay(&indicator);
    let badge = gtk::Label::new(None);
    badge.add_css_class("msdk-badge");
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::Start);
    badge.set_can_target(false);
    overlay.add_overlay(&badge);
    let (icon_t, label_t, ind_t, badge_t, row) = (
        s(node, "icon").unwrap_or("").to_string(),
        s(node, "label").unwrap_or("").to_string(),
        s(node, "indicator").unwrap_or("none").to_string(),
        s(node, "badge").unwrap_or("").to_string(),
        row.clone(),
    );
    let btn2 = btn.clone();
    refresh.push(Rc::new(move |st| {
        let r = row.value.as_ref();
        img.set_icon_name(Some(&interpolate(&icon_t, st, r)));
        let label = interpolate(&label_t, st, r);
        btn2.set_tooltip_text(Some(&label));
        btn2.update_property(&[gtk::accessible::Property::Label(&label)]);
        let ind = interpolate(&ind_t, st, r);
        indicator.set_visible(ind == "open" || ind == "focused");
        if ind == "focused" {
            indicator.add_css_class("focused");
        } else {
            indicator.remove_css_class("focused");
        }
        let bd = interpolate(&badge_t, st, r);
        badge.set_visible(!bd.is_empty());
        badge.set_text(&bd);
    }));
    overlay.upcast()
}

fn entry(node: &Value, d: &D, refresh: &mut Vec<Refresh>) -> gtk::Widget {
    let bind = s(node, "bind").unwrap_or("").to_string();
    let (on_change, on_submit) = (
        s(node, "on_change").map(String::from),
        s(node, "on_submit").map(String::from),
    );
    let kind = s(node, "entry_kind").unwrap_or("text");
    if kind == "multiline" {
        let line_h = 24.0;
        let lines_min = (node.get("lines").and_then(Value::as_f64).unwrap_or(3.0) * line_h) as i32;
        let lines_max =
            (node.get("max_lines").and_then(Value::as_f64).unwrap_or(8.0) * line_h) as i32;
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        a11y(&view, s(node, "label"));
        if node.get("enter_sends").and_then(Value::as_bool) == Some(true)
            && let Some(m) = on_submit.clone()
        {
            // Enter sends; Shift+Enter inserts a line break.
            let (dd, key) = (d.clone(), gtk::EventControllerKey::new());
            key.connect_key_pressed(move |_, k, _, state| {
                if matches!(k, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
                    && !state.contains(gtk::gdk::ModifierType::SHIFT_MASK)
                {
                    dd.send(&m, None);
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
            view.add_controller(key);
        }
        let buf = view.buffer();
        let (dd, bk) = (d.clone(), bind.clone());
        buf.connect_changed(move |b| {
            let t = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            if dd
                .get(&bk)
                .and_then(|v| v.as_str().map(String::from))
                .as_deref()
                == Some(t.as_str())
            {
                return;
            }
            dd.set(&bk, json!(t));
            if let Some(m) = &on_change {
                dd.send(m, Some(json!(t)));
            }
        });
        let b2 = buf.clone();
        refresh.push(Rc::new(move |st| {
            let want = st.get(&bind).map(display).unwrap_or_default();
            if b2.text(&b2.start_iter(), &b2.end_iter(), false) != want {
                b2.set_text(&want);
            }
        }));
        let sw = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(false)
            .hexpand(true)
            .min_content_height(lines_min)
            .max_content_height(lines_max)
            .propagate_natural_height(true)
            .build();
        sw.add_css_class("msdk-editor");
        return sw.upcast();
    }
    let e: gtk::Entry = if kind == "search" {
        gtk::Entry::builder()
            .primary_icon_name("edit-find-symbolic")
            .build()
    } else {
        gtk::Entry::new()
    };
    e.add_css_class("msdk-control");
    if kind == "password" {
        e.set_visibility(false);
        e.set_input_purpose(gtk::InputPurpose::Password);
    }
    if let Some(p) = s(node, "placeholder").or(s(node, "label")) {
        e.set_placeholder_text(Some(p));
    }
    a11y(&e, s(node, "label"));
    let (dd, bk) = (d.clone(), bind.clone());
    e.connect_changed(move |w| {
        let t = w.text().to_string();
        if dd
            .get(&bk)
            .and_then(|v| v.as_str().map(String::from))
            .as_deref()
            == Some(t.as_str())
        {
            return;
        }
        dd.set(&bk, json!(t));
        if let Some(m) = &on_change {
            dd.send(m, Some(json!(t)));
        }
    });
    if let Some(m) = on_submit {
        let dd = d.clone();
        e.connect_activate(move |_| dd.send(&m, None));
    }
    let e2 = e.clone();
    refresh.push(Rc::new(move |st| {
        let want = st.get(&bind).map(display).unwrap_or_default();
        if e2.text() != want {
            e2.set_text(&want);
        }
    }));
    e.upcast()
}

fn list(node: &Value, d: &D, row: &Row, refresh: &mut Vec<Refresh>) -> gtk::Widget {
    let horizontal = s(node, "direction") == Some("row");
    let holder = gtk::Box::new(
        if horizontal {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        },
        4,
    );
    let empty = node.get("empty").map(|n| build(n, d, row));
    if let Some(e) = &empty {
        holder.append(&e.widget);
        refresh.extend(e.refresh.iter().cloned());
    }
    let (items_key, key_field) = (
        s(node, "items").unwrap_or("").to_string(),
        s(node, "key").unwrap_or("id").to_string(),
    );
    let (template, on_select) = (
        node.get("row").cloned().unwrap_or(Value::Null),
        s(node, "on_select").map(String::from),
    );
    let group_by = s(node, "group_by").map(String::from);
    let selected_key = s(node, "selected").map(String::from);
    let scroll = node.get("scroll").and_then(Value::as_bool).unwrap_or(true) && !horizontal;
    let last: Rc<RefCell<Option<Value>>> = Rc::new(RefCell::new(None));
    let kids: Rc<RefCell<Vec<Kid>>> = Rc::new(RefCell::new(Vec::new()));
    let d = d.clone();
    let empty_widget = empty.map(|e| e.widget);
    let list_box = holder.clone();
    refresh.push(Rc::new(move |st| {
        let items = st.get(&items_key).cloned().unwrap_or(json!([]));
        if last.borrow().as_ref() != Some(&items) {
            for (w, _) in kids.borrow_mut().drain(..) {
                list_box.remove(&w);
            }
            let arr = items.as_array().cloned().unwrap_or_default();
            if let Some(e) = &empty_widget {
                e.set_visible(arr.is_empty());
            }
            // Sections: items are shown under a header per distinct `group_by` value, in order of first appearance.
            let arr = match &group_by {
                Some(g) => {
                    let mut order: Vec<String> = Vec::new();
                    for it in &arr {
                        let v = it.get(g).map(display).unwrap_or_default();
                        if !order.contains(&v) {
                            order.push(v);
                        }
                    }
                    let mut out = Vec::new();
                    for v in order {
                        out.push(json!({ "__section": v.clone() }));
                        out.extend(
                            arr.iter()
                                .filter(|it| it.get(g).map(display).unwrap_or_default() == v)
                                .cloned(),
                        );
                    }
                    out
                }
                None => arr,
            };
            for item in arr {
                if let Some(title) = item.get("__section").and_then(Value::as_str) {
                    let h = gtk::Label::builder()
                        .label(if title.is_empty() { "Other" } else { title })
                        .xalign(0.0)
                        .margin_top(8)
                        .build();
                    h.add_css_class("msdk-caption");
                    h.add_css_class("heading");
                    list_box.append(&h);
                    h.set_widget_name("__section");
                    kids.borrow_mut().push((h.upcast(), Vec::new()));
                    continue;
                }
                let key = item.get(&key_field).map(display);
                let key_name = key.clone();
                let r = Row {
                    value: Some(item),
                    key: key.clone(),
                };
                let built = build(&template, &d, &r);
                if let (Some(m), Some(k)) = (on_select.clone(), key) {
                    let (dd, g) = (d.clone(), gtk::GestureClick::new());
                    g.connect_released(move |_, _, _, _| dd.send(&m, Some(json!(k))));
                    built.widget.add_controller(g);
                }
                built.widget.add_css_class("msdk-row");
                built
                    .widget
                    .set_widget_name(key_name.as_deref().unwrap_or(""));
                list_box.append(&built.widget);
                kids.borrow_mut().push((built.widget, built.refresh));
            }
            *last.borrow_mut() = Some(items);
        }
        let sel = selected_key
            .as_ref()
            .and_then(|k| st.get(k))
            .map(display)
            .unwrap_or_default();
        for (w, rs) in kids.borrow().iter() {
            for r in rs {
                r(st);
            }
            if selected_key.is_some() && w.widget_name() != "__section" {
                if !sel.is_empty() && w.widget_name() == sel.as_str() {
                    w.add_css_class("msdk-selected");
                } else {
                    w.remove_css_class("msdk-selected");
                }
            }
        }
    }));
    if scroll {
        let sw = gtk::ScrolledWindow::builder()
            .child(&holder)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        return sw.upcast();
    }
    holder.upcast()
}

/// A conversation: each entry is drawn by its `kind`. Permission entries carry Allow and Deny buttons that send the entry id.
fn transcript(node: &Value, d: &D, row: &Row, refresh: &mut Vec<Refresh>) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_margin_start(msdk::tokens::layout_px("content_padding_x"));
    column.set_margin_end(msdk::tokens::layout_px("content_padding_x"));
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .vexpand(true)
        .hexpand(true)
        .build();
    let empty = node.get("empty").map(|n| build(n, d, row));
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroller));
    if let Some(e) = &empty {
        overlay.add_overlay(&e.widget);
        refresh.extend(e.refresh.iter().cloned());
    }
    let empty_widget = empty.map(|e| e.widget);
    let items_key = s(node, "items").unwrap_or("").to_string();
    let (on_allow, on_deny) = (
        s(node, "on_allow").map(String::from),
        s(node, "on_deny").map(String::from),
    );
    let last: Rc<RefCell<Option<Value>>> = Rc::new(RefCell::new(None));
    let d = d.clone();
    refresh.push(Rc::new(move |st| {
        let items = st.get(&items_key).cloned().unwrap_or(json!([]));
        if last.borrow().as_ref() == Some(&items) {
            return;
        }
        while let Some(c) = column.first_child() {
            column.remove(&c);
        }
        let arr = items.as_array().cloned().unwrap_or_default();
        if let Some(e) = &empty_widget {
            e.set_visible(arr.is_empty());
        }
        for it in &arr {
            let text = it.get("text").map(display).unwrap_or_default();
            let id = it.get("id").map(display).unwrap_or_default();
            let label = |css: &str, xalign: f32| {
                let l = gtk::Label::builder()
                    .label(&text)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(xalign)
                    .selectable(true)
                    .build();
                l.add_css_class(css);
                l
            };
            let w: gtk::Widget = match it
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
            {
                "user" => {
                    let l = label("msdk-bubble-user", 0.0);
                    l.set_halign(gtk::Align::End);
                    l.upcast()
                }
                "code" => label("msdk-code", 0.0).upcast(),
                "tool" => label("msdk-tool", 0.0).upcast(),
                "permission" => {
                    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    card.add_css_class("msdk-card");
                    card.append(&label("msdk-body", 0.0));
                    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    for (title, msg, css) in [
                        ("Allow once", on_allow.clone(), "suggested-action"),
                        ("Deny", on_deny.clone(), "destructive-action"),
                    ] {
                        let b = gtk::Button::with_label(title);
                        b.add_css_class(css);
                        b.add_css_class("msdk-control");
                        if let Some(m) = msg {
                            let (dd, id) = (d.clone(), id.clone());
                            b.connect_clicked(move |_| dd.send(&m, Some(json!(id))));
                        }
                        buttons.append(&b);
                    }
                    card.append(&buttons);
                    card.upcast()
                }
                _ => label("msdk-bubble", 0.0).upcast(),
            };
            column.append(&w);
        }
        // Keep the newest entry in view.
        let adj = scroller.vadjustment();
        gtk::glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
        *last.borrow_mut() = Some(items);
    }));
    overlay.upcast()
}
