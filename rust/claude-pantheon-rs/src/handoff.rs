//! "Hand off to repo": the user's durable cross-agent memory is the repository itself (handoff docs,
//! AGENTS.md, PR descriptions), not T3's 240-character provider handoffs. We ask the thread's
//! provider for a structured summary, append it to the project's handoff document and offer a commit.

use crate::app::App;
use gtk::{gio, glib};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const PROMPT: &str = "Write a handoff summary for the next agent working in this repository, as Markdown \
with exactly these four headings: '## What was done', '## What is open', '## Files touched', '## Decisions'. \
Be concrete (paths, commands, PR numbers). Do not run tools; answer from this conversation only.";

pub struct Pending {
    pub thread_id: String,
    pub root: PathBuf,
    pub doc: PathBuf,
}

/// The handoff document of a project: an existing `*HANDOFF*.md` or `AGENTS.md` at the root,
/// otherwise `docs/HANDOFF.md`.
pub fn doc_path(root: &Path) -> PathBuf {
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.to_uppercase().contains("HANDOFF") && n.to_lowercase().ends_with(".md"))
            .collect();
        names.sort();
        if let Some(n) = names.first() {
            return root.join(n);
        }
    }
    // mseries-style repos keep it under mseries/.
    if let Ok(entries) = std::fs::read_dir(root.join("mseries")) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| n.to_uppercase().contains("HANDOFF") && n.to_lowercase().ends_with(".md"))
            .collect();
        names.sort();
        if let Some(n) = names.first() {
            return root.join("mseries").join(n);
        }
    }
    if root.join("AGENTS.md").exists() {
        return root.join("AGENTS.md");
    }
    root.join("docs").join("HANDOFF.md")
}

/// The section appended to the document.
pub fn section(thread_id: &str, summary: &str) -> String {
    let now =
        glib::DateTime::now_local().and_then(|d| d.format("%Y-%m-%d %H:%M")).map(|s| s.to_string()).unwrap_or_default();
    format!("\n\n## Handoff {now} (thread {thread_id})\n\n{}\n", summary.trim())
}

pub fn append(doc: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = doc.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut existing = std::fs::read_to_string(doc).unwrap_or_default();
    if existing.is_empty() {
        existing.push_str("# Handoff\n\nNotes for the next agent working in this repository.\n");
    }
    existing.push_str(text);
    std::fs::write(doc, existing)
}

/// Writes the summary and asks whether to commit it.
pub fn finish(app: &Rc<App>, pending: Pending, summary: String) {
    let text = section(&pending.thread_id, &summary);
    if let Err(e) = append(&pending.doc, &text) {
        app.toast(&format!("Could not write the handoff: {e}"));
        return;
    }
    let rel = pending.doc.strip_prefix(&pending.root).unwrap_or(&pending.doc).to_string_lossy().into_owned();
    let dialog = gtk::AlertDialog::builder()
        .message("Handoff written")
        .detail(format!("The summary was appended to {rel}. Commit it now?"))
        .buttons(["Leave Uncommitted", "Commit"])
        .default_button(1)
        .cancel_button(0)
        .build();
    let app2 = app.clone();
    let root = pending.root.clone();
    dialog.choose(Some(app.window()), gio::Cancellable::NONE, move |response| {
        if response != Ok(1) {
            return;
        }
        let app = app2.clone();
        let root = root.clone();
        let rel = rel.clone();
        glib::spawn_future_local(async move {
            let launcher =
                gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE);
            launcher.set_cwd(&root);
            let message = format!(
                "Handoff: {}",
                glib::DateTime::now_local()
                    .and_then(|d| d.format("%Y-%m-%d %H:%M"))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            );
            let argv: Vec<std::ffi::OsString> = ["git", "add", "--", rel.as_str()].iter().map(Into::into).collect();
            let refs: Vec<&std::ffi::OsStr> = argv.iter().map(AsRef::as_ref).collect();
            if let Ok(p) = launcher.spawn(&refs) {
                let _ = p.wait_future().await;
            }
            let argv: Vec<std::ffi::OsString> =
                ["git", "commit", "-m", message.as_str(), "--", rel.as_str()].iter().map(Into::into).collect();
            let refs: Vec<&std::ffi::OsStr> = argv.iter().map(AsRef::as_ref).collect();
            match launcher.spawn(&refs) {
                Ok(p) => match p.communicate_utf8_future(None).await {
                    Ok(_) if p.has_exited() && p.exit_status() == 0 => app.toast(&format!("Committed {rel}")),
                    Ok((out, _)) => {
                        app.toast(&format!("git commit: {}", out.map(|s| s.to_string()).unwrap_or_default().trim()))
                    }
                    Err(e) => app.toast(&format!("git commit failed: {e}")),
                },
                Err(e) => app.toast(&format!("git not available: {e}")),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_handoff_doc() {
        let dir = std::env::temp_dir().join(format!("handoff-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(doc_path(&dir), dir.join("docs").join("HANDOFF.md"));
        std::fs::write(dir.join("AGENTS.md"), "x").unwrap();
        assert_eq!(doc_path(&dir), dir.join("AGENTS.md"));
        std::fs::write(dir.join("OS9-HANDOFF.md"), "x").unwrap();
        assert_eq!(doc_path(&dir), dir.join("OS9-HANDOFF.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
