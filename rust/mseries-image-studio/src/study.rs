//! Native review and sequential comparison workspaces.
use crate::style;
use crate::{Recipe, recipe::load_outputs};
use gtk::prelude::*;
use std::{cell::RefCell, path::Path, rc::Rc};

type Reuse = Rc<dyn Fn(&Recipe)>;
type Snapshot = Rc<dyn Fn() -> Recipe>;

fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label
}
fn page(title: &str, subtitle: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_margin_top(22);
    page.set_margin_bottom(24);
    page.set_margin_start(26);
    page.set_margin_end(26);
    page.append(&style::page_header(title, subtitle));
    page
}
fn field_label(text: &str) -> gtk::Label {
    let paragraph = label(text);
    paragraph.add_css_class("is-field-label");
    paragraph
}
fn scroll(page: &gtk::Box) -> gtk::Widget {
    gtk::ScrolledWindow::builder()
        .child(page)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}
fn base() -> std::path::PathBuf {
    gtk::glib::user_data_dir().join("mseries-image-studio")
}
/// "Illustration · 512×768 · Misty alpine lake at dawn…" so images are recognisable in pickers.
fn title(recipe: &Recipe) -> String {
    let prompt: String = recipe.prompt.chars().take(44).collect();
    let ellipsis = if recipe.prompt.chars().count() > 44 {
        "…"
    } else {
        ""
    };
    format!(
        "{} · {}×{} · {}{}",
        recipe.intent, recipe.width, recipe.height, prompt, ellipsis
    )
}
fn text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}
fn run(script: &'static str, arguments: Vec<String>, status: &gtk::Label, button: &gtk::Button) {
    let directory = base();
    if directory.join("generation.lock").exists() || directory.join("batch.lock").exists() {
        status.set_text("A workflow is active. Stop it or wait before starting another.");
        return;
    }
    let _ = std::fs::remove_file(directory.join("cancel.request"));
    button.set_sensitive(false);
    status.set_text("Starting local workflow…");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .args(arguments)
            .output();
        let message = match result {
            Ok(output) if output.status.success() => {
                "Workflow finished. Results are saved locally.".to_string()
            }
            Ok(output) => format!(
                "Workflow unavailable: {}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(3000)
                    .collect::<String>()
            ),
            Err(error) => format!("Could not start workflow: {error}"),
        };
        let _ = tx.send(message);
    });
    let status = status.clone();
    let button = button.clone();
    gtk::glib::timeout_add_seconds_local(1, move || match rx.try_recv() {
        Ok(message) => {
            status.set_text(&message);
            button.set_sensitive(true);
            gtk::glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            status.set_text("Workflow worker disconnected.");
            button.set_sensitive(true);
            gtk::glib::ControlFlow::Break
        }
        Err(_) => gtk::glib::ControlFlow::Continue,
    });
}

fn review_text(value: &serde_json::Value, path: &str) -> String {
    let score = value["overall_score"]
        .as_f64()
        .map(|v| format!("{v:.2}/5 (advisory)"))
        .unwrap_or_else(|| "Not scored".into());
    let mut lines = vec![format!(
        "Review: {}\n{}\nOverall: {}\n\n{}\n",
        value["recipe_path"].as_str().unwrap_or("Image unavailable"),
        value["evaluator"]
            .as_str()
            .unwrap_or("Evaluator unavailable"),
        score,
        value["summary"].as_str().unwrap_or("No summary")
    )];
    if let Some(scores) = value["scores"].as_object() {
        for (name, row) in scores {
            lines.push(format!(
                "{}: {} / 5\n{}\n",
                name.replace('_', " "),
                row["score"],
                row["reason"].as_str().unwrap_or("No explanation")
            ));
        }
    }
    lines.push(format!(
        "Suggested prompt\n{}\n\nRubric: {}\nReport retained at {}",
        value["suggested_prompt"].as_str().unwrap_or("Unavailable"),
        value["rubric"].as_str().unwrap_or("Unavailable"),
        path
    ));
    lines.join("\n")
}

pub fn review(on_reuse: Reuse) -> gtk::Widget {
    let page = page(
        "Review an image",
        "Compare a generated image with your goal and an optional reference. Qwen vision runs locally after generation unloads. Scores are advisory; each report records its evaluator, rubric, image digest and settings.",
    );
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 10);
    controls.set_hexpand(true);
    let left = style::padded_card(12);
    left.set_width_request(340);
    left.set_valign(gtk::Align::Start);
    let left_body = style::card_content(&left);
    let images = Rc::new(RefCell::new(Vec::<Recipe>::new()));
    let names = gtk::StringList::new(&[]);
    let selection = gtk::DropDown::new(Some(names.clone()), None::<gtk::Expression>);
    selection.update_property(&[gtk::accessible::Property::Label("Image to review")]);
    controls.append(&field_label("Image to review"));
    controls.append(&selection);
    let reference_names = gtk::StringList::new(&["No reference"]);
    let reference = gtk::DropDown::new(Some(reference_names.clone()), None::<gtk::Expression>);
    reference.update_property(&[gtk::accessible::Property::Label("Optional reference image")]);
    controls.append(&field_label("Optional appearance or composition reference"));
    controls.append(&reference);
    let refresh = gtk::Button::with_label("Refresh image list");
    refresh.set_halign(gtk::Align::Start);
    refresh.add_css_class("is-pill");
    refresh.add_css_class("flat");
    controls.append(&refresh);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(-1, 300);
    left_body.append(&picture);
    let source = label("Select an image from Library.");
    source.set_lines(3);
    source.set_ellipsize(gtk::pango::EllipsizeMode::End);
    source.add_css_class("is-caption");
    left_body.append(&source);
    let refill = {
        let images = images.clone();
        let names = names.clone();
        let reference_names = reference_names.clone();
        let picture = picture.clone();
        let source = source.clone();
        move || {
            let values = load_outputs(&base().join("images"), 100).unwrap_or_default();
            names.splice(0, names.n_items(), &[]);
            reference_names.splice(0, reference_names.n_items(), &["No reference"]);
            for recipe in &values {
                names.append(&title(recipe));
                reference_names.append(&title(recipe));
            }
            if let Some(recipe) = values.first() {
                picture.set_filename(Some(&recipe.image_path));
                source.set_text(&recipe.prompt);
            } else {
                picture.set_filename(None::<&str>);
                source.set_text("No saved images yet.");
            }
            *images.borrow_mut() = values;
        }
    };
    refill();
    refresh.connect_clicked(move |_| refill());
    {
        let images = images.clone();
        let picture = picture.clone();
        let source = source.clone();
        selection.connect_selected_notify(move |selection| {
            if let Some(recipe) = images.borrow().get(selection.selected() as usize) {
                picture.set_filename(Some(&recipe.image_path));
                source.set_text(&recipe.prompt);
            }
        });
    }
    controls.append(&field_label("What should the reviewer prioritize?"));
    let rubric = gtk::TextView::new();
    rubric.update_property(&[gtk::accessible::Property::Label(
        "Vision review goal and rubric",
    )]);
    rubric.set_wrap_mode(gtk::WrapMode::WordChar);
    rubric.set_size_request(-1, 90);
    rubric.set_left_margin(10);
    rubric.set_right_margin(10);
    rubric.set_top_margin(8);
    rubric.set_bottom_margin(8);
    rubric.buffer().set_text("Natural adult expression, accurate cozy knitwear, warm comfortable lighting, coherent composition and believable hands. Preserve the subject and intent.");
    let rubric_scroll = gtk::ScrolledWindow::builder()
        .child(&rubric)
        .min_content_height(90)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    rubric_scroll.add_css_class("is-field");
    controls.append(&rubric_scroll);
    let review = gtk::Button::with_label("Review selected image");
    review.add_css_class("suggested-action");
    review.add_css_class("is-pill");
    review.set_halign(gtk::Align::Start);
    controls.append(&review);
    let status = label(
        "Requires Qwen3-VL 2B, its matching vision projector, and the local llama-server runtime.",
    );
    status.add_css_class("is-caption");
    controls.append(&status);
    let report = gtk::TextView::new();
    report.set_editable(false);
    report.set_wrap_mode(gtk::WrapMode::WordChar);
    report.set_left_margin(12);
    report.set_right_margin(12);
    report.set_top_margin(10);
    report.set_bottom_margin(10);
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    columns.append(&left);
    columns.append(&controls);
    page.append(&columns);
    page.append(&field_label("Review report"));
    let report_scroll = gtk::ScrolledWindow::builder()
        .child(&report)
        .min_content_height(200)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    report_scroll.add_css_class("is-code");
    page.append(&report_scroll);
    let suggestion = Rc::new(RefCell::new(String::new()));
    let report_recipe = Rc::new(RefCell::new(None::<Recipe>));
    let use_suggestion = gtk::Button::with_label("Use suggested prompt");
    use_suggestion.set_halign(gtk::Align::Start);
    use_suggestion.add_css_class("is-pill");
    use_suggestion.set_sensitive(false);
    page.append(&use_suggestion);
    {
        let images = images.clone();
        let report = report.clone();
        let suggestion = suggestion.clone();
        let report_recipe = report_recipe.clone();
        let use_suggestion = use_suggestion.clone();
        selection.connect_selected_notify(move |selection| {
            *suggestion.borrow_mut() = String::new();
            *report_recipe.borrow_mut() = None;
            use_suggestion.set_sensitive(false);
            report
                .buffer()
                .set_text("No saved review for this image yet.");
            if let Some(recipe) = images.borrow().get(selection.selected() as usize) {
                let path = Path::new(&recipe.image_path).with_extension("review.json");
                if let Ok(raw) = std::fs::read(&path) {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) {
                        report
                            .buffer()
                            .set_text(&review_text(&value, &path.to_string_lossy()));
                        *suggestion.borrow_mut() =
                            value["suggested_prompt"].as_str().unwrap_or("").to_string();
                        *report_recipe.borrow_mut() = Some(recipe.clone());
                        use_suggestion.set_sensitive(!suggestion.borrow().is_empty());
                    }
                }
            }
        });
    }
    {
        let images = images.clone();
        let selection = selection.clone();
        let reference = reference.clone();
        let rubric = rubric.clone();
        let status = status.clone();
        review.connect_clicked(move |button| {
            let values = images.borrow();
            let Some(recipe) = values.get(selection.selected() as usize) else {
                status.set_text("Select a saved image first.");
                return;
            };
            let reference_path = reference
                .selected()
                .checked_sub(1)
                .and_then(|index| values.get(index as usize))
                .map(|r| r.image_path.clone())
                .unwrap_or_default();
            run(
                include_str!("review_image.py"),
                vec![
                    base().to_string_lossy().into_owned(),
                    Path::new(&recipe.image_path)
                        .with_extension("json")
                        .to_string_lossy()
                        .into_owned(),
                    text(&rubric),
                    reference_path,
                ],
                &status,
                button,
            );
        });
    }
    {
        let suggestion = suggestion.clone();
        let report_recipe = report_recipe.clone();
        use_suggestion.connect_clicked(move |_| {
            if let Some(mut recipe) = report_recipe.borrow().clone() {
                recipe.prompt = suggestion.borrow().clone();
                on_reuse(&recipe);
            }
        });
    }
    let mut previous = String::new();
    let weak = page.downgrade();
    gtk::glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return gtk::glib::ControlFlow::Break;
        }
        if let Ok(raw) = std::fs::read_to_string(base().join("review-status.json")) {
            if raw != previous {
                if let Ok(job) = serde_json::from_str::<serde_json::Value>(&raw) {
                    status.set_text(
                        job["status"]
                            .as_str()
                            .unwrap_or("Review status unavailable"),
                    );
                    if let Some(path) = job["report_path"].as_str() {
                        if let Ok(raw) = std::fs::read_to_string(path) {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                                report.buffer().set_text(&review_text(&value, path));
                                *suggestion.borrow_mut() =
                                    value["suggested_prompt"].as_str().unwrap_or("").to_string();
                                *report_recipe.borrow_mut() = value["recipe_path"]
                                    .as_str()
                                    .and_then(|path| std::fs::read(path).ok())
                                    .and_then(|raw| serde_json::from_slice(&raw).ok());
                                use_suggestion.set_sensitive(
                                    !suggestion.borrow().is_empty()
                                        && report_recipe.borrow().is_some(),
                                );
                            }
                        }
                    }
                }
                previous = raw;
            }
        }
        gtk::glib::ControlFlow::Continue
    });
    scroll(&page)
}

pub fn compare(snapshot: Snapshot, on_reuse: Reuse) -> gtk::Widget {
    let page = page(
        "Compare runs",
        "Run a small, sequential sweep using the recipe in Create. Keep the seed fixed and change one variable. Each result retains its exact prompt, model, steps, sampler, timing and optional vision review.",
    );
    let setup = style::padded_card(14);
    let setup_body = style::card_content(&setup);
    setup_body.set_spacing(12);
    let mode = gtk::DropDown::from_strings(&[
        "Prompt sweep · three lighting variations",
        "Step sweep · 8 (speed test), 16, 24",
        "Model comparison · installed SD 1.5 Q4 and Q8",
    ]);
    setup_body.append(&field_label("Sweep"));
    setup_body.append(&mode);
    let review = gtk::CheckButton::with_label(
        "Review each result with Qwen vision after generation unloads",
    );
    setup_body.append(&review);
    mode.update_property(&[gtk::accessible::Property::Label("Sweep type")]);
    let goal = gtk::Entry::new();
    goal.update_property(&[gtk::accessible::Property::Label("Sweep review goal")]);
    goal.set_placeholder_text(Some(
        "Review goal: clothing, warm lighting, natural expression…",
    ));
    setup_body.append(&goal);
    let run_button = gtk::Button::with_label("Run sweep");
    run_button.add_css_class("suggested-action");
    run_button.add_css_class("is-pill");
    run_button.set_halign(gtk::Align::Start);
    setup_body.append(&run_button);
    let status = label(
        "Sweeps run one image at a time. Stop in the header cancels the current job and the remaining queue.",
    );
    status.add_css_class("is-caption");
    setup_body.append(&status);
    page.append(&setup);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 10);
    page.append(&results);
    {
        let mode = mode.clone();
        let review = review.clone();
        let goal = goal.clone();
        let status = status.clone();
        run_button.connect_clicked(move |button| {
        let recipe=snapshot();let plan=serde_json::json!({"kind":match mode.selected(){0=>"lighting",1=>"steps",_=>"models"},"review":review.is_active(),"rubric":goal.text().to_string()});
        run(include_str!("sweep_runner.py"),vec![base().to_string_lossy().into_owned(),serde_json::to_string(&recipe).unwrap(),plan.to_string(),crate::local_generation_worker(),include_str!("review_image.py").to_string()],&status,button);
    });
    }
    let mut previous = String::new();
    let weak = page.downgrade();
    gtk::glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return gtk::glib::ControlFlow::Break;
        }
        let raw = std::fs::read_to_string(base().join("latest-sweep.txt"))
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok());
        if let Some(raw) = raw {
            if raw != previous {
                if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw) {
                    status.set_text(&format!(
                        "{}{}",
                        manifest["status"].as_str().unwrap_or("Unknown"),
                        manifest["error"]
                            .as_str()
                            .map(|e| format!(": {e}"))
                            .unwrap_or_default()
                    ));
                    while let Some(child) = results.first_child() {
                        results.remove(&child);
                    }
                    if let Some(runs) = manifest["runs"].as_array() {
                        for item in runs {
                            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
                            row.add_css_class("is-row-card");
                            if let Some(path) = item["image_path"].as_str() {
                                let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
                                spacer.set_size_request(150, 150);
                                let thumb = gtk::Overlay::new();
                                thumb.add_css_class("is-tile");
                                thumb.set_overflow(gtk::Overflow::Hidden);
                                thumb.set_valign(gtk::Align::Start);
                                thumb.set_child(Some(&spacer));
                                let picture = gtk::Picture::for_filename(path);
                                picture.set_can_shrink(true);
                                picture.set_content_fit(gtk::ContentFit::Cover);
                                thumb.add_overlay(&picture);
                                row.append(&thumb);
                            }
                            let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
                            details.set_hexpand(true);
                            let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            let heading = label(item["label"].as_str().unwrap_or("Run"));
                            heading.add_css_class("is-card-title");
                            heading.set_hexpand(true);
                            head.append(&heading);
                            let state = item["status"].as_str().unwrap_or("Unknown");
                            let failed =
                                item["error"].is_string() || state.to_lowercase().contains("fail");
                            head.append(&style::chip(
                                state,
                                if failed {
                                    "is-error"
                                } else if state.starts_with("Complete") {
                                    "is-ok"
                                } else {
                                    "is-accent"
                                },
                                None,
                            ));
                            details.append(&head);
                            let stats = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                            stats.append(&style::chip(
                                &item["elapsed_seconds"]
                                    .as_f64()
                                    .map(|v| format!("{v:.1} s"))
                                    .unwrap_or_else(|| "Time not measured".into()),
                                "",
                                Some("content-loading-symbolic"),
                            ));
                            stats.append(&style::chip(
                                &item["peak_memory_mib"]
                                    .as_u64()
                                    .map(|v| format!("{v} MiB peak RSS"))
                                    .unwrap_or_else(|| "Memory not measured".into()),
                                "",
                                None,
                            ));
                            stats.append(&style::chip(
                                &item["overall_score"]
                                    .as_f64()
                                    .map(|v| format!("Vision {v:.2}/5 · advisory"))
                                    .unwrap_or_else(|| "Not reviewed".into()),
                                "",
                                Some("image-red-eye-symbolic"),
                            ));
                            details.append(&stats);
                            if let Some(error) =
                                item["error"].as_str().or(item["review_error"].as_str())
                            {
                                let message = label(error);
                                message.add_css_class("is-caption");
                                details.append(&message);
                            }
                            if let Some(path) = item["recipe_path"].as_str() {
                                if let Ok(raw) = std::fs::read(path) {
                                    if let Ok(recipe) = serde_json::from_slice::<Recipe>(&raw) {
                                        let reuse = gtk::Button::with_label("Use this run");
                                        reuse.set_halign(gtk::Align::Start);
                                        reuse.add_css_class("is-pill");
                                        let on_reuse = on_reuse.clone();
                                        reuse.connect_clicked(move |_| on_reuse(&recipe));
                                        details.append(&reuse);
                                    }
                                }
                            }
                            row.append(&details);
                            results.append(&row);
                        }
                    }
                }
                previous = raw;
            }
        }
        gtk::glib::ControlFlow::Continue
    });
    scroll(&page)
}
