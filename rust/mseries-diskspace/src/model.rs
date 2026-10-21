//! The scanned tree, stored as an arena so a node id stays valid while the view re-sorts.

use std::collections::BTreeSet;
use std::path::PathBuf;

pub type NodeId = u32;
pub const NO_NODE: NodeId = NodeId::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    Other,
    /// A directory on another filesystem. Listed, never entered, never counted.
    Mount,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Size,
    Name,
    Items,
    Modified,
}

pub struct Node {
    pub name: Box<str>,
    pub parent: NodeId,
    /// Always kept largest first, so views can take the head without sorting.
    pub children: Vec<NodeId>,
    /// Disk usage (allocated blocks) including everything below.
    pub size: u64,
    /// Sum of file lengths including everything below.
    pub apparent: u64,
    /// Entries in this subtree, itself included.
    pub items: u64,
    pub kind: Kind,
    pub mtime: i64,
    /// A directory we were not allowed to read.
    pub unreadable: bool,
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub root_path: PathBuf,
    /// Directories that could not be read during the scan.
    pub errors: u64,
    /// Other filesystems (mounts, Btrfs subvolumes) met on the way and left out of the totals.
    pub skipped: Vec<PathBuf>,
}

impl Tree {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn is_dir(&self, id: NodeId) -> bool {
        self.node(id).kind == Kind::Dir
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != self.root && cur != NO_NODE {
            let n = self.node(cur);
            parts.push(&*n.name);
            cur = n.parent;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// Root first, `id` last.
    pub fn crumbs(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        let mut cur = self.node(id).parent;
        while cur != NO_NODE {
            out.push(cur);
            cur = self.node(cur).parent;
        }
        out.reverse();
        out
    }

    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cur = self.node(id).parent;
        while cur != NO_NODE {
            if cur == ancestor {
                return true;
            }
            cur = self.node(cur).parent;
        }
        false
    }

    /// Detach `id` and take its weight off every ancestor. The arena slot stays, orphaned.
    pub fn remove(&mut self, id: NodeId) {
        let (size, apparent, items, parent) = {
            let n = self.node(id);
            (n.size, n.apparent, n.items, n.parent)
        };
        if parent == NO_NODE {
            return;
        }
        self.nodes[parent as usize].children.retain(|c| *c != id);
        let mut cur = parent;
        while cur != NO_NODE {
            let n = &mut self.nodes[cur as usize];
            n.size = n.size.saturating_sub(size);
            n.apparent = n.apparent.saturating_sub(apparent);
            n.items = n.items.saturating_sub(items);
            let next = n.parent;
            let kids = std::mem::take(&mut self.nodes[cur as usize].children);
            let mut kids = kids;
            kids.sort_by_key(|k| std::cmp::Reverse(self.nodes[*k as usize].size));
            self.nodes[cur as usize].children = kids;
            cur = next;
        }
    }

    /// Natural order: largest, most items and newest first, names A to Z. `reverse` flips it.
    pub fn sorted_children(&self, id: NodeId, key: SortKey, reverse: bool) -> Vec<NodeId> {
        let mut kids = self.node(id).children.clone();
        match key {
            SortKey::Size => {} // already largest first
            SortKey::Name => kids.sort_by_cached_key(|k| self.node(*k).name.to_lowercase()),
            SortKey::Items => kids.sort_by_key(|k| std::cmp::Reverse(self.node(*k).items)),
            SortKey::Modified => kids.sort_by_key(|k| std::cmp::Reverse(self.node(*k).mtime)),
        }
        if reverse {
            kids.reverse();
        }
        kids
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Cards,
    Circle,
}

pub struct ScanRun {
    pub generation: u64,
    pub path: PathBuf,
    pub progress: std::sync::Arc<crate::scan::Progress>,
}

pub struct Model {
    pub tree: Option<Tree>,
    pub cwd: NodeId,
    pub selected: BTreeSet<NodeId>,
    pub hover: Option<NodeId>,
    pub sort: SortKey,
    /// Flip the sort key's natural order.
    pub reverse: bool,
    pub mode: Mode,
    pub cross_fs: bool,
    pub scan: Option<ScanRun>,
    pub generation: u64,
    pub scan_secs: f64,
    /// The folder the current tree (or running scan) came from, for Rescan.
    pub target: Option<PathBuf>,
    pub rings: Vec<crate::layout::ArcSeg>,
}

impl Model {
    pub fn new() -> Self {
        Model {
            tree: None,
            cwd: 0,
            selected: BTreeSet::new(),
            hover: None,
            sort: SortKey::Size,
            reverse: false,
            mode: Mode::Cards,
            cross_fs: false,
            scan: None,
            generation: 0,
            scan_secs: 0.0,
            target: None,
            rings: Vec::new(),
        }
    }

    pub fn sorted_children(&self, id: NodeId) -> Vec<NodeId> {
        let tree = self.tree.as_ref().expect("tree");
        tree.sorted_children(id, self.sort, self.reverse)
    }

    pub fn recompute_rings(&mut self) {
        self.rings = match &self.tree {
            Some(tree) if self.mode == Mode::Circle => {
                crate::layout::sunburst(tree, self.cwd, self.sort, self.reverse, crate::layout::RING_DEPTH, 0.004)
            }
            _ => Vec::new(),
        };
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}
