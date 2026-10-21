//! Cairo drawing for the two views, plus hit testing against what was last drawn.
#![allow(clippy::too_many_arguments)] // drawing helpers pass geometry and colour explicitly

use crate::layout::{RING_DEPTH, Rect, squarify};
use crate::look::{Category, category, size_text};
use crate::model::{Kind, Mode, Model, NodeId, Tree};
use gtk::{cairo, pango, prelude::*};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const MAX_CARDS: usize = 60;
const MIN_CARD_SHARE: f64 = 0.003;
const MAX_NESTED: usize = 24;
const MIN_NESTED_SHARE: f64 = 0.012;
const HEADER_H: f64 = 38.0;

pub struct CardHit {
    pub rect: Rect,
    /// `None` is the "smaller items" card.
    pub id: Option<NodeId>,
    pub more: Option<(usize, u64)>,
}

pub struct RingGeometry {
    pub cx: f64,
    pub cy: f64,
    pub r0: f64,
    pub ring_w: f64,
}

#[derive(Default)]
pub enum Hits {
    #[default]
    None,
    Cards(Vec<CardHit>),
    Rings(RingGeometry),
}

pub enum Hit {
    Node(NodeId),
    More(usize, u64),
    Centre,
}

impl Hits {
    pub fn at(&self, m: &Model, x: f64, y: f64) -> Option<Hit> {
        match self {
            Hits::None => None,
            Hits::Cards(cards) => cards.iter().find(|c| c.rect.contains(x, y)).map(|c| match (c.id, c.more) {
                (Some(id), _) => Hit::Node(id),
                (None, Some((n, size))) => Hit::More(n, size),
                (None, None) => Hit::More(0, 0),
            }),
            Hits::Rings(g) => {
                let (dx, dy) = (x - g.cx, y - g.cy);
                let r = dx.hypot(dy);
                if r < g.r0 {
                    return Some(Hit::Centre);
                }
                let depth = ((r - g.r0) / g.ring_w).floor() as u32 + 1;
                if depth > RING_DEPTH {
                    return None;
                }
                let mut theta = dy.atan2(dx) + FRAC_PI_2;
                if theta < 0.0 {
                    theta += TAU;
                }
                m.rings.iter().find(|s| s.depth == depth && theta >= s.a0 && theta < s.a1).map(|s| Hit::Node(s.id))
            }
        }
    }
}

type Rgba = (f64, f64, f64, f64);

struct Text {
    layout: pango::Layout,
    base: pango::FontDescription,
}

impl Text {
    fn new(area: &gtk::DrawingArea) -> Self {
        Text {
            layout: area.create_pango_layout(None),
            base: area.pango_context().font_description().unwrap_or_default(),
        }
    }

    /// One ellipsized, left-aligned line at `(x, y)` no wider than `w`; returns its height in pixels.
    fn line(&self, cr: &cairo::Context, s: &str, x: f64, y: f64, w: f64, pt: f64, bold: bool, c: Rgba) -> f64 {
        self.aligned(cr, s, x, y, w, pt, bold, c, pango::Alignment::Left)
    }

    /// Same, centred inside the `w` pixels starting at `x`.
    fn centred(&self, cr: &cairo::Context, s: &str, x: f64, y: f64, w: f64, pt: f64, bold: bool, c: Rgba) -> f64 {
        self.aligned(cr, s, x, y, w, pt, bold, c, pango::Alignment::Center)
    }

    #[allow(clippy::too_many_arguments)]
    fn aligned(
        &self,
        cr: &cairo::Context,
        s: &str,
        x: f64,
        y: f64,
        w: f64,
        pt: f64,
        bold: bool,
        c: Rgba,
        align: pango::Alignment,
    ) -> f64 {
        let mut font = self.base.clone();
        font.set_size((pt * pango::SCALE as f64) as i32);
        font.set_weight(if bold { pango::Weight::Bold } else { pango::Weight::Normal });
        self.layout.set_font_description(Some(&font));
        self.layout.set_text(s);
        self.layout.set_width((w.max(1.0) * pango::SCALE as f64) as i32);
        self.layout.set_ellipsize(pango::EllipsizeMode::End);
        self.layout.set_height(-1);
        self.layout.set_alignment(align);
        cr.set_source_rgba(c.0, c.1, c.2, c.3);
        cr.move_to(x, y);
        pangocairo::functions::show_layout(cr, &self.layout);
        self.layout.pixel_size().1 as f64
    }
}

pub fn paint(m: &Model, area: &gtk::DrawingArea, cr: &cairo::Context, w: i32, h: i32) -> Hits {
    let Some(tree) = &m.tree else { return Hits::None };
    let fg = area.color();
    let fg = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
    let text = Text::new(area);
    let (w, h) = (w as f64, h as f64);
    match m.mode {
        Mode::Cards => Hits::Cards(paint_cards(m, tree, &text, cr, w, h, fg)),
        Mode::Circle => Hits::Rings(paint_rings(m, tree, &text, cr, w, h, fg)),
    }
}

fn rounded(cr: &cairo::Context, r: Rect, radius: f64) {
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(r.x + r.w - radius, r.y + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(r.x + r.w - radius, r.y + r.h - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(r.x + radius, r.y + r.h - radius, radius, FRAC_PI_2, PI);
    cr.arc(r.x + radius, r.y + radius, radius, PI, PI + FRAC_PI_2);
    cr.close_path();
}

fn paint_empty(text: &Text, cr: &cairo::Context, w: f64, h: f64, fg: (f64, f64, f64), msg: &str) {
    text.centred(cr, msg, 0.0, h / 2.0 - 8.0, w, 11.0, false, (fg.0, fg.1, fg.2, 0.6));
}

fn paint_cards(
    m: &Model,
    tree: &Tree,
    text: &Text,
    cr: &cairo::Context,
    w: f64,
    h: f64,
    fg: (f64, f64, f64),
) -> Vec<CardHit> {
    let cwd = tree.node(m.cwd);
    let bounds = Rect { x: 12.0, y: 12.0, w: (w - 24.0).max(0.0), h: (h - 24.0).max(0.0) };
    let mut hits = Vec::new();
    let total: u64 = cwd.children.iter().map(|c| tree.node(*c).size).sum();
    if total == 0 {
        paint_empty(
            text,
            cr,
            w,
            h,
            fg,
            if cwd.children.is_empty() { "This folder is empty" } else { "Nothing here uses disk space" },
        );
        return hits;
    }

    // Largest cards, then one card for everything too small to see.
    let mut cards: Vec<(Option<NodeId>, u64)> = Vec::new();
    let mut taken = 0u64;
    for &child in &cwd.children {
        let size = tree.node(child).size;
        if size == 0 || cards.len() >= MAX_CARDS || (size as f64 / total as f64) < MIN_CARD_SHARE {
            break;
        }
        cards.push((Some(child), size));
        taken += size;
    }
    let rest_count = cwd.children.len() - cards.len();
    let rest_size = total - taken;
    if rest_size > 0 {
        cards.push((None, rest_size));
        cards.sort_by_key(|c| std::cmp::Reverse(c.1));
    }

    let sizes: Vec<f64> = cards.iter().map(|c| c.1 as f64).collect();
    let rects = squarify(&sizes, bounds);
    for ((id, size), rect) in cards.into_iter().zip(rects) {
        let rect = rect.inset(2.0);
        if rect.w < 2.0 || rect.h < 2.0 {
            continue;
        }
        let hovered = id.is_some() && id == m.hover;
        let selected = id.is_some_and(|i| m.selected.contains(&i));
        match id {
            Some(id) => draw_card(tree, text, cr, id, rect, total, hovered, selected, fg),
            None => draw_more(text, cr, rect, rest_count, size, hovered),
        }
        hits.push(CardHit { rect, id, more: id.is_none().then_some((rest_count, size)) });
    }
    hits
}

fn draw_card(
    tree: &Tree,
    text: &Text,
    cr: &cairo::Context,
    id: NodeId,
    r: Rect,
    total: u64,
    hovered: bool,
    selected: bool,
    fg: (f64, f64, f64),
) {
    let node = tree.node(id);
    let (cr_, cg, cb) = category(node).rgb();
    rounded(cr, r, 7.0);
    cr.set_source_rgba(cr_, cg, cb, if node.kind == Kind::Mount { 0.45 } else { 1.0 });
    let _ = cr.fill_preserve();
    if hovered {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        let _ = cr.fill_preserve();
    }
    cr.new_path();

    let white = (1.0, 1.0, 1.0, 1.0);
    let dim = (1.0, 1.0, 1.0, 0.78);
    let pad = 9.0;
    let roomy = r.w >= 70.0 && r.h >= 36.0;
    let mut header = 0.0;
    if roomy {
        let h1 = text.line(cr, &node.name, r.x + pad, r.y + 7.0, r.w - 2.0 * pad, 10.5, true, white);
        let line2 = format!("{} · {}", size_text(node.size), crate::look::percent_text(node.size, total));
        let h2 = text.line(cr, &line2, r.x + pad, r.y + 7.0 + h1 + 1.0, r.w - 2.0 * pad, 9.0, false, dim);
        header = 7.0 + h1 + h2 + 6.0;
    } else if r.w >= 40.0 && r.h >= 18.0 {
        text.line(cr, &node.name, r.x + 5.0, r.y + 3.0, r.w - 10.0, 8.0, true, white);
    }

    // A glimpse of what is inside, so a big folder shows where its weight sits.
    if node.kind == Kind::Dir && !node.children.is_empty() && roomy && r.h - header > 40.0 && r.w > 90.0 {
        let inner =
            Rect { x: r.x + 6.0, y: r.y + header.max(HEADER_H), w: r.w - 12.0, h: r.h - header.max(HEADER_H) - 6.0 };
        if inner.w > 20.0 && inner.h > 20.0 {
            draw_nested(tree, text, cr, id, inner);
        }
    }

    if selected {
        rounded(cr, r, 7.0);
        cr.set_source_rgba(fg.0, fg.1, fg.2, 0.95);
        cr.set_line_width(3.0);
        let _ = cr.stroke();
        rounded(cr, r.inset(3.0), 5.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
}

fn draw_nested(tree: &Tree, text: &Text, cr: &cairo::Context, id: NodeId, inner: Rect) {
    let node = tree.node(id);
    let mut tiles: Vec<(Option<NodeId>, u64)> = Vec::new();
    let mut taken = 0u64;
    for &child in &node.children {
        let size = tree.node(child).size;
        if size == 0 || tiles.len() >= MAX_NESTED || (size as f64 / node.size as f64) < MIN_NESTED_SHARE {
            break;
        }
        tiles.push((Some(child), size));
        taken += size;
    }
    let children_total: u64 = node.children.iter().map(|c| tree.node(*c).size).sum();
    if children_total > taken {
        tiles.push((None, children_total - taken));
        tiles.sort_by_key(|t| std::cmp::Reverse(t.1));
    }
    let sizes: Vec<f64> = tiles.iter().map(|t| t.1 as f64).collect();
    for ((child, size), rect) in tiles.into_iter().zip(squarify(&sizes, inner)) {
        let rect = rect.inset(1.0);
        if rect.w < 3.0 || rect.h < 3.0 {
            continue;
        }
        rounded(cr, rect, 3.0);
        match child {
            Some(c) => {
                let (r, g, b) = category(tree.node(c)).rgb();
                // Tint toward white so the tile reads as "inside" its card yet keeps its type colour.
                cr.set_source_rgba(r * 0.55 + 0.45, g * 0.55 + 0.45, b * 0.55 + 0.45, 0.30);
            }
            None => cr.set_source_rgba(1.0, 1.0, 1.0, 0.07),
        }
        let _ = cr.fill();
        if let Some(c) = child
            && rect.w >= 64.0
            && rect.h >= 22.0
        {
            let n = tree.node(c);
            let h1 =
                text.line(cr, &n.name, rect.x + 5.0, rect.y + 3.0, rect.w - 10.0, 8.5, false, (1.0, 1.0, 1.0, 0.92));
            if rect.h >= 22.0 + h1 {
                text.line(
                    cr,
                    &size_text(size),
                    rect.x + 5.0,
                    rect.y + 3.0 + h1,
                    rect.w - 10.0,
                    8.0,
                    false,
                    (1.0, 1.0, 1.0, 0.65),
                );
            }
        }
    }
}

fn draw_more(text: &Text, cr: &cairo::Context, r: Rect, count: usize, size: u64, hovered: bool) {
    rounded(cr, r, 7.0);
    let (red, green, blue) = Category::Mount.rgb();
    cr.set_source_rgba(red, green, blue, if hovered { 0.45 } else { 0.32 });
    let _ = cr.fill();
    if r.w >= 70.0 && r.h >= 36.0 {
        let label = format!("{} smaller items", crate::look::group_digits(count as u64));
        let h1 = text.line(cr, &label, r.x + 9.0, r.y + 7.0, r.w - 18.0, 9.5, true, (1.0, 1.0, 1.0, 0.9));
        text.line(cr, &size_text(size), r.x + 9.0, r.y + 7.0 + h1 + 1.0, r.w - 18.0, 9.0, false, (1.0, 1.0, 1.0, 0.7));
    }
}

fn paint_rings(
    m: &Model,
    tree: &Tree,
    text: &Text,
    cr: &cairo::Context,
    w: f64,
    h: f64,
    fg: (f64, f64, f64),
) -> RingGeometry {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r_max = (w.min(h) / 2.0 - 14.0).max(60.0);
    let r0 = r_max * 0.27;
    let ring_w = (r_max - r0) / RING_DEPTH as f64;
    let geometry = RingGeometry { cx, cy, r0, ring_w };
    let start = -FRAC_PI_2;
    let cwd = tree.node(m.cwd);

    for seg in &m.rings {
        let node = tree.node(seg.id);
        let r_in = r0 + (seg.depth - 1) as f64 * ring_w + 1.5;
        let r_out = r0 + seg.depth as f64 * ring_w - 1.5;
        let gap = 1.6 / r_out;
        let (a0, a1) = (start + seg.a0 + gap / 2.0, start + seg.a1 - gap / 2.0);
        if a1 <= a0 {
            continue;
        }
        let (mut r, mut g, mut b) = category(node).rgb();
        if node.kind == Kind::Dir {
            // Deeper folders fade toward white, so the rings read outward from the centre.
            let t = (seg.depth - 1) as f64 * 0.11;
            (r, g, b) = (r + (1.0 - r) * t, g + (1.0 - g) * t, b + (1.0 - b) * t);
        }
        cr.new_path();
        cr.arc(cx, cy, r_out, a0, a1);
        cr.arc_negative(cx, cy, r_in, a1, a0);
        cr.close_path();
        cr.set_source_rgba(r, g, b, if node.kind == Kind::Mount { 0.5 } else { 1.0 });
        let _ = cr.fill_preserve();
        if m.hover == Some(seg.id) {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.18);
            let _ = cr.fill_preserve();
        }
        if m.selected.contains(&seg.id) {
            cr.set_source_rgba(fg.0, fg.1, fg.2, 0.95);
            cr.set_line_width(2.5);
            let _ = cr.stroke();
        } else {
            cr.new_path();
        }
    }

    // Name the big wedges of the first ring.
    for seg in m.rings.iter().filter(|s| s.depth == 1 && s.a1 - s.a0 > 0.42) {
        let mid = start + (seg.a0 + seg.a1) / 2.0;
        let rm = r0 + ring_w * 0.5;
        let (px, py) = (cx + rm * mid.cos(), cy + rm * mid.sin());
        let width = ring_w * 1.25;
        let node = tree.node(seg.id);
        let h1 = text.centred(cr, &node.name, px - width / 2.0, py - 12.0, width, 9.0, true, (1.0, 1.0, 1.0, 0.95));
        text.centred(
            cr,
            &size_text(node.size),
            px - width / 2.0,
            py - 12.0 + h1,
            width,
            8.0,
            false,
            (1.0, 1.0, 1.0, 0.75),
        );
    }

    // Centre disc: where you are, and a way up.
    cr.new_path();
    cr.arc(cx, cy, r0 - 4.0, 0.0, TAU);
    cr.set_source_rgba(fg.0, fg.1, fg.2, if m.hover.is_none() { 0.07 } else { 0.05 });
    let _ = cr.fill();
    let inner = (r0 - 14.0) * 2.0 * 0.86;
    let top = cy - 30.0;
    let h1 = text.centred(cr, &size_text(cwd.size), cx - inner / 2.0, top, inner, 15.0, true, (fg.0, fg.1, fg.2, 0.95));
    let h2 = text.centred(cr, &cwd.name, cx - inner / 2.0, top + h1 + 2.0, inner, 9.5, false, (fg.0, fg.1, fg.2, 0.75));
    if cwd.parent != crate::model::NO_NODE {
        text.centred(
            cr,
            "Click to go up",
            cx - inner / 2.0,
            top + h1 + h2 + 4.0,
            inner,
            8.0,
            false,
            (fg.0, fg.1, fg.2, 0.5),
        );
    }
    geometry
}
