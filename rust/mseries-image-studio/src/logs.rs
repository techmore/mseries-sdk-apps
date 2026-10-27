//! Bounded, read-only structured render diagnostics.
use gtk::prelude::*;
use std::{
    cell::RefCell,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    rc::Rc,
};

fn read_tail(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let offset = size.saturating_sub(65_536);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(65_536).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(if offset > 0 {
        text.split_once('\n')
            .map(|(_, tail)| tail.to_owned())
            .unwrap_or_default()
    } else {
        text
    })
}

fn readable(path: &Path) -> String {
    let text = match read_tail(path) {
        Ok(text) => text,
        Err(error) => return format!("Could not read this run: {error}"),
    };
    let mut output = String::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let elapsed = value
            .get("elapsed_seconds")
            .and_then(|v| v.as_f64())
            .unwrap_or_default();
        let event = value
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("event");
        output.push_str(&format!("{elapsed:>7.1}s   {}\n", event.replace('_', " ")));
        // Explicit allowlist: raw arguments, prompt text and arbitrary errors do not enter the viewer.
        for key in [
            "status",
            "preview_count",
            "image_path",
            "output",
            "MemAvailable_bytes",
            "VmRSS_bytes",
            "VmHWM_bytes",
        ] {
            if let Some(v) = value.get(key) {
                let display = if key.ends_with("_bytes") {
                    v.as_u64()
                        .map(|n| format!("{:.0} MiB", n as f64 / 1_048_576.0))
                        .unwrap_or_else(|| v.to_string())
                } else {
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                };
                output.push_str(&format!(
                    "    {}: {display}\n",
                    key.trim_end_matches("_bytes").replace('_', " ")
                ));
            }
        }
        if event == "assets" {
            for key in ["model", "vae", "engine"] {
                if let Some(asset) = value.get(key).filter(|v| !v.is_null()) {
                    let filename = asset
                        .get("path")
                        .and_then(|v| v.as_str())
                        .and_then(|p| Path::new(p).file_name())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    output.push_str(&format!("    {key}: {filename}\n"));
                }
            }
        }
        output.push('\n');
    }
    if output.is_empty() {
        "Waiting for structured events…".into()
    } else {
        output
    }
}

pub fn page(base: PathBuf) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.set_margin_top(22);
    page.set_margin_bottom(24);
    page.set_margin_start(26);
    page.set_margin_end(26);
    page.append(&crate::style::page_header(
        "Run logs",
        "Model, preview progress and measured memory for each render. Private prompt text stays in the saved recipe. Showing the latest 64 KiB of each event log.",
    ));
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let list = gtk::StringList::new(&[]);
    let dropdown = gtk::DropDown::new(Some(list.clone()), None::<gtk::Expression>);
    dropdown.set_hexpand(true);
    dropdown.set_tooltip_text(Some("Choose a render log"));
    let refresh = gtk::Button::with_label("Refresh");
    refresh.add_css_class("is-pill");
    toolbar.append(&dropdown);
    toolbar.append(&refresh);
    page.append(&toolbar);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_monospace(true);
    view.set_left_margin(14);
    view.set_right_margin(14);
    view.set_top_margin(12);
    view.set_bottom_margin(12);
    view.buffer()
        .set_text("No render logs yet. Start a render to collect diagnostics.");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.add_css_class("is-code");
    scroller.set_child(Some(&view));
    page.append(&scroller);
    let paths = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let load: Rc<dyn Fn()> = Rc::new({
        let paths = paths.clone();
        let dropdown = dropdown.clone();
        let list = list.clone();
        let view = view.clone();
        move || {
            let previous = paths.borrow().get(dropdown.selected() as usize).cloned();
            let mut found: Vec<PathBuf> = std::fs::read_dir(base.join("logs"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect();
            found.sort();
            found.reverse();
            found.truncate(100);
            let names: Vec<String> = found
                .iter()
                .map(|p| {
                    p.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let selected = previous
                .and_then(|p| found.iter().position(|n| *n == p))
                .unwrap_or(0);
            *paths.borrow_mut() = found;
            list.splice(0, list.n_items(), &refs);
            dropdown.set_selected(selected as u32);
            view.buffer().set_text(
                &paths
                    .borrow()
                    .get(selected)
                    .map(|p| readable(p))
                    .unwrap_or_else(|| {
                        "No render logs yet. Start a render to collect diagnostics.".into()
                    }),
            );
        }
    });
    dropdown.connect_selected_notify({
        let paths = paths.clone();
        let view = view.clone();
        move |dropdown| {
            if let Some(path) = paths.borrow().get(dropdown.selected() as usize) {
                view.buffer().set_text(&readable(path));
            }
        }
    });
    refresh.connect_clicked({
        let load = load.clone();
        move |_| load()
    });
    load();
    page.upcast()
}
