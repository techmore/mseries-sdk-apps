//! Removing things, and deciding what must never be removed from here.

use crate::model::{Kind, NodeId, Tree};
use std::path::{Path, PathBuf};

pub struct Removal {
    pub id: NodeId,
    pub size: u64,
    pub result: Result<(), String>,
}

/// Delete each path for good (files, symlinks and whole folders). Runs on a worker thread.
pub fn delete_all(items: Vec<(NodeId, PathBuf, u64)>) -> Vec<Removal> {
    items
        .into_iter()
        .map(|(id, path, size)| {
            let result = match std::fs::symlink_metadata(&path) {
                Ok(md) if md.is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(e) => Err(e),
            };
            Removal { id, size, result: result.map_err(|e| e.to_string()) }
        })
        .collect()
}

/// Things no one should remove from here: the scan root, `/`, top-level system folders, the home folder
/// and anything that contains it, and other filesystems.
pub fn is_protected(tree: &Tree, id: NodeId, home: &Path) -> bool {
    let n = tree.node(id);
    if id == tree.root || n.kind == Kind::Mount {
        return true;
    }
    let path = tree.path(id);
    path == Path::new("/") || path.parent() == Some(Path::new("/")) || home.starts_with(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Progress, scan_blocking};
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mseries-diskspace-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deletes_files_folders_and_symlinks_and_reports_failures() {
        let dir = scratch("delete");
        fs::create_dir_all(dir.join("folder/deep")).unwrap();
        fs::write(dir.join("folder/deep/a.bin"), vec![1u8; 5000]).unwrap();
        fs::write(dir.join("file.bin"), vec![1u8; 5000]).unwrap();
        std::os::unix::fs::symlink(dir.join("file.bin"), dir.join("link")).unwrap();
        let results =
            delete_all(vec![(1, dir.join("folder"), 10), (2, dir.join("link"), 0), (3, dir.join("missing"), 0)]);
        assert!(results[0].result.is_ok() && results[1].result.is_ok());
        assert!(results[2].result.is_err());
        assert!(!dir.join("folder").exists());
        assert!(!dir.join("link").exists());
        assert!(dir.join("file.bin").exists(), "deleting the link must not touch its target");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn protects_the_root_system_folders_and_home() {
        let dir = scratch("protect");
        fs::create_dir_all(dir.join("home/me/docs")).unwrap();
        fs::write(dir.join("home/me/docs/a.txt"), b"x").unwrap();
        let tree = scan_blocking(&dir, false, &Progress::default()).unwrap();
        let find = |from: NodeId, name: &str| {
            tree.node(from).children.iter().copied().find(|c| &*tree.node(*c).name == name).unwrap()
        };
        let home_dir = find(find(tree.root, "home"), "me");
        let docs = find(home_dir, "docs");
        let home = dir.join("home/me");
        assert!(is_protected(&tree, tree.root, &home), "the scan root");
        assert!(is_protected(&tree, home_dir, &home), "the home folder itself");
        assert!(is_protected(&tree, find(tree.root, "home"), &home), "a folder that contains home");
        assert!(!is_protected(&tree, docs, &home), "a folder inside home");
        fs::remove_dir_all(&dir).unwrap();
    }
}
