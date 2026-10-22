//! Where the Claude Code CLI lives on this machine: the copy bundled with the desktop app
//! (`~/.config/Claude/claude-code/<version>/<hash>/claude`, newest version wins), else `claude` on PATH.

use std::path::PathBuf;

pub fn find() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let base = home.join(".config/Claude/claude-code");
    let mut best: Option<(Vec<u64>, std::time::SystemTime, PathBuf)> = None;
    if let Ok(versions) = std::fs::read_dir(&base) {
        for v in versions.flatten() {
            let key: Vec<u64> = v.file_name().to_string_lossy().split('.').map(|p| p.parse().unwrap_or(0)).collect();
            let Ok(hashes) = std::fs::read_dir(v.path()) else { continue };
            for h in hashes.flatten() {
                let candidate = h.path().join("claude");
                let Ok(meta) = std::fs::metadata(&candidate) else { continue };
                let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                let better = match &best {
                    None => true,
                    Some((k, m, _)) => (&key, mtime) > (k, *m),
                };
                if better {
                    best = Some((key.clone(), mtime, candidate));
                }
            }
        }
    }
    if let Some((_, _, p)) = best {
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("claude")).find(|p| p.is_file())
}
