//! Loading skeletons and compact empty states: grey bars that pulse where content is about to
//! appear (a transcript, the thread list, an issue list), and an icon + title + hint block for a
//! pane with nothing in it. The pulse is a CSS animation, so it costs nothing once the skeleton is
//! replaced or hidden.

use crate::chat::label;
use gtk::prelude::*;

/// The pulse runs only while the skeleton is on screen: the class that carries the CSS animation
/// is added on map and removed on unmap, so a skeleton parked in a hidden stack page costs nothing.
fn pulse_while_mapped(root: &gtk::Box) {
    root.connect_map(|r| r.add_css_class("pulsing"));
    root.connect_unmap(|r| r.remove_css_class("pulsing"));
}

fn bar(width: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("skeleton-bar");
    b.set_size_request(width, 12);
    b.set_halign(gtk::Align::Start);
    b
}

/// Chat bubbles: alternating sides, a few lines each.
pub fn chat(bubbles: usize) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 18);
    root.add_css_class("skeleton");
    root.set_margin_top(24);
    root.set_margin_start(24);
    root.set_margin_end(24);
    root.update_property(&[gtk::accessible::Property::Label("Loading")]);
    for i in 0..bubbles {
        let bubble = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let widths: &[i32] = if i % 2 == 0 { &[420, 360, 240] } else { &[300, 180] };
        for w in widths {
            bubble.append(&bar(*w));
        }
        bubble.set_halign(if i % 2 == 0 { gtk::Align::Start } else { gtk::Align::End });
        root.append(&bubble);
    }
    pulse_while_mapped(&root);
    root
}

/// List rows: a dot and two lines each.
pub fn rows(n: usize) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.add_css_class("skeleton");
    root.set_margin_top(12);
    root.set_margin_start(12);
    root.set_margin_end(12);
    root.update_property(&[gtk::accessible::Property::Label("Loading")]);
    for i in 0..n {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let dot = bar(14);
        dot.set_size_request(14, 14);
        dot.set_valign(gtk::Align::Start);
        row.append(&dot);
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 6);
        lines.append(&bar(if i % 3 == 0 { 170 } else { 130 }));
        lines.append(&bar(90));
        row.append(&lines);
        root.append(&row);
    }
    pulse_while_mapped(&root);
    root
}

/// A compact empty state for a pane or a list: icon, title, hint.
pub fn empty(icon: &str, title: &str, hint: &str) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("empty-state");
    root.set_valign(gtk::Align::Center);
    root.set_halign(gtk::Align::Center);
    root.set_margin_top(24);
    root.set_margin_bottom(24);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(40);
    image.add_css_class("dim-label");
    root.append(&image);
    let t = label(title, &["title-4"]);
    t.set_justify(gtk::Justification::Center);
    t.set_wrap(true);
    root.append(&t);
    let h = label(hint, &["dim-label", "caption"]);
    h.set_justify(gtk::Justification::Center);
    h.set_wrap(true);
    h.set_max_width_chars(34);
    root.append(&h);
    root
}

/// The title and hint labels of an `empty()` block, for updating in place.
pub fn empty_labels(block: &gtk::Box) -> (gtk::Label, gtk::Label) {
    let mut labels = Vec::new();
    let mut child = block.first_child();
    while let Some(c) = child {
        if let Some(l) = c.downcast_ref::<gtk::Label>() {
            labels.push(l.clone());
        }
        child = c.next_sibling();
    }
    (labels[0].clone(), labels[1].clone())
}
