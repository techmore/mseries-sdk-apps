//! Installed applications (from `.desktop` files) and the resolver that maps a window's `app-id` to one of them.
//!
//! Rules (from `mseries/DOCK-APP-TRACKING.md`): match the desktop id with or without `.desktop`, then `StartupWMClass`,
//! then a unique executable basename. Anything ambiguous matches nothing rather than an arbitrary launcher. A window that
//! matches nothing becomes a temporary, unpinnable entry; the dock never runs a command a window supplied.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Desktop id with the `.desktop` suffix, for example `io.elementary.files.desktop`.
    pub id: String,
    pub name: String,
    pub icon: String,
    pub wm_class: Option<String>,
    /// Basename of the first word of `Exec`, for example `brave-browser`.
    pub exec_base: Option<String>,
}

impl AppEntry {
    pub fn short_id(&self) -> &str {
        self.id.strip_suffix(".desktop").unwrap_or(&self.id)
    }
}

/// Parses the `[Desktop Entry]` section of a `.desktop` file. Hidden and non-application entries return None.
pub fn parse_desktop(id: &str, text: &str) -> Option<AppEntry> {
    let (mut in_entry, mut kv) = (false, std::collections::HashMap::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && let Some((k, v)) = line.split_once('=') {
            // Unlocalised keys only: `Name=` wins over `Name[de]=`.
            if !k.contains('[') {
                kv.entry(k.trim().to_string())
                    .or_insert_with(|| v.trim().to_string());
            }
        }
    }
    if kv.get("Type").map(String::as_str) != Some("Application")
        || kv.get("NoDisplay").is_some_and(|v| v == "true")
        || kv.get("Hidden").is_some_and(|v| v == "true")
    {
        return None;
    }
    let exec_base = kv
        .get("Exec")
        .and_then(|e| e.split_whitespace().next())
        .map(|w| w.rsplit('/').next().unwrap_or(w).to_string());
    Some(AppEntry {
        id: id.to_string(),
        name: kv.get("Name")?.clone(),
        icon: kv
            .get("Icon")
            .cloned()
            .unwrap_or_else(|| "application-x-executable".into()),
        wm_class: kv.get("StartupWMClass").cloned(),
        exec_base,
    })
}

/// Reads every application in the XDG data dirs (user dir first, so a user entry overrides a system one).
pub fn load_installed() -> Vec<AppEntry> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    dirs.extend(home);
    let sys =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    let mut out: Vec<AppEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in dirs {
        read_dir(&d.join("applications"), &mut out, &mut seen);
    }
    out
}

fn read_dir(dir: &Path, out: &mut Vec<AppEntry>, seen: &mut std::collections::HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "desktop"))
        .collect();
    files.sort();
    for p in files {
        let Some(id) = p.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        // Reserve even hidden ids: user overrides suppress system launchers.
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&p)
            && let Some(e) = parse_desktop(&id, &text)
        {
            out.push(e);
        }
    }
}

fn unique(hits: Vec<&AppEntry>) -> Option<&AppEntry> {
    if hits.len() == 1 { Some(hits[0]) } else { None }
}

pub struct Resolver {
    pub entries: Vec<AppEntry>,
}

impl Resolver {
    pub fn new(entries: Vec<AppEntry>) -> Resolver {
        Resolver { entries }
    }

    pub fn by_id(&self, id: &str) -> Option<&AppEntry> {
        let want = id.strip_suffix(".desktop").unwrap_or(id);
        self.entries.iter().find(|e| e.short_id() == want)
    }

    /// The installed app a window belongs to, or None when unknown or ambiguous.
    pub fn resolve(&self, app_id: &str) -> Option<&AppEntry> {
        if app_id.is_empty() {
            return None;
        }
        if let Some(e) = self.by_id(app_id) {
            return Some(e);
        }
        let lower = app_id.to_lowercase();
        // Case-folded id, only when exactly one entry matches.
        if let Some(e) = unique(
            self.entries
                .iter()
                .filter(|e| e.short_id().to_lowercase() == lower)
                .collect(),
        ) {
            return Some(e);
        }
        if self.entries.iter().any(|e| {
            e.wm_class
                .as_deref()
                .is_some_and(|c| c.to_lowercase() == lower)
        }) {
            return unique(
                self.entries
                    .iter()
                    .filter(|e| {
                        e.wm_class
                            .as_deref()
                            .is_some_and(|c| c.to_lowercase() == lower)
                    })
                    .collect(),
            );
        }
        unique(
            self.entries
                .iter()
                .filter(|e| e.exec_base.as_deref().is_some_and(|b| b == app_id))
                .collect(),
        )
    }
}
