//! The session index: every transcript under `~/.claude/projects`, with its title, project and last
//! change. Files are scanned in parallel with rayon; results are cached by (mtime, size) under
//! `~/.cache/claude-pantheon-rs/index.json` so later launches only re-read sessions that changed.

use crate::transcript::{self, Content, content_text, title_from_prompt};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub path: PathBuf,
    /// The session uuid (file stem), what `claude --resume` takes.
    pub id: String,
    /// Working directory of the session, from its first record; falls back to the folder slug.
    pub cwd: String,
    pub title: String,
    /// Unix seconds of the last change.
    pub mtime: i64,
    pub size: u64,
}

impl Session {
    /// The project a session belongs to, as the sidebar groups it: the last path component of its cwd.
    pub fn project(&self) -> String {
        let cwd = self.cwd.trim_end_matches('/');
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() && cwd == home {
            return "Home".into();
        }
        cwd.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(cwd).to_string()
    }
}

pub fn projects_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    home.join(".claude").join("projects")
}

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".cache"));
    base.join("claude-pantheon-rs")
}

fn cache_file() -> PathBuf {
    cache_dir().join("index.json")
}

fn slug_to_path(slug: &str) -> String {
    slug.replace('-', "/")
}

/// One scan of one file: the first real prompt gives the title unless a summary or custom title exists.
fn scan(path: &Path, slug: &str) -> Option<(String, String)> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(128 * 1024, file);
    let mut line = String::new();
    let mut cwd: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut named: Option<String> = None;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let is_user = line.contains("\"type\":\"user\"");
        let is_named = line.contains("\"type\":\"custom-title\"") || line.contains("\"type\":\"summary\"");
        if !(is_user || is_named) {
            continue;
        }
        let Some(rec) = transcript::parse_line(&line) else { continue };
        if cwd.is_none() && rec.cwd.is_some() {
            cwd = rec.cwd.clone();
        }
        if let Some(t) = rec.custom_title.or(rec.summary) {
            named = Some(t);
            continue;
        }
        if prompt.is_none()
            && rec.kind == "user"
            && !rec.is_meta
            && !rec.is_sidechain
            && let Some(msg) = &rec.message
        {
            let text = match &msg.content {
                Content::Text(s) => s.clone(),
                other => content_text(other),
            };
            prompt = title_from_prompt(&text);
        }
    }
    let title = named.or(prompt)?;
    Some((cwd.unwrap_or_else(|| slug_to_path(slug)), title))
}

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    entries: HashMap<String, Session>,
}

fn list_files() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(projects_dir()) else { return out };
    for project in projects.flatten() {
        let Ok(kind) = project.file_type() else { continue };
        if !kind.is_dir() {
            continue;
        }
        let slug = project.file_name().to_string_lossy().into_owned();
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "jsonl") {
                out.push((p, slug.clone()));
            }
        }
    }
    out
}

/// All sessions, newest first. Sessions without a single real prompt are left out.
pub fn index_all() -> Vec<Session> {
    let cache: Cache =
        std::fs::read(cache_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let files = list_files();
    let scanned: Vec<Option<Session>> = files
        .par_iter()
        .map(|(path, slug)| {
            let meta = std::fs::metadata(path).ok()?;
            let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
            let size = meta.len();
            let key = path.to_string_lossy().into_owned();
            if let Some(hit) = cache.entries.get(&key)
                && hit.mtime == mtime
                && hit.size == size
            {
                return Some(hit.clone());
            }
            let (cwd, title) = scan(path, slug)?;
            let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Some(Session { path: path.clone(), id, cwd, title, mtime, size })
        })
        .collect();
    let mut sessions: Vec<Session> = scanned.into_iter().flatten().collect();
    sessions.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));

    let fresh =
        Cache { entries: sessions.iter().map(|s| (s.path.to_string_lossy().into_owned(), s.clone())).collect() };
    let changed = fresh.entries.len() != cache.entries.len()
        || fresh
            .entries
            .iter()
            .any(|(k, v)| cache.entries.get(k).is_none_or(|c| c.mtime != v.mtime || c.size != v.size));
    if changed
        && std::fs::create_dir_all(cache_dir()).is_ok()
        && let Ok(bytes) = serde_json::to_vec(&fresh)
    {
        let _ = std::fs::write(cache_file(), bytes);
    }
    sessions
}
