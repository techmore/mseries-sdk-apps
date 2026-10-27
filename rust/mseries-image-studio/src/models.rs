use gtk::prelude::*;
use std::path::Path;

pub fn storage_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let Ok(m) = e.path().symlink_metadata() else {
                return 0;
            };
            if m.is_symlink() {
                0
            } else if m.is_dir() {
                storage_bytes(&e.path())
            } else {
                m.len()
            }
        })
        .sum()
}

/// Four storage figures (models, other AI, images, engine) as quiet stat tiles,
/// measured on a worker thread so the window never waits on the disk.
pub fn storage_tiles(compact: bool) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .column_homogeneous(true)
        .column_spacing(if compact { 10 } else { 18 })
        .row_spacing(if compact { 6 } else { 4 })
        .build();
    let mut values = Vec::new();
    for (index, caption_text) in ["Models", "Other AI", "Images", "Engine"]
        .iter()
        .enumerate()
    {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let value = gtk::Label::new(Some("…"));
        value.set_xalign(0.0);
        value.add_css_class(if compact {
            "is-kv-val"
        } else {
            "is-stat-value"
        });
        let caption = gtk::Label::new(Some(caption_text));
        caption.set_xalign(0.0);
        caption.add_css_class(if compact { "is-kv-key" } else { "is-caption" });
        tile.append(&value);
        tile.append(&caption);
        if compact {
            grid.attach(&tile, (index % 2) as i32, (index / 2) as i32, 1, 1);
        } else {
            grid.attach(&tile, index as i32, 0, 1, 1);
        }
        values.push(value);
    }
    grid.set_tooltip_text(Some(
        "Disk used by this app: downloaded models, shared AI models, images with previews, and the engine build",
    ));
    let (tx, rx) = std::sync::mpsc::channel::<[String; 4]>();
    let base = gtk::glib::user_data_dir();
    std::thread::spawn(move || {
        loop {
            let models = storage_bytes(&base.join("mseries-image-studio/models"));
            let shared = storage_bytes(&base.join("mseries-ai/models"));
            let images = storage_bytes(&base.join("mseries-image-studio/images"))
                + storage_bytes(&base.join("mseries-image-studio/previews"))
                + storage_bytes(&base.join("mseries-image-studio/references"));
            let engine = storage_bytes(&base.join("mseries-image-studio/engine"))
                + storage_bytes(&base.join("mseries-image-studio/engine-source"));
            let gib = |bytes: u64| format!("{:.2} GiB", bytes as f64 / 1073741824.0);
            if tx
                .send([
                    gib(models),
                    gib(shared),
                    format!("{:.1} MiB", images as f64 / 1048576.0),
                    gib(engine),
                ])
                .is_err()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    let weak = grid.downgrade();
    gtk::glib::timeout_add_seconds_local(1, move || {
        let Some(grid) = weak.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        let _ = grid;
        while let Ok(update) = rx.try_recv() {
            for (label, text) in values.iter().zip(update.iter()) {
                label.set_text(text);
            }
        }
        gtk::glib::ControlFlow::Continue
    });
    grid.upcast()
}

/// (name, role, size, kind, tone, files that must exist for "Installed")
const CATALOG: [(&str, &str, &str, &str, &str, &[&str]); 7] = [
    (
        "Stable Diffusion 1.5 · Q4",
        "Smallest first calibration candidate",
        "1.57 GB",
        "Generator",
        "is-accent",
        &["stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf"],
    ),
    (
        "Stable Diffusion 1.5 · Q8",
        "Higher fidelity, same starter settings",
        "1.76 GB",
        "Generator",
        "is-accent",
        &["stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf"],
    ),
    (
        "SDXL Turbo · FP16",
        "Fast composition experiments",
        "6.94 GB",
        "Generator",
        "is-accent",
        &["sd_xl_turbo_1.0_fp16.safetensors"],
    ),
    (
        "Qwen3-VL 2B + projector",
        "Reviews images against a rubric",
        "1.55 GB",
        "Reviewer",
        "is-warn",
        &[
            "Qwen3VL-2B-Instruct-Q4_K_M.gguf",
            "mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf",
        ],
    ),
    (
        "Realistic Vision V6 FP16 + VAE",
        "Photographic portraits and fashion",
        "2.47 GB",
        "Generator",
        "is-accent",
        &[
            "Realistic_Vision_V6.0_NV_B1_fp16.safetensors",
            "vae-ft-mse-840000-ema-pruned.safetensors",
        ],
    ),
    (
        "Reference appearance",
        "IP-Adapter Plus Face + vision encoder",
        "3.9 GB",
        "Add-on",
        "is-warn",
        &[
            "ip-adapter-plus-face_sd15.safetensors",
            "clip-vision-h-f16.gguf",
        ],
    ),
    (
        "TAESD · live previews",
        "Preview decoder only; keeps the full VAE for the saved image",
        "9.8 MB",
        "Add-on",
        "",
        &["taesd.safetensors"],
    ),
];

pub fn page(on_select: std::rc::Rc<dyn Fn(&str)>) -> gtk::Widget {
    let (scroller, page) = crate::style::page_body();
    page.append(&crate::style::page_header(
        "Models",
        "Apple Silicon · Linux ARM64 / Vulkan target. These compact downloads fit the disk budget; actual inference memory and GPU support still need qualification. macOS-only Metal / MLX packages are excluded.",
    ));
    let storage = crate::style::padded_card(14);
    crate::style::card_content(&storage).append(&storage_tiles(false));
    page.append(&storage);

    // The dropdown stays as the single source of truth for the selection;
    // the cards below drive it so download/use logic keeps one index.
    let choices = gtk::DropDown::from_strings(&[
        "Stable Diffusion 1.5 · Q4 · 1.57 GB",
        "Stable Diffusion 1.5 · Q8 · 1.76 GB",
        "SDXL Turbo · FP16 · 6.94 GB",
        "Qwen3-VL 2B + projector · review images · 1.55 GB",
        "Realistic Vision V6 FP16 + VAE · photography · 2.47 GB",
        "Reference appearance · IP-Adapter Plus Face + vision encoder · 3.9 GB",
        "TAESD · fast live previews for SD 1.5 · 9.8 MB",
    ]);
    page.append(&crate::style::section_caption("Available models"));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let cards: std::rc::Rc<Vec<(gtk::Button, gtk::Box)>> = std::rc::Rc::new(
        CATALOG
            .iter()
            .enumerate()
            .map(|(index, (name, role, size, kind, tone, _))| {
                let card = gtk::Button::new();
                card.add_css_class("flat");
                card.add_css_class("is-card");
                card.add_css_class("is-selectable");
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                row.set_margin_top(12);
                row.set_margin_bottom(12);
                row.set_margin_start(14);
                row.set_margin_end(14);
                let icon = gtk::Image::from_icon_name(match *kind {
                    "Reviewer" => "image-red-eye-symbolic",
                    "Add-on" => "insert-image-symbolic",
                    _ => "applications-graphics-symbolic",
                });
                icon.set_pixel_size(28);
                icon.add_css_class("is-dim");
                let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                text.set_hexpand(true);
                text.append(
                    &gtk::Label::builder()
                        .label(*name)
                        .xalign(0.0)
                        .css_classes(["is-card-title"])
                        .build(),
                );
                text.append(&crate::style::caption(role));
                let status_slot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                status_slot.set_valign(gtk::Align::Center);
                row.append(&icon);
                row.append(&text);
                row.append(&crate::style::chip(kind, tone, None));
                row.append(&crate::style::chip(size, "", None));
                row.append(&status_slot);
                card.set_child(Some(&row));
                card.update_property(&[gtk::accessible::Property::Label(name)]);
                let choices = choices.clone();
                card.connect_clicked(move |_| choices.set_selected(index as u32));
                list.append(&card);
                (card, status_slot)
            })
            .collect(),
    );
    page.append(&list);
    let refresh_cards = {
        let cards = cards.clone();
        let choices = choices.clone();
        move || {
            let directory = gtk::glib::user_data_dir().join("mseries-image-studio/models");
            for (index, (card, slot)) in cards.iter().enumerate() {
                if choices.selected() as usize == index {
                    card.add_css_class("is-selected");
                } else {
                    card.remove_css_class("is-selected");
                }
                let installed = CATALOG[index]
                    .5
                    .iter()
                    .all(|file| directory.join(file).is_file());
                while let Some(child) = slot.first_child() {
                    slot.remove(&child);
                }
                slot.append(&if installed {
                    crate::style::chip("Installed", "is-ok", Some("emblem-ok-symbolic"))
                } else {
                    crate::style::chip("Not downloaded", "", None)
                });
            }
        }
    };
    refresh_cards();
    {
        let refresh_cards = refresh_cards.clone();
        choices.connect_selected_notify(move |_| refresh_cards());
    }
    {
        let weak = list.downgrade();
        gtk::glib::timeout_add_seconds_local(2, move || {
            if weak.upgrade().is_none() {
                return gtk::glib::ControlFlow::Break;
            }
            refresh_cards();
            gtk::glib::ControlFlow::Continue
        });
    }

    // Selected model: requirements, license, actions, progress.
    let detail = crate::style::padded_card(14);
    let detail_body = crate::style::card_content(&detail);
    detail_body.set_spacing(10);
    let requirements = gtk::Label::new(Some(
        "SD 1.5 Q4: estimated 3 GiB worker budget plus 6 GiB desktop reserve. Euler, 20 steps, guidance 7, 512 × 512. Good first calibration candidate.",
    ));
    requirements.set_wrap(true);
    requirements.set_xalign(0.0);
    detail_body.append(&requirements);
    choices.connect_selected_notify(move |choices| {
        requirements.set_text(match choices.selected() {
            0 => "SD 1.5 Q4: estimated 3 GiB worker budget plus 6 GiB desktop reserve. Euler, 20 steps, guidance 7, 512 × 512. Good first calibration candidate.",
            1 => "SD 1.5 Q8: estimated 3.5 GiB worker budget plus 6 GiB reserve. Same starter settings as Q4; use Compare to measure quality and speed differences.",
            2 => "SDXL Turbo: estimated 6 GiB worker budget plus 6 GiB reserve. Euler ancestral, 4 steps, engine guidance 1, no negative prompt, 512 × 512. Commercial use needs the applicable model license.",
            4 => "Realistic Vision V6: photographic portrait candidate. Downloads the full FP16 checkpoint and required VAE. Start at 512 × 768, 30 DPM++2M steps, Karras, CFG 5. Local image-quality validation is in progress.",
            6 => "TAESD: lightweight SD 1.5 live-preview decoder (MIT). Final image still uses the full selected VAE. This is an auxiliary preview component, not an image-generation checkpoint.",
            5 => "Reference appearance bundle: SD 1.5 Plus Face adapter and CLIP vision encoder, converted to FP16 locally. Apache 2.0. Requires an installed engine, about 3.9 GB disk plus reserve, and separate inference-memory qualification. Not an image-generation checkpoint.",
            _ => "Qwen3-VL 2B: image reviewer, not an image generator. Downloads both the language weights and matching Q8 vision projector. Uses a separate CPU worker after image generation unloads. Requires 9 GiB available.",
        });
    });
    detail_body.append(&crate::style::caption(
        "SD 1.5: CreativeML Open RAIL-M · SDXL Turbo: Stability AI non-commercial community license. Review the model license before commercial fashion use.",
    ));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let button = gtk::Button::with_label("Download selected model");
    button.add_css_class("suggested-action");
    button.add_css_class("is-pill");
    let cancel_download = gtk::Button::with_label("Cancel download");
    cancel_download.add_css_class("is-pill");
    cancel_download.connect_clicked(move |_| {
        let directory = gtk::glib::user_data_dir().join("mseries-image-studio/models");
        let _ = std::fs::create_dir_all(&directory);
        let _ = std::fs::write(directory.join("download.cancel"), "cancel");
    });
    let use_model = gtk::Button::with_label("Use selected model");
    use_model.add_css_class("is-pill");
    actions.append(&button);
    actions.append(&cancel_download);
    actions.append(&use_model);
    detail_body.append(&actions);
    let download_bar = gtk::ProgressBar::new();
    download_bar.set_visible(false);
    detail_body.append(&download_bar);
    let status = gtk::Label::new(Some(
        "Downloads verify file size and SHA-256, and keep 2 GiB of disk space free. Installing a model does not qualify inference.",
    ));
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.add_css_class("is-caption");
    detail_body.append(&status);
    page.append(&detail);
    let detail_body_for_progress = detail_body.clone();
    {
        let choices = choices.clone();
        let status = status.clone();
        use_model.connect_clicked(move |_| {
            if choices.selected() >= 5 {
                status.set_text("Auxiliary component installed separately; keep an SD 1.5 checkpoint selected in Create.");
                return;
            }
            let names = [
                "stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf",
                "stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf",
                "sd_xl_turbo_1.0_fp16.safetensors",
                "Qwen3VL-2B-Instruct-Q4_K_M.gguf",
                "Realistic_Vision_V6.0_NV_B1_fp16.safetensors",
            ];
            let file = gtk::glib::user_data_dir()
                .join("mseries-image-studio/models")
                .join(names[choices.selected().min(4) as usize]);
            if file.is_file() {
                on_select(&file.to_string_lossy());
            } else {
                status.set_text("Download the selected model first.");
            }
        });
    }
    page.append(&crate::style::section_caption("Image engine"));
    let engine_card = crate::style::padded_card(14);
    let engine_body = crate::style::card_content(&engine_card);
    let engine_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let engine_dot = crate::style::dot("");
    let engine_text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    engine_text.set_hexpand(true);
    engine_text.append(
        &gtk::Label::builder()
            .label("Local image engine · stable-diffusion.cpp")
            .xalign(0.0)
            .css_classes(["is-card-title"])
            .build(),
    );
    let engine_status = gtk::Label::new(Some(
        "Requires Git, CMake, a C++ compiler and Vulkan development tools. Builds the pinned engine with two compiler jobs.",
    ));
    engine_status.set_wrap(true);
    engine_status.set_xalign(0.0);
    engine_status.add_css_class("is-caption");
    engine_text.append(&engine_status);
    let install = gtk::Button::with_label("Install or rebuild");
    install.add_css_class("is-pill");
    install.set_valign(gtk::Align::Center);
    engine_row.append(&engine_dot);
    engine_row.append(&engine_text);
    engine_row.append(&install);
    engine_body.append(&engine_row);
    page.append(&engine_card);
    {
        let engine_dot = engine_dot.clone();
        let weak = engine_card.downgrade();
        let update = move || {
            let installed = gtk::glib::user_data_dir()
                .join("mseries-image-studio/engine/sd-cli")
                .is_file();
            crate::style::set_tone(&engine_dot, if installed { "is-ok" } else { "is-warn" });
        };
        update();
        gtk::glib::timeout_add_seconds_local(3, move || {
            if weak.upgrade().is_none() {
                return gtk::glib::ControlFlow::Break;
            }
            update();
            gtk::glib::ControlFlow::Continue
        });
    }
    install.connect_clicked(move |button| {
        button.set_sensitive(false);
        engine_status.set_text("Preparing the local engine… This can take several minutes.");
        let directory = gtk::glib::user_data_dir().join("mseries-image-studio");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = std::process::Command::new("python3")
                .arg("-c")
                .arg(include_str!("install_engine.py"))
                .arg(directory)
                .output();
            let message = match result {
                Ok(out) if out.status.success() => {
                    "Engine installed. Use a model starter preset to begin calibration.".to_string()
                }
                Ok(out) => String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .rev()
                    .take(3000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
                Err(error) => error.to_string(),
            };
            let _ = tx.send(message);
        });
        let button = button.clone();
        let status = engine_status.clone();
        gtk::glib::timeout_add_seconds_local(1, move || {
            if let Ok(message) = rx.try_recv() {
                status.set_text(&message);
                button.set_sensitive(true);
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    });
    let progress = gtk::Label::new(None);
    progress.set_wrap(true);
    progress.set_xalign(0.0);
    progress.add_css_class("is-caption");
    detail_body_for_progress.append(&progress);
    let bar = download_bar.clone();
    let weak_progress = progress.downgrade();
    gtk::glib::timeout_add_seconds_local(1, move || {
        let Some(progress) = weak_progress.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        let path =
            gtk::glib::user_data_dir().join("mseries-image-studio/models/download-progress.json");
        if let Ok(raw) = std::fs::read(path) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) {
                let received = value["received"].as_u64().unwrap_or(0) as f64;
                let total = value["total"].as_u64().unwrap_or(0) as f64;
                bar.set_visible(total > 0.0 && received < total);
                if total > 0.0 {
                    bar.set_fraction((received / total).clamp(0.0, 1.0));
                }
                progress.set_text(&format!(
                    "{} · {} · {:.1} / {:.1} MiB",
                    value["file"].as_str().unwrap_or("Model"),
                    value["phase"].as_str().unwrap_or("Unknown"),
                    value["received"].as_u64().unwrap_or(0) as f64 / 1048576.0,
                    value["total"].as_u64().unwrap_or(0) as f64 / 1048576.0
                ));
            }
        }
        gtk::glib::ControlFlow::Continue
    });
    let choices_for_download = choices.clone();
    button.connect_clicked(move |button| {
        let choices = choices_for_download.clone();
        button.set_sensitive(false);
        choices.set_sensitive(false);
        status.set_text("Downloading and verifying… Storage totals include partial downloads.");
        let index = choices.selected();
        let directory = gtk::glib::user_data_dir().join("mseries-image-studio/models");
        let _ = std::fs::remove_file(directory.join("download.cancel"));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if index == 5 || index == 6 {
                let message = match std::process::Command::new("python3")
                    .arg("-c")
                    .arg(if index == 6 {
                        TAESD_DOWNLOAD
                    } else {
                        REFERENCE_BUNDLE_DOWNLOAD
                    })
                    .arg(&directory)
                    .output()
                {
                    Ok(out) => format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    ),
                    Err(error) => error.to_string(),
                };
                let _ = tx.send(message);
                return;
            }
            if index == 3 || index == 4 {
                let mut messages = Vec::new();
                let bundle = if index == 3 {
                    [
                        (
                            "Qwen/Qwen3-VL-2B-Instruct-GGUF",
                            "Qwen3VL-2B-Instruct-Q4_K_M.gguf",
                        ),
                        (
                            "Qwen/Qwen3-VL-2B-Instruct-GGUF",
                            "mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf",
                        ),
                    ]
                } else {
                    [
                        (
                            "SG161222/Realistic_Vision_V6.0_B1_noVAE",
                            "Realistic_Vision_V6.0_NV_B1_fp16.safetensors",
                        ),
                        (
                            "stabilityai/sd-vae-ft-mse-original",
                            "vae-ft-mse-840000-ema-pruned.safetensors",
                        ),
                    ]
                };
                for (repo, file) in bundle {
                    match std::process::Command::new("python3")
                        .arg("-c")
                        .arg(include_str!("download_model.py"))
                        .args([repo, file])
                        .arg(&directory)
                        .output()
                    {
                        Ok(output) => {
                            messages.push(format!(
                                "{}{}",
                                String::from_utf8_lossy(&output.stdout),
                                String::from_utf8_lossy(&output.stderr)
                            ));
                            if !output.status.success() {
                                break;
                            }
                        }
                        Err(error) => {
                            messages.push(error.to_string());
                            break;
                        }
                    }
                }
                let _ = tx.send(messages.join("\n"));
                return;
            }
            let (repo, file) = match index {
                0 => (
                    "second-state/stable-diffusion-v1-5-GGUF",
                    "stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf",
                ),
                1 => (
                    "second-state/stable-diffusion-v1-5-GGUF",
                    "stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf",
                ),
                _ => ("stabilityai/sdxl-turbo", "sd_xl_turbo_1.0_fp16.safetensors"),
            };
            let result = std::process::Command::new("python3")
                .arg("-c")
                .arg(include_str!("download_model.py"))
                .args([repo, file])
                .arg(directory)
                .output();
            let text = match result {
                Ok(out) => format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
                Err(e) => e.to_string(),
            };
            let _ = tx.send(text);
        });
        let button = button.clone();
        let choices = choices.clone();
        let status = status.clone();
        gtk::glib::timeout_add_seconds_local(1, move || {
            if let Ok(text) = rx.try_recv() {
                status.set_text(&text);
                button.set_sensitive(true);
                choices.set_sensitive(true);
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    });
    scroller.upcast()
}

// Runs on the downloader worker thread, never on GTK's main loop.
const REFERENCE_BUNDLE_DOWNLOAD: &str = r#"
import os,sys,json,hashlib,urllib.request,subprocess,time,shutil
from pathlib import Path
os.umask(0o077)
dest=Path(sys.argv[1]);dest.mkdir(parents=True,exist_ok=True)
base=dest.parent;revision='018e402774aeeddd60609b4ecdb7e298259dc729'
cancel=dest/'download.cancel';lock=dest/'reference-download.lock';owned=False
generation_lock=base/'generation.lock';generation_owned=False
engine_revision='fd21ecfceef2665d2c5f2db8536e3202d864fd30'
def publish(phase,name='Reference appearance bundle',received=0,total=0):
    tmp=dest/'download-progress.reference.tmp'
    tmp.write_text(json.dumps(dict(file=name,phase=phase,received=received,total=total)))
    os.replace(tmp,dest/'download-progress.json')
def cancelled():
    if cancel.exists():raise RuntimeError('Reference download or conversion cancelled.')
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while True:
            cancelled();chunk=f.read(1024*1024)
            if not chunk:break
            h.update(chunk)
    return h.hexdigest()
def receipt(path,data):
    tmp=path.with_suffix(path.suffix+'.receipt.tmp');tmp.write_text(json.dumps(data,indent=2));os.replace(tmp,path.with_suffix(path.suffix+'.json'))
try:
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.close(fd);owned=True
    cancelled()
    binary=base/'engine/sd-cli'
    source=json.loads((base/'engine/source.json').read_text())
    if source.get('revision')!=engine_revision or not binary.is_file():raise RuntimeError('Install the pinned local engine before the reference bundle.')
    if (base/'generation.lock').exists() or (base/'batch.lock').exists():raise RuntimeError('Wait for the active generation or comparison before conversion.')
    for remote,name in [('models/image_encoder/model.safetensors','clip-vision-h.safetensors'),('models/ip-adapter-plus-face_sd15.safetensors','ip-adapter-plus-face_sd15.safetensors')]:
        cancelled();publish('Reading pinned metadata',name)
        with urllib.request.urlopen('https://huggingface.co/api/models/h94/IP-Adapter/revision/'+revision+'?blobs=true',timeout=30) as response:info=json.load(response)
        entry=next(x for x in info['siblings'] if x['rfilename']==remote);size=entry['lfs']['size'];expected=entry['lfs']['sha256']
        target=dest/name;part=dest/(name+'.reference.part')
        if target.exists():
            if target.stat().st_size!=size or digest(target)!=expected:raise RuntimeError('Existing '+name+' differs from pinned source; preserved without overwrite.')
        else:
            if shutil.disk_usage(dest).free<size+int(1.4*1024**3)+2*1024**3:raise RuntimeError('Insufficient storage including conversion and 2 GiB disk reserve.')
            part_created=False
            try:
                received=0;last=0
                with urllib.request.urlopen('https://huggingface.co/h94/IP-Adapter/resolve/'+revision+'/'+remote,timeout=60) as response,part.open('xb') as stream:
                    part_created=True
                    while True:
                        cancelled();chunk=response.read(1024*1024)
                        if not chunk:break
                        stream.write(chunk);received+=len(chunk)
                        if received>size:raise RuntimeError('Source exceeded declared size.')
                        if time.monotonic()-last>1:publish('Downloading',name,received,size);last=time.monotonic()
                if received!=size or digest(part)!=expected:raise RuntimeError('Reference integrity verification failed.')
                os.replace(part,target)
            finally:
                if part_created:part.unlink(missing_ok=True)
        receipt(target,dict(repo='h94/IP-Adapter',revision=revision,source_path=remote,sha256=expected,size=size,license='apache-2.0',qualified=False))
    encoder=dest/'clip-vision-h.safetensors';output=dest/'clip-vision-h-f16.gguf';partial=dest/'clip-vision-h-f16.reference.tmp.gguf'
    source_sha=digest(encoder)
    if output.exists():
        saved=json.loads(output.with_suffix(output.suffix+'.json').read_text())
        if saved.get('source_sha256')!=source_sha or saved.get('engine_revision')!=engine_revision or saved.get('sha256')!=digest(output):raise RuntimeError('Existing converted encoder has no matching verified receipt; preserved.')
    else:
        generation_fd=os.open(generation_lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
        generation_owned=True
        try:os.write(generation_fd,str(os.getpid()).encode())
        finally:os.close(generation_fd)
        if (base/'batch.lock').exists():raise RuntimeError('A comparison started; wait before converting the vision encoder.')
        available=next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
        if available<9*1024**3:raise RuntimeError('FP16 conversion needs 9 GiB available including the protected desktop reserve.')
        if partial.exists():raise RuntimeError('A previous conversion partial exists; inspect it before retrying.')
        publish('Converting vision encoder to FP16; cancellation remains available')
        command=[str(binary),'-M','convert','-m',str(encoder),'--type','f16','-o',str(partial)]
        try:
            with (dest/'reference-conversion.log').open('w') as log:
                proc=subprocess.Popen(command,stdout=log,stderr=log)
                try:
                    while proc.poll() is None:
                        cancelled()
                        free=next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
                        if free<6*1024**3:raise RuntimeError('Conversion stopped to preserve the 6 GiB desktop reserve.')
                        time.sleep(0.25)
                finally:
                    if proc.poll() is None:
                        proc.terminate()
                        try:proc.wait(timeout=5)
                        except subprocess.TimeoutExpired:proc.kill();proc.wait()
                if proc.returncode!=0:raise RuntimeError('Vision conversion failed; see reference-conversion.log.')
            cancelled()
            if not partial.is_file() or partial.stat().st_size==0:raise RuntimeError('Conversion produced no encoder.')
            converted_sha=digest(partial);os.replace(partial,output)
            receipt(output,dict(source_sha256=source_sha,source_revision=revision,engine_revision=engine_revision,engine_sha256=digest(binary),sha256=converted_sha,size=output.stat().st_size,conversion=command,qualified=False))
        finally:partial.unlink(missing_ok=True)
    publish('Reference bundle verified; inference qualification required')
    print('Reference appearance bundle installed and verified. Select an SD 1.5 checkpoint and reference image; memory qualification is still required.')
except Exception as error:
    publish('Unavailable: '+str(error));print(str(error),file=sys.stderr);sys.exit(1)
finally:
    if generation_owned:generation_lock.unlink(missing_ok=True)
    if owned:lock.unlink(missing_ok=True)
"#;

const TAESD_DOWNLOAD: &str = r#"
import sys,os,json,urllib.request,hashlib,shutil,time
from pathlib import Path
os.umask(0o077)
dest=Path(sys.argv[1]);dest.mkdir(parents=True,exist_ok=True)
revision='614f76814bbe30edbe2e627ace1c2234c81a2c0e';repo='madebyollin/taesd';remote='diffusion_pytorch_model.safetensors'
target=dest/'taesd.safetensors';part=dest/'taesd.safetensors.download.part';cancel=dest/'download.cancel'
lock=dest/'taesd-download.lock';owned=False;created=False
size=9793292
def check():
    if cancel.exists():raise RuntimeError('Preview decoder download cancelled.')
def publish(phase,received=0):
    tmp=dest/'download-progress.taesd.tmp';tmp.write_text(json.dumps(dict(file='TAESD',phase=phase,received=received,total=size)));os.replace(tmp,dest/'download-progress.json')
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while True:
            check();chunk=f.read(1024*1024)
            if not chunk:break
            h.update(chunk)
    return h.hexdigest()
try:
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.close(fd);owned=True
    check();publish('Reading pinned preview decoder metadata')
    with urllib.request.urlopen('https://huggingface.co/api/models/'+repo+'/revision/'+revision+'?blobs=true',timeout=30) as response:info=json.load(response)
    entry=next(x for x in info['siblings'] if x['rfilename']==remote);expected=entry['lfs']['sha256']
    if entry['lfs']['size']!=size:raise RuntimeError('Pinned preview decoder size mismatch.')
    if target.exists():
        if target.stat().st_size!=size or digest(target)!=expected:raise RuntimeError('Existing TAESD differs from pinned source; preserved.')
    else:
        if shutil.disk_usage(dest).free<size+2*1024**3:raise RuntimeError('Insufficient disk space with 2 GiB reserve.')
        received=0
        with urllib.request.urlopen('https://huggingface.co/'+repo+'/resolve/'+revision+'/'+remote,timeout=60) as response,part.open('xb') as stream:
            created=True
            while True:
                check();chunk=response.read(1024*1024)
                if not chunk:break
                stream.write(chunk);received+=len(chunk);publish('Downloading preview decoder',received)
                if received>size:raise RuntimeError('Preview decoder exceeded declared size.')
        if received!=size or digest(part)!=expected:raise RuntimeError('Preview decoder integrity verification failed.')
        os.replace(part,target)
    receipt=target.with_suffix(target.suffix+'.json');tmp=receipt.with_suffix('.tmp')
    tmp.write_text(json.dumps(dict(repo=repo,revision=revision,source_path=remote,sha256=expected,size=size,license='MIT',purpose='preview-only; final image uses full VAE',qualified=False),indent=2));os.replace(tmp,receipt)
    publish('Preview decoder verified',size);print('TAESD installed and verified for live previews; final images keep the selected full VAE.')
except Exception as error:
    publish('Unavailable: '+str(error));print(str(error),file=sys.stderr);sys.exit(1)
finally:
    if created:part.unlink(missing_ok=True)
    if owned:lock.unlink(missing_ok=True)
"#;
