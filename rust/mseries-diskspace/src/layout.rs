//! Pure geometry: squarified treemap rectangles and sunburst arcs. No GTK here, so it is unit tested.

use crate::model::{Kind, NodeId, SortKey, Tree};
use std::f64::consts::TAU;

/// Rings drawn around the centre disc.
pub const RING_DEPTH: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn inset(self, d: f64) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: (self.w - 2.0 * d).max(0.0), h: (self.h - 2.0 * d).max(0.0) }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Bruls, Huizing and van Wijk squarified treemap. `sizes` must be largest first; the result has one
/// rectangle per size and the rectangles tile `bounds` exactly, each with area proportional to its size.
pub fn squarify(sizes: &[f64], bounds: Rect) -> Vec<Rect> {
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return vec![Rect::ZERO; sizes.len()];
    }
    let scale = bounds.w * bounds.h / total;
    let areas: Vec<f64> = sizes.iter().map(|s| s * scale).collect();
    let mut out = Vec::with_capacity(areas.len());
    let mut rest = bounds;
    let mut i = 0;
    while i < areas.len() {
        let side = rest.w.min(rest.h);
        let mut j = i + 1;
        let mut sum = areas[i];
        let mut best = worst_ratio(&areas[i..j], sum, side);
        while j < areas.len() {
            let next_sum = sum + areas[j];
            let ratio = worst_ratio(&areas[i..=j], next_sum, side);
            if ratio > best {
                break;
            }
            sum = next_sum;
            best = ratio;
            j += 1;
        }
        if rest.w >= rest.h {
            let col_w = sum / rest.h;
            let mut y = rest.y;
            for a in &areas[i..j] {
                let h = a / col_w;
                out.push(Rect { x: rest.x, y, w: col_w, h });
                y += h;
            }
            rest.x += col_w;
            rest.w = (rest.w - col_w).max(0.0);
        } else {
            let row_h = sum / rest.w;
            let mut x = rest.x;
            for a in &areas[i..j] {
                let w = a / row_h;
                out.push(Rect { x, y: rest.y, w, h: row_h });
                x += w;
            }
            rest.y += row_h;
            rest.h = (rest.h - row_h).max(0.0);
        }
        i = j;
    }
    out
}

fn worst_ratio(row: &[f64], sum: f64, side: f64) -> f64 {
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let s2 = side * side;
    ((s2 * max) / (sum * sum)).max((sum * sum) / (s2 * min))
}

#[derive(Clone, Copy, Debug)]
pub struct ArcSeg {
    pub id: NodeId,
    /// 1 is the ring touching the centre disc.
    pub depth: u32,
    /// Radians clockwise from twelve o'clock.
    pub a0: f64,
    pub a1: f64,
}

/// Arcs for the subtree under `root`, `max_depth` rings deep. Arcs thinner than `min_angle` are dropped
/// (their space stays empty so the neighbours keep honest proportions).
pub fn sunburst(tree: &Tree, root: NodeId, key: SortKey, reverse: bool, max_depth: u32, min_angle: f64) -> Vec<ArcSeg> {
    let mut out = Vec::new();
    walk(tree, root, 0.0, TAU, 1, key, reverse, max_depth, min_angle, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn walk(
    tree: &Tree,
    id: NodeId,
    a0: f64,
    a1: f64,
    depth: u32,
    key: SortKey,
    reverse: bool,
    max_depth: u32,
    min_angle: f64,
    out: &mut Vec<ArcSeg>,
) {
    let parent = tree.node(id);
    if parent.size == 0 {
        return;
    }
    let mut a = a0;
    for child in tree.sorted_children(id, key, reverse) {
        let n = tree.node(child);
        let span = n.size as f64 / parent.size as f64 * (a1 - a0);
        if n.size == 0 || span < min_angle {
            a += span;
            continue;
        }
        out.push(ArcSeg { id: child, depth, a0: a, a1: a + span });
        if depth < max_depth && n.kind == Kind::Dir {
            walk(tree, child, a, a + span, depth + 1, key, reverse, max_depth, min_angle, out);
        }
        a += span;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NO_NODE, Node};

    fn area(r: &Rect) -> f64 {
        r.w * r.h
    }

    #[test]
    fn rectangles_tile_the_bounds_in_proportion() {
        let bounds = Rect { x: 10.0, y: 20.0, w: 800.0, h: 500.0 };
        let sizes = [60.0, 25.0, 10.0, 3.0, 2.0];
        let rects = squarify(&sizes, bounds);
        assert_eq!(rects.len(), sizes.len());
        let total_area: f64 = rects.iter().map(area).sum();
        assert!((total_area - area(&bounds)).abs() < 1e-6);
        for (r, s) in rects.iter().zip(sizes) {
            let expect = s / 100.0 * area(&bounds);
            assert!((area(r) - expect).abs() < 1e-6, "{r:?} vs {expect}");
            assert!(r.x >= bounds.x - 1e-9 && r.y >= bounds.y - 1e-9);
            assert!(r.x + r.w <= bounds.x + bounds.w + 1e-6 && r.y + r.h <= bounds.y + bounds.h + 1e-6);
        }
    }

    #[test]
    fn rectangles_do_not_overlap() {
        let rects = squarify(&[9.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0], Rect { x: 0.0, y: 0.0, w: 640.0, h: 400.0 });
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let ox = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let oy = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                assert!(ox <= 1e-6 || oy <= 1e-6, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn keeps_cards_close_to_square() {
        let sizes: Vec<f64> = (1..=30).rev().map(|n| n as f64).collect();
        let rects = squarify(&sizes, Rect { x: 0.0, y: 0.0, w: 900.0, h: 600.0 });
        let worst = rects.iter().map(|r| (r.w / r.h).max(r.h / r.w)).fold(0.0, f64::max);
        assert!(worst < 4.0, "worst aspect ratio {worst}");
    }

    #[test]
    fn empty_input_is_harmless() {
        assert!(squarify(&[], Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }).is_empty());
        assert_eq!(squarify(&[0.0], Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }), vec![Rect::ZERO]);
    }

    fn node(name: &str, parent: NodeId, size: u64, kind: Kind, children: Vec<NodeId>) -> Node {
        Node { name: name.into(), parent, children, size, apparent: size, items: 1, kind, mtime: 0, unreadable: false }
    }

    /// root(100) -> a(60, dir) -> [a1(40), a2(20)], b(30), c(10)
    fn sample() -> Tree {
        let nodes = vec![
            node("/", NO_NODE, 100, Kind::Dir, vec![1, 4, 5]),
            node("a", 0, 60, Kind::Dir, vec![2, 3]),
            node("a1", 1, 40, Kind::File, vec![]),
            node("a2", 1, 20, Kind::File, vec![]),
            node("b", 0, 30, Kind::File, vec![]),
            node("c", 0, 10, Kind::File, vec![]),
        ];
        Tree { nodes, root: 0, root_path: "/".into(), errors: 0, skipped: Vec::new() }
    }

    #[test]
    fn sunburst_angles_follow_sizes_and_nest() {
        let segs = sunburst(&sample(), 0, SortKey::Size, false, RING_DEPTH, 0.0);
        let a = segs.iter().find(|s| s.id == 1).unwrap();
        assert_eq!(a.depth, 1);
        assert!(((a.a1 - a.a0) - 0.6 * TAU).abs() < 1e-9);
        let a1 = segs.iter().find(|s| s.id == 2).unwrap();
        assert_eq!(a1.depth, 2);
        assert!(a1.a0 >= a.a0 - 1e-9 && a1.a1 <= a.a1 + 1e-9, "child stays inside parent");
        assert!(((a1.a1 - a1.a0) - 0.4 * TAU).abs() < 1e-9);
    }

    #[test]
    fn sunburst_drops_slivers_and_honours_depth() {
        let tree = sample();
        let segs = sunburst(&tree, 0, SortKey::Size, false, 1, 0.0);
        assert!(segs.iter().all(|s| s.depth == 1));
        let thin = sunburst(&tree, 0, SortKey::Size, false, RING_DEPTH, 0.25 * TAU);
        assert!(thin.iter().all(|s| s.id != 5 && s.id != 3), "10% and 20% of the circle fall under a 25% cutoff");
    }
}
