//! Parallel filesystem scan. Measures allocated blocks (what `df` and `du` count), never follows
//! symlinks, counts a hard-linked file once, and stays on one filesystem unless asked not to.

use crate::model::{Kind, NO_NODE, Node, NodeId, Tree};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

/// Pseudo filesystems that are never worth walking, even when crossing filesystems.
const SKIP: &[&str] = &["/proc", "/sys", "/dev"];

#[derive(Default)]
pub struct Progress {
    pub items: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
}

pub enum ScanMsg {
    Done(Tree, Duration),
    Cancelled,
}

struct Entry {
    name: Box<str>,
    kind: Kind,
    size: u64,
    apparent: u64,
    items: u64,
    mtime: i64,
    unreadable: bool,
    children: Vec<Entry>,
}

struct Ctx<'a> {
    dev: u64,
    cross_fs: bool,
    progress: &'a Progress,
    hardlinks: Mutex<HashSet<(u64, u64)>>,
    skipped: Mutex<Vec<PathBuf>>,
}

pub fn spawn(root: PathBuf, cross_fs: bool, progress: std::sync::Arc<Progress>, tx: mpsc::Sender<ScanMsg>) {
    std::thread::spawn(move || {
        let start = Instant::now();
        let tree = scan_blocking(&root, cross_fs, &progress);
        let msg = match tree {
            Some(tree) if !progress.cancel.load(Relaxed) => ScanMsg::Done(tree, start.elapsed()),
            _ => ScanMsg::Cancelled,
        };
        let _ = tx.send(msg);
    });
}

/// Scan `root` to completion on a private thread pool. `None` when the root is unreadable.
pub fn scan_blocking(root: &Path, cross_fs: bool, progress: &Progress) -> Option<Tree> {
    let meta = fs::metadata(root).ok()?;
    if !meta.is_dir() {
        return None;
    }
    // The walk is I/O bound, so run more workers than cores, and give deep trees room to recurse.
    let threads = (std::thread::available_parallelism().map_or(4, |n| n.get()) * 2).clamp(4, 16);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).stack_size(16 << 20).build().ok()?;
    let ctx = Ctx {
        dev: meta.dev(),
        cross_fs,
        progress,
        hardlinks: Mutex::new(HashSet::new()),
        skipped: Mutex::new(Vec::new()),
    };
    let entry = pool.install(|| scan_dir(root, root.to_string_lossy().into(), &meta, &ctx));
    let mut nodes = Vec::new();
    flatten(&mut nodes, entry, NO_NODE);
    let mut skipped = ctx.skipped.into_inner().unwrap_or_default();
    skipped.sort();
    Some(Tree { nodes, root: 0, root_path: root.to_path_buf(), errors: progress.errors.load(Relaxed), skipped })
}

fn scan_dir(path: &Path, name: Box<str>, meta: &Metadata, ctx: &Ctx) -> Entry {
    let own = meta.blocks() * 512;
    let mut dir = Entry {
        name,
        kind: Kind::Dir,
        size: own,
        apparent: meta.len(),
        items: 1,
        mtime: meta.mtime(),
        unreadable: false,
        children: Vec::new(),
    };
    ctx.progress.items.fetch_add(1, Relaxed);
    ctx.progress.bytes.fetch_add(own, Relaxed);
    if ctx.progress.cancel.load(Relaxed) {
        return dir;
    }
    if let Ok(mut current) = ctx.progress.current.try_lock() {
        current.clear();
        current.push_str(&path.to_string_lossy());
    }
    let read = match fs::read_dir(path) {
        Ok(read) => read,
        Err(_) => {
            dir.unreadable = true;
            ctx.progress.errors.fetch_add(1, Relaxed);
            return dir;
        }
    };
    let mut subdirs: Vec<(PathBuf, Box<str>, Metadata)> = Vec::new();
    for entry in read {
        let Ok(entry) = entry else {
            ctx.progress.errors.fetch_add(1, Relaxed);
            continue;
        };
        // `DirEntry::metadata` is an lstat: symlinks are reported, not followed.
        let Ok(md) = entry.metadata() else {
            ctx.progress.errors.fetch_add(1, Relaxed);
            continue;
        };
        let name: Box<str> = entry.file_name().to_string_lossy().into();
        let ft = md.file_type();
        if ft.is_dir() {
            let child = entry.path();
            let other_fs = md.dev() != ctx.dev;
            let pseudo = SKIP.iter().any(|s| child == Path::new(s));
            if pseudo || (other_fs && !ctx.cross_fs) {
                if !pseudo && let Ok(mut skipped) = ctx.skipped.lock() {
                    skipped.push(entry.path());
                }
                ctx.progress.items.fetch_add(1, Relaxed);
                dir.children.push(Entry {
                    name,
                    kind: Kind::Mount,
                    size: 0,
                    apparent: 0,
                    items: 1,
                    mtime: md.mtime(),
                    unreadable: false,
                    children: Vec::new(),
                });
            } else {
                subdirs.push((child, name, md));
            }
            continue;
        }
        let kind = if ft.is_symlink() {
            Kind::Symlink
        } else if ft.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        let mut size = md.blocks() * 512;
        if kind == Kind::File && md.nlink() > 1 {
            // A hard-linked file occupies its blocks once, however many names it has.
            let first = ctx.hardlinks.lock().map_or(true, |mut seen| seen.insert((md.dev(), md.ino())));
            if !first {
                size = 0;
            }
        }
        ctx.progress.items.fetch_add(1, Relaxed);
        ctx.progress.bytes.fetch_add(size, Relaxed);
        dir.children.push(Entry {
            name,
            kind,
            size,
            apparent: md.len(),
            items: 1,
            mtime: md.mtime(),
            unreadable: false,
            children: Vec::new(),
        });
    }
    let scanned: Vec<Entry> = subdirs.into_par_iter().map(|(p, n, md)| scan_dir(&p, n, &md, ctx)).collect();
    dir.children.extend(scanned);
    for child in &dir.children {
        dir.size += child.size;
        dir.apparent += child.apparent;
        dir.items += child.items;
    }
    dir.children.sort_by_key(|c| std::cmp::Reverse(c.size));
    dir
}

fn flatten(nodes: &mut Vec<Node>, entry: Entry, parent: NodeId) -> NodeId {
    let id = nodes.len() as NodeId;
    nodes.push(Node {
        name: entry.name,
        parent,
        children: Vec::new(),
        size: entry.size,
        apparent: entry.apparent,
        items: entry.items,
        kind: entry.kind,
        mtime: entry.mtime,
        unreadable: entry.unreadable,
    });
    let kids: Vec<NodeId> = entry.children.into_iter().map(|c| flatten(nodes, c, id)).collect();
    nodes[id as usize].children = kids;
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().write_all(&vec![7u8; bytes]).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mseries-diskspace-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sums_children_and_orders_largest_first() {
        let dir = scratch("sum");
        write(&dir.join("big/a.bin"), 400_000);
        write(&dir.join("big/deep/b.bin"), 200_000);
        write(&dir.join("small.txt"), 10);
        let tree = scan_blocking(&dir, false, &Progress::default()).unwrap();
        let root = tree.node(tree.root);
        assert!(root.size >= 600_000, "size {}", root.size);
        assert!(tree.skipped.is_empty());
        assert!(root.apparent >= 600_010);
        let first = tree.node(root.children[0]);
        assert_eq!(&*first.name, "big");
        assert!(first.size >= 600_000);
        // 1 root + big + a.bin + deep + b.bin + small.txt
        assert_eq!(root.items, 6);
        let sizes: Vec<u64> = root.children.iter().map(|c| tree.node(*c).size).collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]));
        let deep = tree.node(
            tree.node(root.children[0]).children.iter().copied().find(|c| &*tree.node(*c).name == "deep").unwrap(),
        );
        assert_eq!(tree.path(deep.children[0]), dir.join("big/deep/b.bin"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn counts_a_hard_link_once() {
        let dir = scratch("hardlink");
        write(&dir.join("one.bin"), 300_000);
        fs::hard_link(dir.join("one.bin"), dir.join("two.bin")).unwrap();
        let tree = scan_blocking(&dir, false, &Progress::default()).unwrap();
        let root = tree.node(tree.root);
        assert!(root.size < 450_000, "counted twice: {}", root.size);
        assert!(root.size >= 300_000);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn does_not_follow_symlinks() {
        let dir = scratch("symlink");
        write(&dir.join("real/a.bin"), 300_000);
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let tree = scan_blocking(&dir, false, &Progress::default()).unwrap();
        let root = tree.node(tree.root);
        let link = root.children.iter().map(|c| tree.node(*c)).find(|n| &*n.name == "link").unwrap();
        assert_eq!(link.kind, Kind::Symlink);
        assert!(root.size < 450_000);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_unreadable_directories() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("denied");
        write(&dir.join("locked/a.bin"), 1000);
        fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let progress = Progress::default();
        let tree = scan_blocking(&dir, false, &progress).unwrap();
        let readable_anyway = fs::read_dir(dir.join("locked")).is_ok(); // running as root
        if !readable_anyway {
            assert_eq!(tree.errors, 1);
            let locked = tree.node(tree.node(tree.root).children[0]);
            assert!(locked.unreadable);
        }
        fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_a_node_updates_every_ancestor() {
        let dir = scratch("remove");
        write(&dir.join("a/x.bin"), 500_000);
        write(&dir.join("a/y.bin"), 100_000);
        write(&dir.join("b/z.bin"), 300_000);
        let mut tree = scan_blocking(&dir, false, &Progress::default()).unwrap();
        let before = tree.node(tree.root).size;
        let a = tree.node(tree.root).children.iter().copied().find(|c| &*tree.node(*c).name == "a").unwrap();
        let x = tree.node(a).children[0];
        let x_size = tree.node(x).size;
        let items = tree.node(tree.root).items;
        tree.remove(x);
        assert_eq!(tree.node(tree.root).size, before - x_size);
        assert_eq!(tree.node(tree.root).items, items - 1);
        let sizes: Vec<u64> = tree.node(tree.root).children.iter().map(|c| tree.node(*c).size).collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "re-sorted after removal");
        fs::remove_dir_all(&dir).unwrap();
    }
}
