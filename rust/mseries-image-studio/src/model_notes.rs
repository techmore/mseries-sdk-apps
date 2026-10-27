//! Personal model assessments, deliberately separate from automated vision reviews.
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
struct Assessment {
    rating: Option<u8>,
    notes: String,
    updated_at_unix: u64,
}

type Assessments = BTreeMap<String, Assessment>;
fn identity(model: &str) -> String {
    std::fs::canonicalize(model)
        .unwrap_or_else(|_| PathBuf::from(model))
        .to_string_lossy()
        .into_owned()
}
fn read(base: &Path) -> Result<Assessments, String> {
    match std::fs::read(base.join("model-assessments.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Could not read model notes: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("Could not read model notes: {e}")),
    }
}
fn save(base: &Path, key: String, assessment: Assessment) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut entries = read(base)?;
    entries.insert(key, assessment);
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let temporary = base.join(format!(".model-assessments-{}.tmp", std::process::id()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec_pretty(&entries).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, base.join("model-assessments.json")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Personal assessment editor for one exact installed model path.
pub fn model_notes_editor(base: PathBuf, model_id: String) -> gtk::Box {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let title = gtk::Label::new(Some("Your model rating & notes"));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    panel.append(&title);
    let help = gtk::Label::new(Some(
        "Your assessment of this model. Saved locally, separately from automated image scores.",
    ));
    help.set_wrap(true);
    help.set_xalign(0.0);
    panel.append(&help);
    let rating = gtk::DropDown::from_strings(&[
        "Not rated",
        "1 — Poor",
        "2 — Fair",
        "3 — Good",
        "4 — Very good",
        "5 — Excellent",
    ]);
    rating.set_tooltip_text(Some("Your personal model rating"));
    panel.append(&rating);
    let notes = gtk::TextView::new();
    notes.set_wrap_mode(gtk::WrapMode::WordChar);
    notes.set_top_margin(8);
    notes.set_bottom_margin(8);
    notes.set_left_margin(8);
    notes.set_right_margin(8);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(110)
        .child(&notes)
        .build();
    scroll.add_css_class("frame");
    panel.append(&scroll);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    let key = identity(&model_id);
    match read(&base) {
        Ok(entries) => {
            if let Some(entry) = entries.get(&key) {
                rating
                    .set_selected(entry.rating.filter(|v| (1..=5).contains(v)).unwrap_or(0) as u32);
                notes.buffer().set_text(&entry.notes);
            }
        }
        Err(error) => status.set_text(&error),
    }
    let button = gtk::Button::with_label("Save my assessment");
    button.set_halign(gtk::Align::Start);
    panel.append(&status);
    button.connect_clicked(move |_| {
        let buffer = notes.buffer();
        let selected = rating.selected();
        let assessment = Assessment {
            rating: if (1..=5).contains(&selected) {
                Some(selected as u8)
            } else {
                None
            },
            notes: buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string(),
            updated_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        match save(&base, key.clone(), assessment) {
            Ok(()) => status.set_text("Your assessment is saved."),
            Err(error) => status.set_text(&format!("Assessment was not saved: {error}")),
        }
    });
    panel.append(&button);
    // The callback owns a reference; retain the visible status label as well.
    // GTK object clones refer to the same widget.
    panel
}
