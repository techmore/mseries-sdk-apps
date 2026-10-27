use crate::assistant;
use crate::host::HostCapabilities;
use crate::recipe::{Recipe, RecipeStore, load_outputs};
use crate::style::{self, caption, chip, dot, icon_button};
use granite::prelude::{PlaceholderExt, ToastExt};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

const INTENTS: &[&str] = &[
    "Portrait",
    "Streamer scene",
    "Product photo",
    "Illustration",
    "Graphic or poster",
];
const SAMPLE_SERIES: &str = "Three-image editorial series of an original fictional adult character, a friendly mid-20s college student and game streamer: (1) hosting a relaxed gaming stream, (2) recording a campus-day vlog, (3) editing a video in a bright campus cafe. Casual everyday clothing, friendly expression, natural light, consistent identity and visual style.";

/// (page name, label, icon) for every destination, grouped by sidebar section.
const STUDIO_PAGES: &[(&str, &str, &str)] = &[
    ("create", "Create", "applications-graphics-symbolic"),
    ("library", "Library", "folder-pictures-symbolic"),
    ("compare", "Compare", "view-dual-symbolic"),
    ("review", "Review", "image-red-eye-symbolic"),
];
const SETUP_PAGES: &[(&str, &str, &str)] = &[
    ("models", "Models", "drive-harddisk-symbolic"),
    ("guide", "Model guide", "help-browser-symbolic"),
    ("logs", "Run logs", "utilities-terminal-symbolic"),
];

fn page_title(name: &str) -> &'static str {
    STUDIO_PAGES
        .iter()
        .chain(SETUP_PAGES)
        .find(|(page, _, _)| *page == name)
        .map(|(_, label, _)| *label)
        .unwrap_or("Image Studio")
}

pub struct ImageStudioWindow {
    window: gtk::ApplicationWindow,
}

impl ImageStudioWindow {
    pub fn build(application: &gtk::Application) -> Self {
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Image Studio")
            .default_width(1380)
            .default_height(900)
            .build();
        window.add_css_class("mseries-image-studio");
        style::install(&window);

        let header = gtk::HeaderBar::builder().show_title_buttons(true).build();
        header.add_css_class(granite::STYLE_CLASS_FLAT);
        let title = gtk::Label::new(Some("Create"));
        title.add_css_class("is-title-text");
        header.set_title_widget(Some(&title));
        let menu = gio::Menu::new();
        menu.append(Some("About Image Studio"), Some("app.about"));
        menu.append(Some("Quit"), Some("app.quit"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .build();
        menu_button.update_property(&[gtk::accessible::Property::Label("Application menu")]);
        header.pack_end(&menu_button);

        let stack = gtk::Stack::builder()
            .hexpand(true)
            .vexpand(true)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(140)
            .build();
        let recipe_store = RecipeStore::default_store();
        let create = build_create_page();
        let library = build_library_page(&recipe_store);
        stack.add_named(&create.page, Some("create"));
        stack.add_named(
            &crate::logs::page(glib::user_data_dir().join("mseries-image-studio")),
            Some("logs"),
        );
        stack.add_named(&library.page, Some("library"));

        let inspector = build_inspector();
        // The model picker lives in the inspector; the dock shows a live summary of the recipe.
        let settings = inspector.clone_refs();
        install_recipe_chips(&create, &settings);
        let reuse: Rc<dyn Fn(&Recipe)> = {
            let create = create.clone_refs();
            let inspector = inspector.clone_refs();
            let stack = stack.clone();
            Rc::new(move |recipe| {
                load_recipe(recipe, &create, &inspector);
                stack.set_visible_child_name("create");
            })
        };
        let snapshot: Rc<dyn Fn() -> Recipe> = {
            let create = create.clone_refs();
            let inspector = inspector.clone_refs();
            Rc::new(move || current_recipe(&create, &inspector))
        };
        stack.add_named(&crate::study::review(reuse.clone()), Some("review"));
        stack.add_named(&crate::study::compare(snapshot, reuse), Some("compare"));
        let select_model: Rc<dyn Fn(&str)> = {
            let create = create.clone_refs();
            let inspector = inspector.clone_refs();
            let stack = stack.clone();
            Rc::new(move |path| {
                if path.contains("Qwen3VL") {
                    stack.set_visible_child_name("review");
                    return;
                }
                inspector.model.set_text(path);
                inspector.dimensions.set_selected(0);
                inspector.batch.set_value(1.0);
                inspector.seed.set_value(424242.0);
                inspector.lora.set_text("");
                let turbo = path.contains("turbo");
                inspector.steps.set_value(if turbo { 4.0 } else { 20.0 });
                inspector.cfg.set_value(if turbo { 1.0 } else { 7.0 });
                inspector.sampler.set_selected(if turbo { 1 } else { 0 });
                if path.contains("Realistic") {
                    inspector.steps.set_value(30.0);
                    inspector.cfg.set_value(5.0);
                    inspector.dimensions.set_selected(3);
                    inspector.sampler.set_selected(2);
                    inspector.scheduler.set_selected(1);
                }
                if turbo {
                    set_text(&create.negative, "");
                }
                inspector.recipe_status.set_text("Selected model and starter settings loaded. Local memory admission still applies.");
                stack.set_visible_child_name("create");
            })
        };
        stack.add_named(&crate::models::page(select_model), Some("models"));

        stack.add_named(
            &build_model_guide(&create, &inspector, &stack),
            Some("guide"),
        );
        let reload = gtk::Button::from_icon_name("document-open-recent-symbolic");
        reload.set_tooltip_text(Some(
            "Reload recipe: load the active or last completed generation into Create",
        ));
        reload.update_property(&[gtk::accessible::Property::Label("Reload recipe")]);
        let reload_last: Rc<dyn Fn(bool)> = {
            let create = create.clone_refs();
            let inspector = inspector.clone_refs();
            let stack = stack.clone();
            Rc::new(move |silent| {
                let path = gtk::glib::user_data_dir().join("mseries-image-studio/current-job.json");
                let result = std::fs::read(path)
                    .ok()
                    .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
                    .and_then(|job| job["recipe_path"].as_str().map(str::to_owned))
                    .and_then(|path| std::fs::read(path).ok())
                    .and_then(|raw| serde_json::from_slice::<Recipe>(&raw).ok());
                if let Some(recipe) = result {
                    load_recipe(&recipe, &create, &inspector);
                    if !silent {
                        stack.set_visible_child_name("create");
                    }
                } else if !silent {
                    inspector
                        .recipe_status
                        .set_text("No readable generation recipe is available yet.");
                }
            })
        };
        let reload_now = reload_last.clone();
        reload.connect_clicked(move |_| reload_now(false));
        header.pack_start(&reload);

        let action = inspector.generate.clone();
        let run_action = gio::SimpleAction::new("run", None);
        run_action.connect_activate(move |_, _| {
            if action.is_sensitive() {
                action.emit_clicked();
            }
        });
        inspector
            .generate
            .bind_property("sensitive", &run_action, "enabled")
            .sync_create()
            .build();
        application.add_action(&run_action);
        application.set_accels_for_action("app.run", &["<Control>Return"]);
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.set_tooltip_text(Some(
            "Stop: cancel the current image, review or remaining sweep queue",
        ));
        stop.update_property(&[gtk::accessible::Property::Label("Stop")]);
        stop.connect_clicked(move |_| {
            let base = glib::user_data_dir().join("mseries-image-studio");
            let _ = std::fs::create_dir_all(&base);
            if let Err(error) = std::fs::write(base.join("cancel.request"), "cancel") {
                eprintln!("Could not request cancellation: {error}");
            }
        });
        header.pack_end(&stop);
        let settings_popover = gtk::Popover::new();
        settings_popover.set_size_request(340, 560);
        let settings_label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        settings_label.append(&gtk::Image::from_icon_name("preferences-system-symbolic"));
        settings_label.append(&gtk::Label::new(Some("Settings")));
        let settings_button = gtk::MenuButton::builder()
            .child(&settings_label)
            .tooltip_text("Generation settings")
            .popover(&settings_popover)
            .build();
        settings_button.set_visible(false);
        settings_button.update_property(&[gtk::accessible::Property::Label("Generation settings")]);
        header.pack_end(&settings_button);
        let main_split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&stack)
            .end_child(&inspector.panel)
            .position(720)
            .resize_start_child(true)
            .resize_end_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        let visible_settings = inspector.panel.clone();
        let compact_settings_button = settings_button.clone();
        let settings_window = window.downgrade();
        let header_title = title.clone();
        stack.connect_visible_child_name_notify(move |stack| {
            let name = stack.visible_child_name();
            let on_create = name.as_deref() == Some("create");
            header_title.set_text(page_title(name.as_deref().unwrap_or("create")));
            let compact = settings_window
                .upgrade()
                .is_some_and(|w| w.has_css_class("pantheon-compact"));
            compact_settings_button.set_visible(compact && on_create);
            visible_settings.set_visible(on_create);
        });
        let sidebar = build_sidebar(&stack, &create, &inspector);
        let split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&sidebar)
            .end_child(&main_split)
            .position(224)
            .resize_start_child(false)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        let toast = granite::Toast::new("");
        toast.set_halign(gtk::Align::Center);
        toast.set_valign(gtk::Align::End);
        toast.set_margin_bottom(28);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&split));
        overlay.add_overlay(&toast);
        window.set_titlebar(Some(&header));
        window.set_child(Some(&overlay));
        let toast_status = inspector.recipe_status.clone();
        toast_status.connect_label_notify(move |label| {
            let text = label.label();
            if [
                "Recipe saved",
                "Recipe loaded",
                "Selected model",
                "Starter defaults",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
            {
                toast.set_title(text.as_str());
                toast.send_notification();
            }
        });
        // Sidebar + inspector + a usable canvas needs about 1100 px (76 text units); below
        // that the inspector moves into the Settings popover. The regular layout's minimum
        // width must stay under this breakpoint or the window could never switch.
        mseries_pantheon_kit::watch_responsive_width(&window, 76.0, {
            let split = split.clone();
            let main_split = main_split.clone();
            let sidebar = sidebar.clone();
            let inspector = inspector.panel.clone();
            let settings_button = settings_button.clone();
            let settings_popover = settings_popover.clone();
            let stack = stack.clone();
            move |class| {
                let (sidebar_width, inspector_width) = match class {
                    mseries_pantheon_kit::LayoutClass::Compact => (224, 340),
                    mseries_pantheon_kit::LayoutClass::Regular => (224, 340),
                };
                let compact = matches!(class, mseries_pantheon_kit::LayoutClass::Compact);
                if compact && main_split.end_child().is_some() {
                    main_split.set_end_child(None::<&gtk::Widget>);
                    settings_popover.set_child(Some(&inspector));
                } else if !compact && main_split.end_child().is_none() {
                    settings_popover.popdown();
                    settings_popover.set_child(None::<&gtk::Widget>);
                    main_split.set_end_child(Some(&inspector));
                }
                settings_button.set_visible(
                    compact && stack.visible_child_name().as_deref() == Some("create"),
                );
                sidebar.set_width_request(sidebar_width);
                inspector.set_width_request(inspector_width);
                if split.width() > 0 {
                    split.set_position(sidebar_width);
                }
                if main_split.width() > 0 {
                    main_split.set_position((main_split.width() - inspector_width).max(420));
                }
            }
        });

        wire_actions(
            application,
            &window,
            &stack,
            &create,
            &library,
            &inspector,
            &recipe_store,
        );
        refresh_hardware(&inspector);
        let refresh_inspector = inspector.clone_refs();
        glib::timeout_add_seconds_local(5, move || {
            refresh_hardware(&refresh_inspector);
            glib::ControlFlow::Continue
        });
        load_recipes(&library.recipes, &create, &inspector, &stack, &recipe_store);
        load_generated_images(&library, &create, &inspector, &stack);
        let result_library = library.clone_refs();
        let result_create = create.clone_refs();
        let result_inspector = inspector.clone_refs();
        let result_stack = stack.clone();
        let mut previous_job = String::new();
        glib::timeout_add_seconds_local(1, move || {
            if result_library.page.root().is_none() {
                return glib::ControlFlow::Break;
            }
            let path = glib::user_data_dir().join("mseries-image-studio/current-job.json");
            if let Ok(raw) = std::fs::read_to_string(path) {
                if raw != previous_job {
                    if let Ok(job) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if job["running"].as_bool() == Some(false)
                            && job["status"]
                                .as_str()
                                .is_some_and(|s| s.starts_with("Complete"))
                        {
                            load_generated_images(
                                &result_library,
                                &result_create,
                                &result_inspector,
                                &result_stack,
                            );
                        }
                    }
                    previous_job = raw;
                }
            }
            glib::ControlFlow::Continue
        });
        if std::env::var_os("M_SERIES_IMAGE_STUDIO_TEST_WORKFLOW").is_none() {
            reload_last(true);
        }
        test_hooks(
            application,
            &window,
            &stack,
            &create,
            &library,
            &inspector,
            &recipe_store,
        );

        Self { window }
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }
}

/// Everything the canvas needs to show one image or one status.
#[derive(Clone)]
struct Stage {
    frame: gtk::AspectFrame,
    picture: gtk::Picture,
    empty: gtk::Box,
    empty_title: gtk::Label,
    empty_detail: gtk::Label,
    status: gtk::Label,
    spinner: gtk::Spinner,
    progress: gtk::ProgressBar,
    details: gtk::Label,
    details_hud: gtk::Box,
}

impl Stage {
    /// Show an image on the canvas. Returns false when the file cannot be decoded.
    fn show(&self, path: &str) -> bool {
        match gdk::Texture::from_filename(path) {
            Ok(texture) => {
                let ratio = texture.width() as f32 / texture.height().max(1) as f32;
                self.frame.set_ratio(ratio);
                self.picture.set_paintable(Some(&texture));
                self.frame.set_visible(true);
                self.empty.set_visible(false);
                true
            }
            Err(_) => {
                self.clear();
                false
            }
        }
    }

    fn clear(&self) {
        self.picture.set_paintable(None::<&gdk::Paintable>);
        self.frame.set_visible(false);
        self.empty.set_visible(true);
    }

    fn set_details(&self, text: &str) {
        self.details.set_text(text);
        self.details_hud.set_visible(!text.is_empty());
        self.details.set_tooltip_text(Some(text));
    }
}

#[derive(Clone)]
struct CreatePage {
    page: gtk::Widget,
    prompt: gtk::TextView,
    negative: gtk::TextView,
    intent: gtk::DropDown,
    suggest_type: gtk::Button,
    help_prompt: gtk::Button,
    assistant_status: gtk::Label,
    workflow_status: gtk::Label,
    workflow: Rc<RefCell<String>>,
    stage: Stage,
    film: gtk::Box,
    film_rail: gtk::Widget,
    dock_actions: gtk::Box,
    chips: gtk::FlowBox,
    run_summary: gtk::Label,
}

#[derive(Clone)]
struct LibraryDetail {
    root: gtk::Box,
    content: gtk::Box,
}

#[derive(Clone)]
struct LibraryPage {
    page: gtk::Widget,
    recipes: gtk::ListBox,
    flow: gtk::FlowBox,
    placeholder: granite::Placeholder,
    gallery: gtk::Widget,
    subtitle: gtk::Label,
    detail: LibraryDetail,
    items: Rc<RefCell<Vec<Recipe>>>,
}

#[derive(Clone)]
struct Inspector {
    panel: gtk::Widget,
    engine: gtk::Label,
    engine_card: gtk::Box,
    engine_dot: gtk::Box,
    model: gtk::Entry,
    lora: gtk::Entry,
    lora_weight: gtk::Scale,
    dimensions: gtk::DropDown,
    batch: gtk::SpinButton,
    seed: gtk::SpinButton,
    steps: gtk::SpinButton,
    cfg: gtk::SpinButton,
    sampler: gtk::DropDown,
    vae: gtk::Entry,
    vae_on_cpu: gtk::CheckButton,
    reference_image: gtk::Entry,
    reference_strength: gtk::SpinButton,
    vae_tile_size: gtk::SpinButton,
    cpu_threads: gtk::SpinButton,
    preview_interval: gtk::SpinButton,
    params_backend: gtk::DropDown,
    max_vram_gib: gtk::SpinButton,
    scheduler: gtk::DropDown,
    tags: gtk::Entry,
    generate: gtk::Button,
    save: gtk::Button,
    recipe_status: gtk::Label,
}

impl CreatePage {
    fn clone_refs(&self) -> Self {
        self.clone()
    }
}
impl LibraryPage {
    fn clone_refs(&self) -> Self {
        self.clone()
    }
}
impl Inspector {
    fn clone_refs(&self) -> Self {
        self.clone()
    }
}

fn short_model_name(value: &str) -> String {
    if value.is_empty() {
        return "Checkpoint default".into();
    }
    let file = Path::new(value)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    if file.contains("Realistic_Vision") {
        "Realistic Vision 6.0 · FP16".into()
    } else if file.contains("vae-ft-mse") {
        "MSE 840000".into()
    } else {
        file.replace('_', " ")
    }
}

const SAMPLERS: [&str; 4] = ["Euler", "Euler ancestral", "DPM++ 2M", "DPM++ 2M SDE"];
const SIZES: [&str; 7] = [
    "512 × 512",
    "768 × 768",
    "1024 × 1024",
    "512 × 768",
    "768 × 512",
    "768 × 1024",
    "1024 × 768",
];

/// "3 min ago", "Today 14:03", "Oct 21, 09:12": a quiet, human timestamp.
fn when(unix: u64) -> String {
    let (Ok(then), Ok(now)) = (
        glib::DateTime::from_unix_local(unix as i64),
        glib::DateTime::now_local(),
    ) else {
        return String::new();
    };
    let seconds = now.to_unix() - then.to_unix();
    let format = |pattern: &str| {
        then.format(pattern)
            .map(|text| text.to_string())
            .unwrap_or_default()
    };
    if (0..60).contains(&seconds) {
        "Just now".into()
    } else if (60..3600).contains(&seconds) {
        format!("{} min ago", seconds / 60)
    } else if now.ymd() == then.ymd() {
        format("Today %H:%M")
    } else {
        format("%b %-d, %H:%M")
    }
}

fn nav_row(page: &str, label: &str, icon: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("is-nav-row");
    row.set_widget_name(page);
    let contents = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    contents.set_margin_start(6);
    contents.set_margin_end(6);
    contents.append(&gtk::Image::from_icon_name(icon));
    let row_label = gtk::Label::new(Some(label));
    row_label.set_xalign(0.0);
    row_label.set_hexpand(true);
    contents.append(&row_label);
    row.set_child(Some(&contents));
    row
}

fn nav_header(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("is-nav-header");
    row.set_selectable(false);
    row.set_activatable(false);
    let label = style::section_caption(text);
    label.set_margin_top(14);
    label.set_margin_bottom(4);
    label.set_margin_start(12);
    row.set_child(Some(&label));
    row
}

fn build_sidebar(stack: &gtk::Stack, create: &CreatePage, inspector: &Inspector) -> gtk::Widget {
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.set_width_request(224);
    sidebar.add_css_class("is-sidebar");
    sidebar.add_css_class(granite::STYLE_CLASS_SIDEBAR);
    let navigation = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(true)
        .build();
    mseries_pantheon_kit::set_accessible_label(&navigation, "Image Studio pages");
    navigation.append(&nav_header("Studio"));
    for (page, label, icon) in STUDIO_PAGES {
        let row = nav_row(page, label, icon);
        navigation.append(&row);
        if *page == "create" {
            navigation.select_row(Some(&row));
        }
    }
    navigation.append(&nav_header("Setup"));
    for (page, label, icon) in SETUP_PAGES {
        navigation.append(&nav_row(page, label, icon));
    }
    let stack_for_navigation = stack.clone();
    navigation.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let name = row.widget_name();
            if stack_for_navigation.visible_child_name().as_deref() != Some(name.as_str()) {
                stack_for_navigation.set_visible_child_name(name.as_str());
            }
        }
    });
    let navigation_weak = navigation.downgrade();
    stack.connect_visible_child_name_notify(move |stack| {
        let Some(navigation) = navigation_weak.upgrade() else {
            return;
        };
        let current = stack.visible_child_name();
        let mut child = navigation.first_child();
        while let Some(row) = child {
            if let Some(row) = row.downcast_ref::<gtk::ListBoxRow>() {
                if row.is_selectable() && Some(row.widget_name()) == current {
                    navigation.select_row(Some(row));
                    break;
                }
            }
            child = row.next_sibling();
        }
    });
    sidebar.append(&navigation);

    let workflows = style::section_caption("Quick starts");
    workflows.set_margin_start(20);
    workflows.set_margin_top(18);
    workflows.set_margin_bottom(4);
    sidebar.append(&workflows);
    let quick = gtk::Box::new(gtk::Orientation::Vertical, 2);
    quick.set_margin_start(8);
    quick.set_margin_end(8);
    for (title, subtitle, workflow, icon) in [
        (
            "Model readiness",
            "One conservative test render",
            "Model readiness",
            "emblem-ok-symbolic",
        ),
        (
            "Prompt type study",
            "Compare prompt structures",
            "Prompt type study",
            "document-edit-symbolic",
        ),
    ] {
        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.add_css_class("is-quick");
        let contents = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("is-dim");
        contents.append(&image);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let name = gtk::Label::new(Some(title));
        name.set_xalign(0.0);
        text.append(&name);
        text.append(&caption(subtitle));
        contents.append(&text);
        button.set_child(Some(&contents));
        button.update_property(&[gtk::accessible::Property::Label(title)]);
        let page = create.clone_refs();
        let settings = inspector.clone_refs();
        let stack = stack.clone();
        button.connect_clicked(move |_| {
            stack.set_visible_child_name("create");
            apply_workflow(workflow, &page, &settings);
        });
        quick.append(&button);
    }
    sidebar.append(&quick);

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);

    // This device: a memory meter that shows headroom above the protected desktop reserve.
    let device = gtk::Box::new(gtk::Orientation::Vertical, 8);
    device.add_css_class("is-card");
    device.set_margin_start(12);
    device.set_margin_end(12);
    device.set_margin_bottom(12);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inner.set_margin_top(10);
    inner.set_margin_bottom(10);
    inner.set_margin_start(12);
    inner.set_margin_end(12);
    let device_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let device_dot = dot("");
    let device_title = gtk::Label::new(Some("This device"));
    device_title.add_css_class("is-card-title");
    device_title.set_xalign(0.0);
    device_title.set_hexpand(true);
    device_head.append(&device_dot);
    device_head.append(&device_title);
    inner.append(&device_head);
    let meter = gtk::LevelBar::builder()
        .min_value(0.0)
        .max_value(1.0)
        .mode(gtk::LevelBarMode::Continuous)
        .css_classes(["is-meter"])
        .build();
    meter.add_offset_value("low", 0.0);
    meter.update_property(&[gtk::accessible::Property::Label("Available memory")]);
    inner.append(&meter);
    let hardware = gtk::Label::new(None);
    hardware.set_xalign(0.0);
    hardware.set_wrap(true);
    hardware.add_css_class("is-caption");
    inner.append(&hardware);
    inner.append(&crate::models::storage_tiles(true));
    device.append(&inner);
    sidebar.append(&device);

    let refresh = {
        let meter = meter.clone();
        let hardware = hardware.clone();
        let device_dot = device_dot.clone();
        move || {
            let host = HostCapabilities::capture();
            let available = host.available_gib();
            // Scale the meter to a 16 GiB window so headroom changes are visible.
            meter.set_value((available / 16.0).clamp(0.0, 1.0));
            style::set_tone(
                &device_dot,
                if host.reserve_met() {
                    "is-ok"
                } else {
                    "is-warn"
                },
            );
            hardware.set_text(&format!(
                "{} · 6 GiB desktop reserve {}\n{}",
                host.memory_summary(),
                if host.reserve_met() { "met" } else { "not met" },
                if host.has_render_node {
                    "Render node detected"
                } else {
                    "No render node detected"
                }
            ));
        }
    };
    refresh();
    let weak_hardware = hardware.downgrade();
    glib::timeout_add_seconds_local(5, move || {
        if weak_hardware.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        refresh();
        glib::ControlFlow::Continue
    });
    let _ = inspector;
    sidebar.upcast()
}

/// Live summary of the next run as chips, rebuilt only when it changes.
fn install_recipe_chips(create: &CreatePage, settings: &Inspector) {
    let flow = create.chips.clone();
    let run_summary = create.run_summary.clone();
    let settings = settings.clone_refs();
    let mut previous: Vec<(String, &'static str, Option<&'static str>)> = Vec::new();
    glib::timeout_add_local(Duration::from_millis(300), move || {
        if flow.root().is_none() {
            return glib::ControlFlow::Continue;
        }
        let mut chips: Vec<(String, &'static str, Option<&'static str>)> = vec![
            (
                short_model_name(settings.model.text().as_str()),
                "is-accent",
                Some("drive-harddisk-symbolic"),
            ),
            (
                SIZES
                    .get(settings.dimensions.selected() as usize)
                    .unwrap_or(&"Unknown size")
                    .to_string(),
                "",
                None,
            ),
            (
                format!(
                    "{} · {}",
                    SAMPLERS
                        .get(settings.sampler.selected() as usize)
                        .unwrap_or(&"Unknown sampler"),
                    if settings.scheduler.selected() == 1 {
                        "Karras"
                    } else {
                        "Discrete"
                    }
                ),
                "",
                None,
            ),
            (format!("{} steps", settings.steps.value_as_int()), "", None),
            (format!("CFG {:.1}", settings.cfg.value()), "", None),
            (
                if settings.seed.value_as_int() < 0 {
                    "Random seed".into()
                } else {
                    format!("Seed {}", settings.seed.value_as_int())
                },
                "",
                None,
            ),
        ];
        run_summary.set_text(&format!(
            "VAE {} · {}px tiles · weights {} · GPU budget {:.1} GiB · preview every {} steps",
            if settings.vae_on_cpu.is_active() {
                "CPU"
            } else {
                "GPU"
            },
            settings.vae_tile_size.value_as_int(),
            ["disk", "cpu", "auto"]
                .get(settings.params_backend.selected() as usize)
                .unwrap_or(&"disk"),
            settings.max_vram_gib.value(),
            settings.preview_interval.value_as_int()
        ));
        if settings.batch.value_as_int() > 1 {
            chips.push((
                format!("{} images", settings.batch.value_as_int()),
                "",
                None,
            ));
        }
        let reference = settings.reference_image.text();
        if !reference.is_empty() {
            chips.push((
                format!("Reference {:.2}", settings.reference_strength.value()),
                "",
                Some("image-x-generic-symbolic"),
            ));
        }
        if !settings.lora.text().is_empty() {
            chips.push((
                format!("LoRA {:.2}", settings.lora_weight.value()),
                "",
                None,
            ));
        }
        if chips != previous {
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
            for (text, tone, icon) in &chips {
                flow.append(&chip(text, tone, *icon));
            }
            previous = chips;
        }
        glib::ControlFlow::Continue
    });
}

fn build_create_page() -> CreatePage {
    let queue_status = gtk::Label::new(Some("Ready"));
    queue_status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    queue_status.set_max_width_chars(48);

    // ----- Canvas -----
    let stage_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stage_box.add_css_class("is-stage");
    stage_box.set_hexpand(true);
    stage_box.set_vexpand(true);
    stage_box.set_size_request(-1, 240);
    stage_box.set_overflow(gtk::Overflow::Hidden);

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_can_shrink(true);
    picture.add_css_class("is-stage-frame");
    picture.set_overflow(gtk::Overflow::Hidden);
    let frame = gtk::AspectFrame::new(0.5, 0.5, 1.0, false);
    frame.set_child(Some(&picture));
    frame.set_hexpand(true);
    frame.set_vexpand(true);
    frame.set_margin_top(28);
    frame.set_margin_bottom(28);
    frame.set_margin_start(76);
    frame.set_margin_end(28);
    frame.set_visible(false);

    let empty = gtk::Box::new(gtk::Orientation::Vertical, 10);
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_vexpand(true);
    empty.set_hexpand(true);
    let empty_icon = gtk::Image::from_icon_name("applications-graphics-symbolic");
    empty_icon.set_pixel_size(64);
    empty_icon.add_css_class("is-empty-icon");
    let empty_title = gtk::Label::new(Some("Describe an image"));
    empty_title.add_css_class("is-empty-title");
    empty_title.set_wrap(true);
    empty_title.set_justify(gtk::Justification::Center);
    empty_title.set_max_width_chars(48);
    let empty_detail = gtk::Label::new(Some(
        "Write a prompt below and press Generate. Live denoising previews appear here as each step finishes.",
    ));
    empty_detail.set_wrap(true);
    empty_detail.set_justify(gtk::Justification::Center);
    empty_detail.set_max_width_chars(58);
    empty_detail.set_lines(4);
    empty_detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    empty_detail.add_css_class("is-dim");
    empty.append(&empty_icon);
    empty.append(&empty_title);
    empty.append(&empty_detail);

    stage_box.append(&frame);
    stage_box.append(&empty);

    let stage_overlay = gtk::Overlay::new();
    stage_overlay.add_css_class("is-stage-clip");
    stage_overlay.set_overflow(gtk::Overflow::Hidden);
    stage_overlay.set_child(Some(&stage_box));
    stage_overlay.set_vexpand(true);
    stage_overlay.set_hexpand(true);

    // Progress hairline along the bottom edge of the canvas.
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("is-stage-progress");
    progress.set_valign(gtk::Align::End);
    progress.set_visible(false);
    stage_overlay.add_overlay(&progress);

    // Status HUD (top left)
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    let hud = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hud.add_css_class("is-hud");
    hud.set_halign(gtk::Align::Start);
    hud.set_valign(gtk::Align::Start);
    hud.set_margin_start(14);
    hud.set_margin_top(14);
    hud.append(&spinner);
    hud.append(&queue_status);
    stage_overlay.add_overlay(&hud);

    // Run details HUD (top right)
    let details = gtk::Label::new(None);
    details.set_ellipsize(gtk::pango::EllipsizeMode::End);
    details.set_max_width_chars(60);
    details.add_css_class("is-caption");
    let details_hud = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    details_hud.add_css_class("is-hud");
    details_hud.set_halign(gtk::Align::End);
    details_hud.set_valign(gtk::Align::Start);
    details_hud.set_margin_end(14);
    details_hud.set_margin_top(14);
    details_hud.set_visible(false);
    details_hud.append(&details);
    stage_overlay.add_overlay(&details_hud);

    // History filmstrip (left edge)
    let film = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let film_scroll = gtk::ScrolledWindow::builder()
        .child(&film)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(520)
        .build();
    let film_rail = gtk::Box::new(gtk::Orientation::Vertical, 0);
    film_rail.add_css_class("is-hud-rail");
    film_rail.set_halign(gtk::Align::Start);
    film_rail.set_valign(gtk::Align::Center);
    film_rail.set_margin_start(14);
    film_rail.set_margin_top(60);
    film_rail.set_margin_bottom(14);
    film_rail.set_visible(false);
    film_rail.append(&film_scroll);
    stage_overlay.add_overlay(&film_rail);

    let stage = Stage {
        frame,
        picture,
        empty,
        empty_title: empty_title.clone(),
        empty_detail: empty_detail.clone(),
        status: queue_status.clone(),
        spinner,
        progress,
        details,
        details_hud,
    };
    install_job_watch(&stage);

    // ----- Prompt dock -----
    let dock = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dock.add_css_class("is-dock");

    let prompt = gtk::TextView::new();
    prompt.set_wrap_mode(gtk::WrapMode::WordChar);
    prompt.set_left_margin(16);
    prompt.set_right_margin(16);
    prompt.set_top_margin(14);
    prompt.set_bottom_margin(10);
    prompt.buffer().set_text(SAMPLE_SERIES);
    prompt.update_property(&[gtk::accessible::Property::Label("Image prompt")]);
    let prompt_scroll = gtk::ScrolledWindow::builder()
        .child(&prompt)
        .min_content_height(64)
        .max_content_height(150)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    prompt_scroll.add_css_class("is-prompt");
    dock.append(&prompt_scroll);

    let negative = gtk::TextView::new();
    negative.set_wrap_mode(gtk::WrapMode::WordChar);
    negative.set_left_margin(10);
    negative.set_right_margin(10);
    negative.set_top_margin(8);
    negative.set_bottom_margin(8);
    negative
        .buffer()
        .set_text("text, watermark, logo, extra fingers, inconsistent person, sexualized styling");
    negative.update_property(&[gtk::accessible::Property::Label("Negative prompt")]);
    let negative_scroll = gtk::ScrolledWindow::builder()
        .child(&negative)
        .min_content_height(44)
        .max_content_height(80)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    negative_scroll.add_css_class("is-field");
    let negative_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    negative_box.set_margin_start(14);
    negative_box.set_margin_end(14);
    negative_box.set_margin_bottom(10);
    negative_box.append(
        &gtk::Label::builder()
            .label("Negative prompt · what to avoid")
            .xalign(0.0)
            .css_classes(["is-field-label"])
            .build(),
    );
    negative_box.append(&negative_scroll);
    let negative_revealer = gtk::Revealer::builder()
        .child(&negative_box)
        .reveal_child(false)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(160)
        .build();
    dock.append(&negative_revealer);
    dock.append(&{
        let divider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        divider.add_css_class("is-dock-divider");
        divider
    });

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.set_margin_top(8);
    actions.set_margin_bottom(8);
    actions.set_margin_start(10);
    actions.set_margin_end(10);
    let intent = gtk::DropDown::from_strings(INTENTS);
    intent.set_selected(1);
    intent.add_css_class("is-pill");
    intent.set_tooltip_text(Some("Prompt type"));
    intent.update_property(&[gtk::accessible::Property::Label("Prompt type")]);
    actions.append(&intent);
    // Secondary actions are icon-only so the dock stays compact; each has a tooltip
    // and an accessible name.
    let suggest_type = gtk::Button::from_icon_name("tag-new-symbolic");
    suggest_type.add_css_class("flat");
    suggest_type.add_css_class("is-pill");
    suggest_type.set_tooltip_text(Some(
        "Suggest type: ask the already-running M-Series local assistant to recommend a prompt type",
    ));
    suggest_type.update_property(&[gtk::accessible::Property::Label("Suggest type")]);
    let help_prompt = gtk::Button::from_icon_name("starred-symbolic");
    help_prompt.add_css_class("flat");
    help_prompt.add_css_class("is-pill");
    help_prompt.set_tooltip_text(Some(
        "Improve prompt: ask the already-running local assistant for an editable suggestion. It uses 127.0.0.1:8766 and never starts or replaces a worker.",
    ));
    help_prompt.update_property(&[gtk::accessible::Property::Label("Improve prompt")]);
    let negative_toggle = gtk::ToggleButton::new();
    negative_toggle.set_icon_name("action-unavailable-symbolic");
    negative_toggle.add_css_class("flat");
    negative_toggle.add_css_class("is-pill");
    negative_toggle.set_tooltip_text(Some("Negative prompt: describe what to avoid"));
    negative_toggle.update_property(&[gtk::accessible::Property::Label("Negative prompt")]);
    let negative_revealer_for_toggle = negative_revealer.clone();
    negative_toggle.connect_toggled(move |toggle| {
        negative_revealer_for_toggle.set_reveal_child(toggle.is_active());
    });
    actions.append(&suggest_type);
    actions.append(&help_prompt);
    actions.append(&negative_toggle);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    let dock_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&dock_actions);
    dock.append(&actions);

    // Hints under the dock: only visible when there is something to say.
    let workflow_status = gtk::Label::new(None);
    let assistant_status = gtk::Label::new(None);
    let hints = gtk::Box::new(gtk::Orientation::Vertical, 2);
    hints.set_margin_start(6);
    for label in [&workflow_status, &assistant_status] {
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("is-caption");
        label.set_visible(false);
        label.connect_label_notify(|label| label.set_visible(!label.label().is_empty()));
        hints.append(label);
    }

    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_homogeneous(false);
    chips.set_min_children_per_line(1);
    chips.set_max_children_per_line(12);
    chips.set_column_spacing(6);
    chips.set_row_spacing(6);
    chips.set_margin_start(2);
    chips.update_property(&[gtk::accessible::Property::Label(
        "Settings for the next run",
    )]);

    let run_summary = caption("");
    run_summary.set_margin_start(6);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(14);
    page.set_margin_bottom(16);
    page.set_margin_start(16);
    page.set_margin_end(16);
    page.append(&stage_overlay);
    page.append(&dock);
    page.append(&chips);
    page.append(&run_summary);
    page.append(&hints);

    CreatePage {
        page: gtk::ScrolledWindow::builder()
            .child(&page)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build()
            .upcast(),
        prompt,
        negative,
        intent,
        suggest_type,
        help_prompt,
        assistant_status,
        workflow_status,
        workflow: Rc::new(RefCell::new("Prompt type study".into())),
        stage,
        film,
        film_rail: film_rail.upcast(),
        dock_actions,
        chips,
        run_summary,
    }
}

/// Mirror `current-job.json` onto the canvas: status, live preview, progress, details.
/// The first sync is immediate so the canvas never opens on stale text.
fn install_job_watch(stage: &Stage) {
    let job_path = gtk::glib::user_data_dir().join("mseries-image-studio/current-job.json");
    let mut last_job = String::new();
    let stage = stage.clone();
    let mut first = true;
    let mut sync = move || -> gtk::glib::ControlFlow {
        // After the first call the widget is parented; a missing root means the window closed.
        if !first && stage.empty_title.root().is_none() {
            return gtk::glib::ControlFlow::Break;
        }
        first = false;
        let Ok(raw) = std::fs::read_to_string(&job_path) else {
            return gtk::glib::ControlFlow::Continue;
        };
        if raw == last_job {
            return gtk::glib::ControlFlow::Continue;
        }
        if let Ok(job) = serde_json::from_str::<serde_json::Value>(&raw) {
            let status = job["status"]
                .as_str()
                .unwrap_or("Generation status unavailable");
            stage.status.set_text(status);
            stage.empty_title.set_text(status);
            stage.empty_detail.set_text(&format!(
                "{}\n{}",
                job["engine"].as_str().unwrap_or("Engine unavailable"),
                job["prompt"].as_str().unwrap_or("")
            ));
            let path = job["image_path"].as_str().unwrap_or("");
            let shown = !path.is_empty() && stage.show(path);
            if !shown {
                stage.clear();
            }
            let running = job["running"].as_bool().unwrap_or(false);
            stage.spinner.set_visible(running);
            stage.spinner.set_spinning(running);
            let step = job["step"].as_f64().unwrap_or(0.0);
            let steps = job["steps"].as_f64().unwrap_or(0.0);
            stage
                .progress
                .set_visible(running || steps > 0.0 && step < steps);
            if steps > 0.0 {
                stage.progress.set_fraction((step / steps).clamp(0.0, 1.0));
            } else if running {
                stage.progress.pulse();
            } else {
                stage.progress.set_fraction(if shown { 1.0 } else { 0.0 });
            }
            if running && steps > 0.0 {
                stage
                    .status
                    .set_text(&format!("{status} · step {step:.0} of {steps:.0}"));
            }
            let mut parts = Vec::new();
            if let Some(model) = job["model"].as_str() {
                parts.push(short_model_name(model));
            }
            if let Some(sampler) = job["sampler"].as_str() {
                parts.push(sampler.to_string());
            }
            if let Some(scheduler) = job["scheduler"].as_str() {
                parts.push(scheduler.to_string());
            }
            if let Some(seconds) = job["elapsed_seconds"].as_f64() {
                parts.push(format!("{seconds:.1} s"));
            }
            if let Some(previews) = job["preview_count"].as_u64() {
                parts.push(format!("{previews} previews"));
            }
            stage.set_details(&parts.join(" · "));
        }
        last_job = raw;
        gtk::glib::ControlFlow::Continue
    };
    sync();
    gtk::glib::timeout_add_seconds_local(1, sync);
}

fn current_recipe(page: &CreatePage, settings: &Inspector) -> Recipe {
    let mut recipe = Recipe::default();
    recipe.prompt = text_of(&page.prompt);
    recipe.negative_prompt = text_of(&page.negative);
    recipe.intent = INTENTS
        .get(page.intent.selected() as usize)
        .copied()
        .unwrap_or("Portrait")
        .into();
    recipe.workflow = page.workflow.borrow().clone();
    recipe.model_id = settings.model.text().to_string();
    recipe.steps = settings.steps.value_as_int() as u32;
    recipe.cfg_scale = settings.cfg.value();
    recipe.sampler = ["euler", "euler_a", "dpm++2m", "dpm++2m_sde"]
        .get(settings.sampler.selected() as usize)
        .unwrap_or(&"euler")
        .to_string();
    recipe.vae_path = settings.vae.text().to_string();
    recipe.vae_on_cpu = settings.vae_on_cpu.is_active();
    recipe.reference_image = settings.reference_image.text().to_string();
    recipe.reference_strength = settings.reference_strength.value();
    recipe.vae_tile_size = settings.vae_tile_size.value_as_int() as u32;
    recipe.cpu_threads = settings.cpu_threads.value_as_int() as u32;
    recipe.preview_interval = settings.preview_interval.value_as_int() as u32;
    recipe.params_backend = ["disk", "cpu", "auto"]
        .get(settings.params_backend.selected() as usize)
        .unwrap_or(&"disk")
        .to_string();
    recipe.max_vram_gib = settings.max_vram_gib.value();
    recipe.scheduler = if settings.scheduler.selected() == 1 {
        "karras"
    } else {
        "discrete"
    }
    .into();
    recipe.lora_id = settings.lora.text().to_string();
    recipe.lora_weight = settings.lora_weight.value();
    (recipe.width, recipe.height) = match settings.dimensions.selected() {
        1 => (768, 768),
        2 => (1024, 1024),
        3 => (512, 768),
        4 => (768, 512),
        5 => (768, 1024),
        6 => (1024, 768),
        _ => (512, 512),
    };
    recipe.batch_size = settings.batch.value_as_int().clamp(1, 8) as u32;
    recipe.seed = settings.seed.value_as_int() as i64;
    recipe.tags = settings
        .tags
        .text()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if recipe.tags.is_empty() {
        recipe.tags = vec![
            "mseries-image-studio".into(),
            recipe.intent.to_lowercase().replace(' ', "-"),
        ];
    }
    recipe
}

fn load_recipe(recipe: &Recipe, page: &CreatePage, settings: &Inspector) {
    set_text(&page.prompt, &recipe.prompt);
    set_text(&page.negative, &recipe.negative_prompt);
    *page.workflow.borrow_mut() = recipe.workflow.clone();
    page.workflow_status.set_text(&format!(
        "Loaded {} · editing this copy will not change the saved recipe.",
        recipe.workflow
    ));
    if let Some(index) = INTENTS.iter().position(|intent| *intent == recipe.intent) {
        page.intent.set_selected(index as u32);
    }
    let installed = gtk::glib::user_data_dir()
        .join("mseries-image-studio/models/stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf");
    if recipe.engine_id.contains("hosted") && installed.is_file() {
        settings.model.set_text(&installed.to_string_lossy());
    } else {
        settings.model.set_text(&recipe.model_id);
    }
    settings.steps.set_value(recipe.steps as f64);
    settings.cfg.set_value(recipe.cfg_scale);
    settings.sampler.set_selected(
        ["euler", "euler_a", "dpm++2m", "dpm++2m_sde"]
            .iter()
            .position(|sampler| *sampler == recipe.sampler)
            .unwrap_or(0) as u32,
    );
    settings.vae.set_text(&recipe.vae_path);
    settings.vae_on_cpu.set_active(recipe.vae_on_cpu);
    settings.reference_image.set_text(&recipe.reference_image);
    settings
        .reference_strength
        .set_value(recipe.reference_strength);
    settings
        .vae_tile_size
        .set_value(recipe.vae_tile_size as f64);
    settings.cpu_threads.set_value(recipe.cpu_threads as f64);
    settings
        .preview_interval
        .set_value(recipe.preview_interval as f64);
    settings.params_backend.set_selected(
        ["disk", "cpu", "auto"]
            .iter()
            .position(|backend| *backend == recipe.params_backend)
            .unwrap_or(0) as u32,
    );
    settings.max_vram_gib.set_value(recipe.max_vram_gib);
    settings
        .scheduler
        .set_selected(if recipe.scheduler == "karras" { 1 } else { 0 });
    settings.lora.set_text(&recipe.lora_id);
    settings.lora_weight.set_value(recipe.lora_weight);
    settings
        .dimensions
        .set_selected(match (recipe.width, recipe.height) {
            (512, 768) => 3,
            (768, 512) => 4,
            (768, 1024) => 5,
            (1024, 768) => 6,
            (768, 768) => 1,
            (1024, 1024) => 2,
            _ => 0,
        });
    settings
        .batch
        .set_value(recipe.batch_size.clamp(1, 8) as f64);
    settings
        .seed
        .set_value(recipe.seed.clamp(-1, i32::MAX as i64) as f64);
    settings.tags.set_text(&recipe.tags.join(", "));
    settings
        .recipe_status
        .set_text("Recipe loaded from Library.");
}

fn load_recipes(
    list: &gtk::ListBox,
    page: &CreatePage,
    settings: &Inspector,
    stack: &gtk::Stack,
    store: &RecipeStore,
) {
    match store.load_recent(100) {
        Ok(recipes) => {
            for recipe in recipes {
                add_recipe_row(list, recipe, page, settings, stack, false);
            }
        }
        Err(error) => eprintln!("Could not load image recipes: {error}"),
    }
}

fn kv(key: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 1);
    row.append(
        &gtk::Label::builder()
            .label(key)
            .xalign(0.0)
            .css_classes(["is-kv-key"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(value)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .css_classes(["is-kv-val"])
            .build(),
    );
    row
}

fn build_library_page(store: &RecipeStore) -> LibraryPage {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // Header: title + count on the left, Images | Recipes switcher on the right.
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.set_margin_top(22);
    header.set_margin_start(26);
    header.set_margin_end(26);
    header.set_margin_bottom(14);
    let title = style::page_header("Library", "");
    title.set_hexpand(true);
    let subtitle = caption("Every image keeps its prompt, model, LoRAs and settings beside it.");
    title.append(&subtitle);
    header.append(&title);
    let content = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let switcher = gtk::StackSwitcher::builder().stack(&content).build();
    switcher.set_valign(gtk::Align::Center);
    header.append(&switcher);
    page.append(&header);

    // Images: gallery + detail
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(10)
        .column_spacing(4)
        .row_spacing(4)
        .activate_on_single_click(true)
        .valign(gtk::Align::Start)
        .css_classes(["is-gallery"])
        .build();
    flow.set_margin_start(20);
    flow.set_margin_end(20);
    flow.set_margin_bottom(20);
    flow.update_property(&[gtk::accessible::Property::Label("Generated images")]);
    let grid_scroll = gtk::ScrolledWindow::builder()
        .child(&flow)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    let placeholder = granite::Placeholder::new("No generated images yet");
    placeholder.set_description(
        "Images you generate appear here with everything needed to repeat them. Saved recipes are in the Recipes tab.",
    );
    placeholder.set_icon(&gio::ThemedIcon::new("folder-pictures-symbolic"));
    placeholder.set_hexpand(true);
    placeholder.set_vexpand(true);
    let gallery = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .build();
    gallery.add_named(&grid_scroll, Some("grid"));
    gallery.add_named(&placeholder, Some("empty"));
    gallery.set_visible_child_name("empty");

    let detail_content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    detail_content.set_margin_top(18);
    detail_content.set_margin_bottom(18);
    detail_content.set_margin_start(16);
    detail_content.set_margin_end(16);
    let detail_scroll = gtk::ScrolledWindow::builder()
        .child(&detail_content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let detail_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    detail_root.add_css_class("is-detail");
    detail_root.set_width_request(360);
    detail_root.append(&detail_scroll);
    detail_root.set_visible(false);
    let images = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    images.append(&gallery);
    images.append(&detail_root);
    content.add_titled(&images, Some("images"), "Images");

    // Recipes
    let recipes = gtk::ListBox::new();
    recipes.set_selection_mode(gtk::SelectionMode::None);
    recipes.add_css_class("is-recipes");
    let recipe_placeholder = granite::Placeholder::new("No saved recipes yet");
    recipe_placeholder.set_description(
        "Use Save recipe in the inspector to keep a prompt and its settings for later.",
    );
    recipe_placeholder.set_icon(&gio::ThemedIcon::new("document-save-symbolic"));
    recipes.set_placeholder(Some(&recipe_placeholder));
    recipes.set_margin_start(20);
    recipes.set_margin_end(20);
    recipes.set_margin_bottom(20);
    let recipe_scroll = gtk::ScrolledWindow::builder()
        .child(&recipes)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    content.add_titled(&recipe_scroll, Some("recipes"), "Recipes");
    page.append(&content);
    let _ = store; // Store is passed in so page construction stays close to its library widgets.
    LibraryPage {
        page: page.upcast(),
        recipes,
        flow,
        placeholder,
        gallery: gallery.upcast(),
        subtitle,
        detail: LibraryDetail {
            root: detail_root,
            content: detail_content,
        },
        items: Rc::new(RefCell::new(Vec::new())),
    }
}

fn build_inspector() -> Inspector {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
    panel.set_margin_top(14);
    panel.set_margin_bottom(14);
    panel.set_margin_start(12);
    panel.set_margin_end(12);

    // Engine / readiness banner
    let engine_card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    engine_card.add_css_class("is-card");
    let engine_dot = dot("");
    engine_dot.set_margin_start(12);
    engine_dot.set_valign(gtk::Align::Start);
    engine_dot.set_margin_top(14);
    let engine = gtk::Label::new(Some(
        "Image engine: not connected\nCompatibility: unverified",
    ));
    engine.set_xalign(0.0);
    engine.set_wrap(true);
    engine.set_margin_top(10);
    engine.set_margin_bottom(10);
    engine.set_margin_end(12);
    engine.add_css_class("is-caption");
    engine_card.append(&engine_dot);
    engine_card.append(&engine);
    panel.append(&engine_card);

    // ----- Model -----
    let (model_section, model_body) = style::collapsible("Model", "drive-harddisk-symbolic", true);
    let model = gtk::Entry::new();
    model.set_placeholder_text(Some("Model name or checkpoint"));
    model.update_property(&[gtk::accessible::Property::Label(
        "Model name or checkpoint for recipe",
    )]);
    let installed = gtk::glib::user_data_dir()
        .join("mseries-image-studio/models/stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf");
    if installed.is_file() {
        model.set_text(&installed.to_string_lossy());
    }
    let installed_models = gtk::DropDown::from_strings(&[]);
    installed_models.set_hexpand(true);
    installed_models.set_tooltip_text(Some(
        "Installed image checkpoints. Full paths are available under Model file.",
    ));
    installed_models.update_property(&[gtk::accessible::Property::Label("Installed image model")]);
    let directory = gtk::glib::user_data_dir().join("mseries-image-studio/models");
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            path.is_file()
                && matches!(
                    name.as_ref(),
                    "stable-diffusion-v1-5-pruned-emaonly-q4_0.gguf"
                        | "stable-diffusion-v1-5-pruned-emaonly-q8_0.gguf"
                        | "sd_xl_turbo_1.0_fp16.safetensors"
                        | "realistic_vision_v6.0_nv_b1_fp16.safetensors"
                )
        })
        .collect();
    files.sort_by_key(|path| (!path.to_string_lossy().contains("Realistic"), path.clone()));
    let names: Vec<String> = files
        .iter()
        .map(|path| short_model_name(&path.to_string_lossy()))
        .collect();
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    installed_models.set_model(Some(&gtk::StringList::new(&labels)));
    let files = Rc::new(files);
    let paths = files.clone();
    let current_model = model.clone();
    installed_models.connect_selected_notify(move |choice| {
        if let Some(path) = paths.get(choice.selected() as usize) {
            if current_model.text().as_str() != path.to_string_lossy() {
                current_model.set_text(&path.to_string_lossy());
            }
        }
    });
    let choices = installed_models.downgrade();
    let paths = files.clone();
    model.connect_changed(move |entry| {
        if let Some(choices) = choices.upgrade() {
            let index = paths
                .iter()
                .position(|path| path.to_string_lossy() == entry.text().as_str())
                .map(|index| index as u32)
                .unwrap_or(gtk::INVALID_LIST_POSITION);
            if choices.selected() != index {
                choices.set_selected(index);
            }
        }
    });
    if let Some(path) = files.first() {
        model.set_text(&path.to_string_lossy());
    }
    if files.is_empty() {
        model_body.append(&caption(
            "No image model is downloaded yet. Open Models in the sidebar to get one.",
        ));
    } else {
        model_body.append(&installed_models);
    }
    let model_file = gtk::Expander::new(Some("Model file"));
    model_file.set_child(Some(&model));
    model_body.append(&model_file);
    panel.append(&model_section);

    // ----- Image -----
    let (image_section, image_body) = style::collapsible("Image", "image-x-generic-symbolic", true);
    let dimensions = gtk::DropDown::from_strings(&[
        "512 × 512 · square",
        "768 × 768 · square",
        "1024 × 1024 · square",
        "512 × 768 · portrait",
        "768 × 512 · landscape",
        "768 × 1024 · portrait",
        "1024 × 768 · landscape",
    ]);
    dimensions.update_property(&[gtk::accessible::Property::Label(
        "Image dimensions for recipe",
    )]);
    image_body.append(&style::field("Size", &dimensions));
    let batch = gtk::SpinButton::with_range(1.0, 8.0, 1.0);
    batch.set_numeric(true);
    batch.update_property(&[gtk::accessible::Property::Label(
        "Number of images in recipe",
    )]);
    let batch_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    batch_row.append(
        &gtk::Label::builder()
            .label("Images in this run")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["is-field-label"])
            .build(),
    );
    batch_row.append(&batch);
    image_body.append(&batch_row);
    panel.append(&image_section);

    // ----- Sampling -----
    let (sampling_section, sampling_body) =
        style::collapsible("Sampling", "view-dual-symbolic", true);
    let sampler = gtk::DropDown::from_strings(&SAMPLERS);
    sampler.update_property(&[gtk::accessible::Property::Label("Sampler")]);
    let scheduler = gtk::DropDown::from_strings(&["Discrete", "Karras"]);
    scheduler.update_property(&[gtk::accessible::Property::Label("Noise scheduler")]);
    let sampler_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    sampler_row.set_homogeneous(true);
    sampler_row.append(&style::field("Sampler", &sampler));
    sampler_row.append(&style::field("Schedule", &scheduler));
    sampling_body.append(&sampler_row);
    let steps = gtk::SpinButton::with_range(1.0, 50.0, 1.0);
    steps.set_value(20.0);
    steps.set_tooltip_text(Some("Denoising steps"));
    steps.update_property(&[gtk::accessible::Property::Label("Denoising steps")]);
    sampling_body.append(&style::slider_row("Steps", &steps));
    let cfg = gtk::SpinButton::with_range(0.0, 15.0, 0.5);
    cfg.set_digits(1);
    cfg.set_value(7.0);
    cfg.set_tooltip_text(Some("Prompt guidance strength (CFG)"));
    cfg.update_property(&[gtk::accessible::Property::Label("Prompt guidance CFG")]);
    sampling_body.append(&style::slider_row("Prompt guidance", &cfg));
    panel.append(&sampling_section);

    // ----- Seed -----
    let (seed_section, seed_body) =
        style::collapsible("Seed", "media-playlist-shuffle-symbolic", true);
    let seed = gtk::SpinButton::with_range(-1.0, 2_147_483_647.0, 1.0);
    seed.set_numeric(true);
    seed.set_value(-1.0);
    seed.set_hexpand(true);
    seed.update_property(&[gtk::accessible::Property::Label(
        "Seed, minus one means random",
    )]);
    let random_seed = gtk::Button::from_icon_name("media-playlist-shuffle-symbolic");
    random_seed.set_tooltip_text(Some("Use a random seed for every run (−1)"));
    random_seed.update_property(&[gtk::accessible::Property::Label("Random seed")]);
    let seed_spin = seed.clone();
    random_seed.connect_clicked(move |_| seed_spin.set_value(-1.0));
    let seed_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    seed_row.append(&seed);
    seed_row.append(&random_seed);
    seed_body.append(&seed_row);
    seed_body.append(&caption(
        "−1 picks a new seed each run. Keep one fixed to compare changes fairly.",
    ));
    panel.append(&seed_section);

    // ----- Reference image -----
    let (reference_section, reference_body) =
        style::collapsible("Reference image", "insert-image-symbolic", false);
    let reference_image = gtk::Entry::new();
    reference_image.set_placeholder_text(Some("No reference · prompt only"));
    reference_image.update_property(&[gtk::accessible::Property::Label("Reference image path")]);
    let thumb = gtk::Picture::new();
    thumb.set_content_fit(gtk::ContentFit::Cover);
    thumb.set_size_request(64, 64);
    thumb.set_can_shrink(true);
    let thumb_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    thumb_frame.add_css_class("is-ref-thumb");
    thumb_frame.set_overflow(gtk::Overflow::Hidden);
    thumb_frame.append(&thumb);
    let choose_reference = icon_button("document-open-symbolic", "Choose…", &["is-pill"]);
    let clear_reference = icon_button("edit-clear-symbolic", "Clear", &["flat", "is-pill"]);
    let reference_buttons = gtk::Box::new(gtk::Orientation::Vertical, 6);
    reference_buttons.set_valign(gtk::Align::Center);
    reference_buttons.append(&choose_reference);
    reference_buttons.append(&clear_reference);
    let reference_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    reference_row.append(&thumb_frame);
    reference_row.append(&reference_buttons);
    let reference = reference_image.clone();
    choose_reference.connect_clicked(move |button| {
        let parent = button.root().and_downcast::<gtk::Window>();
        let chooser = gtk::FileDialog::builder()
            .title("Choose reference image")
            .accept_label("Use reference")
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("PNG and JPEG images"));
        filter.add_mime_type("image/png");
        filter.add_mime_type("image/jpeg");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        chooser.set_filters(Some(&filters));
        let reference = reference.clone();
        chooser.open(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    reference.set_text(&path.to_string_lossy());
                }
            }
        });
    });
    let reference = reference_image.clone();
    clear_reference.connect_clicked(move |_| reference.set_text(""));
    let thumb_for_entry = thumb.clone();
    reference_image.connect_changed(move |entry| {
        let path = entry.text();
        if path.is_empty() || !Path::new(path.as_str()).is_file() {
            thumb_for_entry.set_paintable(None::<&gdk::Paintable>);
        } else {
            thumb_for_entry.set_filename(Some(path.as_str()));
        }
    });
    reference_body.append(&reference_row);
    let reference_path = gtk::Expander::new(Some("Reference file"));
    reference_path.set_child(Some(&reference_image));
    reference_body.append(&reference_path);
    let reference_strength = gtk::SpinButton::with_range(0.0, 2.0, 0.05);
    reference_strength.set_digits(2);
    reference_strength.set_value(0.8);
    reference_strength
        .update_property(&[gtk::accessible::Property::Label("Reference image strength")]);
    reference_body.append(&style::slider_row(
        "Reference strength",
        &reference_strength,
    ));
    panel.append(&reference_section);

    // ----- LoRA -----
    let (lora_section, lora_body) = style::collapsible("LoRA", "emblem-system-symbolic", false);
    let lora = gtk::Entry::new();
    lora.set_placeholder_text(Some("LoRA name (optional)"));
    lora.update_property(&[gtk::accessible::Property::Label("LoRA name for recipe")]);
    lora_body.append(&lora);
    let lora_weight = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.5, 0.05);
    lora_weight.set_draw_value(true);
    lora_weight.set_value_pos(gtk::PositionType::Right);
    lora_weight.set_digits(2);
    lora_weight.set_value(1.0);
    lora_weight.set_hexpand(true);
    lora_weight.update_property(&[gtk::accessible::Property::Label("LoRA strength")]);
    lora_body.append(&style::field("LoRA strength", &lora_weight));
    panel.append(&lora_section);

    // ----- Advanced -----
    let (advanced_section, advanced_body) =
        style::collapsible("Advanced", "preferences-other-symbolic", false);
    let vae = gtk::Entry::new();
    vae.set_placeholder_text(Some("Built into checkpoint, or VAE file path"));
    vae.update_property(&[gtk::accessible::Property::Label("VAE decoder file")]);
    let installed_vae = gtk::glib::user_data_dir()
        .join("mseries-image-studio/models/vae-ft-mse-840000-ema-pruned.safetensors");
    if installed_vae.is_file() {
        vae.set_text(&installed_vae.to_string_lossy());
    }
    advanced_body.append(&style::field("VAE decoder", &vae));
    let vae_on_cpu = gtk::CheckButton::with_label("Decode VAE on CPU (lower GPU use)");
    vae_on_cpu.set_active(true);
    vae_on_cpu.set_tooltip_text(Some("CPU decoding uses less GPU memory. Disable to try faster GPU decoding when memory allows. Saved with each recipe."));
    advanced_body.append(&vae_on_cpu);
    let vae_tile_size = gtk::SpinButton::with_range(64.0, 512.0, 64.0);
    vae_tile_size.set_value(256.0);
    let cpu_threads = gtk::SpinButton::with_range(1.0, 16.0, 1.0);
    cpu_threads.set_value(4.0);
    let preview_interval = gtk::SpinButton::with_range(1.0, 10.0, 1.0);
    preview_interval.set_value(3.0);
    for (title, control) in [
        ("Decoder tile size", &vae_tile_size),
        ("CPU threads", &cpu_threads),
        ("Preview every N steps", &preview_interval),
    ] {
        control.update_property(&[gtk::accessible::Property::Label(title)]);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["is-field-label"])
                .build(),
        );
        row.append(control);
        advanced_body.append(&row);
    }
    let params_backend =
        gtk::DropDown::from_strings(&["Disk · lower memory", "CPU memory", "Automatic"]);
    params_backend.update_property(&[gtk::accessible::Property::Label(
        "Model weights storage backend",
    )]);
    advanced_body.append(&style::field("Model weights storage", &params_backend));
    let max_vram_gib = gtk::SpinButton::with_range(2.0, 8.0, 0.5);
    max_vram_gib.set_digits(1);
    max_vram_gib.set_value(3.0);
    max_vram_gib.update_property(&[gtk::accessible::Property::Label("GPU memory budget in GiB")]);
    let vram_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    vram_row.append(
        &gtk::Label::builder()
            .label("GPU memory budget · GiB")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["is-field-label"])
            .build(),
    );
    vram_row.append(&max_vram_gib);
    advanced_body.append(&vram_row);
    let tags = gtk::Entry::new();
    tags.set_placeholder_text(Some("portrait, streamer, study"));
    tags.update_property(&[gtk::accessible::Property::Label(
        "Comma-separated image tags",
    )]);
    advanced_body.append(&style::field("Tags", &tags));
    panel.append(&advanced_section);
    if model.text().contains("Realistic") {
        dimensions.set_selected(3);
        steps.set_value(30.0);
        cfg.set_value(5.0);
        sampler.set_selected(2);
        scheduler.set_selected(1);
    }

    let generate = gtk::Button::with_label("Generate");
    generate.set_sensitive(false);
    generate.add_css_class("suggested-action");
    generate.add_css_class("is-generate");
    generate.set_tooltip_text(Some(
        "Generation is disabled until an image engine/model pair is calibrated on this device.",
    ));
    generate.update_property(&[gtk::accessible::Property::Label("Generate image")]);

    // Sticky footer: save + status stay visible while the settings scroll.
    let save = gtk::Button::with_label("Save recipe");
    save.add_css_class("is-pill");
    save.set_tooltip_text(Some(
        "Save the prompt and every setting so it can be reopened from the Library",
    ));
    let recipe_status = gtk::Label::new(None);
    recipe_status.set_xalign(0.0);
    recipe_status.set_wrap(true);
    recipe_status.add_css_class("is-caption");
    recipe_status.set_visible(false);
    recipe_status.connect_label_notify(|label| label.set_visible(!label.label().is_empty()));
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    footer.set_margin_top(10);
    footer.set_margin_bottom(12);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.append(&recipe_status);
    footer.append(&save);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&panel)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("is-inspector");
    root.set_width_request(340);
    root.append(&scroller);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&footer);
    Inspector {
        panel: root.upcast(),
        engine,
        engine_card,
        engine_dot,
        model,
        lora,
        lora_weight,
        dimensions,
        batch,
        seed,
        steps,
        cfg,
        sampler,
        vae,
        vae_on_cpu,
        reference_image,
        reference_strength,
        vae_tile_size,
        cpu_threads,
        preview_interval,
        params_backend,
        max_vram_gib,
        scheduler,
        tags,
        generate,
        save,
        recipe_status,
    }
}

fn apply_workflow(workflow: &str, page: &CreatePage, settings: &Inspector) {
    *page.workflow.borrow_mut() = workflow.into();
    settings.steps.set_value(if workflow == "Model readiness" {
        8.0
    } else {
        20.0
    });
    settings.cfg.set_value(7.0);
    settings.sampler.set_selected(0);
    match workflow {
        "Model readiness" => {
            set_text(
                &page.prompt,
                "Editorial portrait of an original fictional adult livestream creator in their mid-20s, relaxed at a tidy desk with a webcam and headphones, friendly confident expression, soft daylight, natural color, realistic photography, shoulders-up composition.",
            );
            set_text(
                &page.negative,
                "text, watermark, logo, extra fingers, distorted hands, blurry face",
            );
            settings.dimensions.set_selected(0);
            settings.batch.set_value(1.0);
            settings.seed.set_value(424242.0);
            page.workflow_status.set_text("Model readiness · one conservative 512 × 512 image, batch 1, fixed seed. Save the recipe while the image runtime is being qualified.");
            page.assistant_status.set_text("Start with one model and one image. The app will not send a job until the runtime is connected and this size has a measured memory budget.");
        }
        "LoRA comparison" => {
            set_text(
                &page.prompt,
                "Editorial scene of an original fictional adult college student hosting a relaxed gaming livestream from a small modern study nook, headphones, webcam, natural window light, candid documentary photography, consistent identity.",
            );
            set_text(
                &page.negative,
                "text, watermark, logo, extra fingers, distorted hands, sexualized styling",
            );
            settings.dimensions.set_selected(0);
            settings.batch.set_value(1.0);
            settings.seed.set_value(424242.0);
            page.workflow_status.set_text("LoRA comparison · save the same prompt and seed with LoRA strength 0, then save again with the chosen weight after a compatible LoRA is available.");
            page.assistant_status.set_text("Keep the base model, prompt, dimensions and seed fixed; change only LoRA strength between recipe copies. The LoRA name and weight are saved as notes.");
        }
        _ => {
            set_text(&page.prompt, SAMPLE_SERIES);
            set_text(
                &page.negative,
                "text, watermark, logo, extra fingers, inconsistent person, sexualized styling",
            );
            settings.dimensions.set_selected(0);
            settings.batch.set_value(1.0);
            settings.seed.set_value(424242.0);
            page.intent.set_selected(1);
            page.workflow_status.set_text("Prompt type study · ask the local assistant to recommend a type, review prompt wording, and save variants with the same subject and seed.");
            page.assistant_status.set_text("This recipe describes a three-scene series. A connected image engine will need a qualified multi-image workflow before it can render the set.");
        }
    }
}

fn wire_actions(
    application: &gtk::Application,
    window: &gtk::ApplicationWindow,
    stack: &gtk::Stack,
    create: &CreatePage,
    library: &LibraryPage,
    inspector: &Inspector,
    store: &RecipeStore,
) {
    create.dock_actions.append(&inspector.generate);
    wire_library(library, create, inspector, stack);
    let generation_page = create.clone_refs();
    let generation_settings = inspector.clone_refs();
    inspector.generate.connect_clicked(move |_| {
        let base = gtk::glib::user_data_dir().join("mseries-image-studio");
        if base.join("batch.lock").exists() || base.join("generation.lock").exists() {
            generation_settings.recipe_status.set_text(
                "A local workflow is active. Stop it or wait before starting another run.",
            );
            return;
        }
        let recipe = current_recipe(&generation_page, &generation_settings);
        let base = gtk::glib::user_data_dir().join("mseries-image-studio");
        let _ = std::fs::remove_file(base.join("cancel.request"));
        generation_settings
            .recipe_status
            .set_text("Submitting supervised local calibration…");
        std::thread::spawn(move || {
            let mut command = std::process::Command::new("python3");
            command.arg("-c");
            if recipe.batch_size > 1 {
                command.arg(include_str!("sweep_runner.py"))
                    .arg(&base).arg(serde_json::to_string(&recipe).unwrap())
                    .arg(serde_json::json!({"kind":"repeat","count":recipe.batch_size,"review":false}).to_string())
                    .arg(crate::local_generation_worker()).arg(include_str!("review_image.py"));
            } else {
                command.arg(crate::local_generation_worker())
                    .arg(&base).arg(serde_json::to_string(&recipe).unwrap());
            }
            let result = command.output();
            if let Err(error) = result {
                eprintln!("Generation worker could not start: {error}");
            }
        });
    });
    let page = create.clone_refs();
    let settings = inspector.clone_refs();
    let library_list = library.recipes.clone();
    let store = store.clone();
    let stack_for_save = stack.clone();
    inspector.save.connect_clicked(move |_| {
        let recipe = current_recipe(&page, &settings);
        match store.save(&recipe) {
            Ok(_) => {
                settings.recipe_status.set_text("Recipe saved to Library.");
                add_recipe_row(
                    &library_list,
                    recipe,
                    &page,
                    &settings,
                    &stack_for_save,
                    true,
                );
            }
            Err(error) => settings
                .recipe_status
                .set_text(&format!("Recipe could not be saved: {error}")),
        }
    });

    let suggest_type = create.suggest_type.clone();
    let help_prompt = create.help_prompt.clone();
    let page = create.clone_refs();
    let status = create.assistant_status.clone();
    let intent = create.intent.clone();
    let type_dialog_parent = window.clone();
    suggest_type.connect_clicked(move |_| {
        let prompt = text_of(&page.prompt).trim().to_string();
        if prompt.is_empty() {
            status.set_text("Add a subject or idea first, then ask for a prompt-type suggestion.");
            return;
        }
        status.set_text(
            "Asking the already-running M-Series local assistant to recommend a prompt type…",
        );
        let status_done = status.clone();
        let intent = intent.clone();
        let window = type_dialog_parent.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(assistant::suggest_intent(&prompt));
        });
        glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
            Ok(Ok(suggestion)) => {
                status_done.set_text(
                    "Prompt type suggestion ready. Review it before changing the selection.",
                );
                show_intent_dialog(&window, &intent, &suggestion);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                status_done.set_text(&format!("Prompt-type help unavailable: {error}"));
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                status_done.set_text("Prompt-type helper stopped unexpectedly.");
                glib::ControlFlow::Break
            }
        });
    });

    let page = create.clone_refs();
    let status = create.assistant_status.clone();
    let intent = create.intent.clone();
    let prompt_dialog_parent = window.clone();
    help_prompt.connect_clicked(move |_| {
        let prompt = text_of(&page.prompt).trim().to_string();
        if prompt.is_empty() {
            status.set_text("Add a subject or idea first, then ask for prompt help.");
            return;
        }
        let index = intent.selected() as usize;
        let intent_name = INTENTS
            .get(index)
            .copied()
            .unwrap_or("Portrait")
            .to_string();
        status.set_text("Asking the already-running M-Series local assistant…");
        let status_done = status.clone();
        let prompt_view = page.prompt.clone();
        let window = prompt_dialog_parent.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(assistant::suggest_prompt(&intent_name, &prompt));
        });
        glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
            Ok(Ok(suggestion)) => {
                status_done.set_text("Suggestion ready. Review it before replacing your prompt.");
                show_prompt_dialog(&window, &prompt_view, &suggestion);
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                status_done.set_text(&format!("Prompt help unavailable: {error}"));
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                status_done.set_text("Prompt helper stopped unexpectedly.");
                glib::ControlFlow::Break
            }
        });
    });
    let _ = stack;
    let _ = application;
}

/// One filmstrip thumbnail per recent image; clicking shows it on the canvas.
fn refresh_filmstrip(create: &CreatePage, items: &[Recipe]) {
    style::clear_box(&create.film);
    let recent: Vec<&Recipe> = items.iter().take(12).collect();
    create.film_rail.set_visible(!recent.is_empty());
    let buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    for recipe in recent {
        let button = gtk::Button::new();
        button.add_css_class("is-film");
        button.set_overflow(gtk::Overflow::Hidden);
        let picture = gtk::Picture::for_filename(&recipe.image_path);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_size_request(48, 48);
        picture.set_can_shrink(true);
        button.set_child(Some(&picture));
        button.set_tooltip_text(Some(&format!(
            "{} · {}×{} · {}",
            recipe.intent,
            recipe.width,
            recipe.height,
            when(recipe.created_at_unix)
        )));
        button.update_property(&[gtk::accessible::Property::Label("Show earlier image")]);
        let stage = create.stage.clone();
        let path = recipe.image_path.clone();
        let details = format!(
            "{} · {} · {} steps · seed {}",
            short_model_name(&recipe.model_id),
            recipe.sampler,
            recipe.steps,
            recipe.seed
        );
        let status = format!("{} · {}", recipe.intent, when(recipe.created_at_unix));
        let siblings = buttons.clone();
        let this = button.clone();
        button.connect_clicked(move |_| {
            if stage.show(&path) {
                stage.status.set_text(&status);
                stage.set_details(&details);
                for other in siblings.borrow().iter() {
                    other.remove_css_class("is-current");
                }
                this.add_css_class("is-current");
            }
        });
        buttons.borrow_mut().push(button.clone());
        create.film.append(&button);
    }
}

fn gallery_tile(recipe: &Recipe) -> gtk::FlowBoxChild {
    // A fixed-size spacer defines the tile's natural size; the picture is an
    // overlay so a large image can never inflate the grid.
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_size_request(168, 168);
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("is-tile");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_child(Some(&spacer));
    let picture = gtk::Picture::for_filename(&recipe.image_path);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_alternative_text(Some(&recipe.prompt));
    overlay.add_overlay(&picture);
    let caption_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
    caption_box.add_css_class("is-tile-caption");
    caption_box.set_valign(gtk::Align::End);
    let intent = gtk::Label::builder()
        .label(&recipe.intent)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["is-card-title"])
        .build();
    let sub = gtk::Label::builder()
        .label(format!(
            "{}×{} · {}",
            recipe.width,
            recipe.height,
            when(recipe.created_at_unix)
        ))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["is-caption"])
        .build();
    caption_box.append(&intent);
    caption_box.append(&sub);
    overlay.add_overlay(&caption_box);
    let square = gtk::AspectFrame::new(0.5, 0.5, 1.0, false);
    square.set_child(Some(&overlay));
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&square));
    child
}

fn wire_library(
    library: &LibraryPage,
    create: &CreatePage,
    inspector: &Inspector,
    stack: &gtk::Stack,
) {
    let items = library.items.clone();
    let detail = library.detail.clone();
    let create = create.clone_refs();
    let inspector = inspector.clone_refs();
    let stack = stack.clone();
    library.flow.connect_selected_children_changed(move |flow| {
        let Some(child) = flow.selected_children().into_iter().next() else {
            detail.root.set_visible(false);
            return;
        };
        let Some(recipe) = items.borrow().get(child.index() as usize).cloned() else {
            return;
        };
        show_library_detail(&detail, &recipe, &create, &inspector, &stack);
    });
}

fn show_library_detail(
    detail: &LibraryDetail,
    recipe: &Recipe,
    create: &CreatePage,
    inspector: &Inspector,
    stack: &gtk::Stack,
) {
    style::clear_box(&detail.content);
    detail.root.set_visible(true);
    let content = &detail.content;

    let picture = gtk::Picture::for_filename(&recipe.image_path);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_can_shrink(true);
    picture.add_css_class("is-detail-image");
    picture.set_overflow(gtk::Overflow::Hidden);
    let frame = gtk::AspectFrame::new(
        0.5,
        0.5,
        recipe.width.max(1) as f32 / recipe.height.max(1) as f32,
        false,
    );
    frame.set_child(Some(&picture));
    frame.set_size_request(-1, 280);
    content.append(&frame);

    let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
    header.append(
        &gtk::Label::builder()
            .label(format!(
                "{} · {}×{}",
                recipe.intent, recipe.width, recipe.height
            ))
            .xalign(0.0)
            .css_classes(["is-card-title"])
            .build(),
    );
    header.append(&caption(&when(recipe.created_at_unix)));
    content.append(&header);

    let prompt = gtk::Label::builder()
        .label(&recipe.prompt)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    content.append(&prompt);

    let mut chips: Vec<(String, &str, Option<&str>)> = vec![
        (
            short_model_name(&recipe.model_id),
            "is-accent",
            Some("drive-harddisk-symbolic"),
        ),
        (format!("{} steps", recipe.steps), "", None),
        (format!("CFG {:.1}", recipe.cfg_scale), "", None),
        (format!("Seed {}", recipe.seed), "", None),
    ];
    if !recipe.lora_id.is_empty() {
        chips.push((
            format!("LoRA {} · {:.2}", recipe.lora_id, recipe.lora_weight),
            "",
            None,
        ));
    }
    for tag in &recipe.tags {
        chips.push((tag.clone(), "", Some("tag-new-symbolic")));
    }
    content.append(&style::chip_rows(&chips, 36));

    // Actions
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let hosted = recipe.engine_id.contains("hosted");
    let repeat = icon_button(
        "document-revert-symbolic",
        "Use these settings",
        &["suggested-action", "is-pill"],
    );
    repeat.set_tooltip_text(Some(if hosted {
        "Exact repeat is unavailable: the hosted provider did not expose its model and inference settings. Loads editable starter settings instead."
    } else {
        "Load this recipe and its saved seed into Create. Press Generate when ready."
    }));
    let new_seed = icon_button(
        "media-playlist-shuffle-symbolic",
        "Try a new seed",
        &["is-pill"],
    );
    new_seed.set_tooltip_text(Some(
        "Load this recipe with a fresh random seed. Press Generate when ready.",
    ));
    for (button, fresh) in [(&repeat, false), (&new_seed, true)] {
        let mut saved = recipe.clone();
        if fresh {
            saved.seed = -1;
        }
        let create = create.clone_refs();
        let inspector = inspector.clone_refs();
        let stack = stack.clone();
        button.connect_clicked(move |_| {
            load_recipe(&saved, &create, &inspector);
            stack.set_visible_child_name("create");
        });
        actions.append(button);
    }
    let view = icon_button(
        "image-red-eye-symbolic",
        "Open full size",
        &["flat", "is-pill"],
    );
    let image_path = recipe.image_path.clone();
    view.connect_clicked(move |button| {
        let viewer = gtk::Window::builder()
            .title("Image Studio · Image")
            .default_width(1100)
            .default_height(760)
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            viewer.set_transient_for(Some(&parent));
        }
        let image = gtk::Picture::for_filename(&image_path);
        image.set_content_fit(gtk::ContentFit::Contain);
        image.set_can_shrink(true);
        viewer.set_child(Some(&image));
        viewer.present();
    });
    actions.append(&view);
    content.append(&actions);

    // Measurements
    let measure = style::padded_card(10);
    let measure_body = style::card_content(&measure);
    measure_body.set_spacing(8);
    let grid = gtk::Grid::builder()
        .column_spacing(14)
        .row_spacing(8)
        .column_homogeneous(true)
        .build();
    let cells = [
        (
            "Sampler",
            format!("{} · {}", recipe.sampler, recipe.scheduler),
        ),
        (
            "Time",
            recipe
                .elapsed_seconds
                .map(|v| format!("{v:.1} s"))
                .unwrap_or_else(|| "Not measured".into()),
        ),
        (
            "First preview",
            recipe
                .time_to_first_preview_seconds
                .map(|v| format!("{v:.1} s"))
                .unwrap_or_else(|| "Not measured".into()),
        ),
        (
            "Peak memory",
            recipe
                .peak_memory_mib
                .map(|v| format!("{v} MiB"))
                .unwrap_or_else(|| "Not measured".into()),
        ),
        ("Previews", recipe.preview_count.to_string()),
        ("Engine", recipe.engine_id.clone()),
    ];
    for (index, (key, value)) in cells.iter().enumerate() {
        grid.attach(
            &kv(key, value),
            (index % 2) as i32,
            (index / 2) as i32,
            1,
            1,
        );
    }
    measure_body.append(&grid);
    let note = if recipe.engine_id.contains("hosted") {
        "Hosted model and inference settings were not exposed; sampler and steps are editable local starter defaults."
    } else {
        "Peak memory is process RSS, excluding GPU memory."
    };
    measure_body.append(&caption(note));
    content.append(&measure);

    if !recipe.negative_prompt.is_empty() {
        let negative = style::padded_card(10);
        let body = style::card_content(&negative);
        body.set_spacing(4);
        body.append(&kv("Negative prompt", &recipe.negative_prompt));
        content.append(&negative);
    }
    let conditioning = format!(
        "Reference {} · strength {:.2} · VAE {} · tiles {} · {} CPU threads",
        if recipe.reference_image.is_empty() {
            "none".into()
        } else {
            Path::new(&recipe.reference_image)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        },
        recipe.reference_strength,
        if recipe.vae_on_cpu { "CPU" } else { "GPU" },
        recipe.vae_tile_size,
        recipe.cpu_threads
    );
    content.append(&caption(&conditioning));
    content.append(&caption(&format!(
        "Weights {} · GPU budget {:.1} GiB · {} previews every {} steps",
        recipe.params_backend, recipe.max_vram_gib, recipe.preview_method, recipe.preview_interval
    )));
    if !recipe.settings_note.is_empty() {
        content.append(&caption(&recipe.settings_note));
    }
    let notes = gtk::Expander::new(Some("My model notes and rating"));
    notes.set_child(Some(&crate::model_notes::model_notes_editor(
        gtk::glib::user_data_dir().join("mseries-image-studio"),
        recipe.model_id.clone(),
    )));
    content.append(&notes);
}

fn add_recipe_row(
    list: &gtk::ListBox,
    recipe: Recipe,
    page: &CreatePage,
    settings: &Inspector,
    stack: &gtk::Stack,
    newest: bool,
) {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    card.add_css_class("is-row-card");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(
        &gtk::Label::builder()
            .label(&recipe.workflow)
            .xalign(0.0)
            .css_classes(["is-card-title"])
            .build(),
    );
    head.append(&chip(&recipe.intent, "is-accent", None));
    head.append(&chip(
        &format!("{}×{}", recipe.width, recipe.height),
        "",
        None,
    ));
    text.append(&head);
    text.append(
        &gtk::Label::builder()
            .label(&recipe.prompt)
            .xalign(0.0)
            .wrap(true)
            .lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["is-dim"])
            .build(),
    );
    text.append(&caption(&when(recipe.created_at_unix)));
    let load = gtk::Button::with_label("Load");
    load.add_css_class("is-pill");
    load.set_valign(gtk::Align::Center);
    let page = page.clone_refs();
    let settings = settings.clone_refs();
    let stack = stack.clone();
    load.connect_clicked(move |_| {
        load_recipe(&recipe, &page, &settings);
        stack.set_visible_child_name("create");
    });
    card.append(&text);
    card.append(&load);
    row.set_child(Some(&card));
    if newest {
        list.prepend(&row);
    } else {
        list.append(&row);
    }
}

fn load_generated_images(
    library: &LibraryPage,
    create: &CreatePage,
    _inspector: &Inspector,
    _stack: &gtk::Stack,
) {
    let directory = gtk::glib::user_data_dir()
        .join("mseries-image-studio")
        .join("images");
    let outputs = load_outputs(&directory, 100).unwrap_or_else(|error| {
        eprintln!("Could not load image metadata: {error}");
        vec![]
    });
    library.detail.root.set_visible(false);
    while let Some(child) = library.flow.first_child() {
        library.flow.remove(&child);
    }
    for recipe in &outputs {
        library.flow.append(&gallery_tile(recipe));
    }
    let has_outputs = !outputs.is_empty();
    library
        .gallery
        .downcast_ref::<gtk::Stack>()
        .expect("gallery stack")
        .set_visible_child_name(if has_outputs { "grid" } else { "empty" });
    library.subtitle.set_text(&if has_outputs {
        format!(
            "{} image{} · each keeps its prompt, model and settings",
            outputs.len(),
            if outputs.len() == 1 { "" } else { "s" }
        )
    } else {
        "Images you generate appear here.".into()
    });
    refresh_filmstrip(create, &outputs);
    // Reopen on the latest image instead of an empty canvas.
    if !create.stage.frame.get_visible() {
        if let Some(latest) = outputs.first() {
            if create.stage.show(&latest.image_path) {
                if let Some(first) = create.film.first_child() {
                    first.add_css_class("is-current");
                }
                create.stage.status.set_text(&format!(
                    "{} · {}",
                    latest.intent,
                    when(latest.created_at_unix)
                ));
                create.stage.set_details(&format!(
                    "{} · {} · {} steps · seed {}",
                    short_model_name(&latest.model_id),
                    latest.sampler,
                    latest.steps,
                    latest.seed
                ));
            }
        }
    }
    *library.items.borrow_mut() = outputs;
    let _ = &library.placeholder;
}

fn refresh_hardware(inspector: &Inspector) {
    let host = HostCapabilities::capture();
    let base = gtk::glib::user_data_dir().join("mseries-image-studio");
    let connected = base.join("engine/sd-cli").is_file();
    let selected = inspector.model.text().to_string();
    let name = Path::new(&selected)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let required = match name {
        "stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf" => 9.0,
        "stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf" => 9.5,
        "sd_xl_turbo_1.0_fp16.safetensors" => 12.0,
        "Realistic_Vision_V6.0_NV_B1_fp16.safetensors" => 10.0,
        _ => f64::INFINITY,
    };
    let dimensions = match inspector.dimensions.selected() {
        1 => (768.0, 768.0),
        2 => (1024.0, 1024.0),
        3 => (512.0, 768.0),
        4 => (768.0, 512.0),
        5 => (768.0, 1024.0),
        6 => (1024.0, 768.0),
        _ => (512.0, 512.0),
    };
    let disk = inspector.params_backend.selected() == 0;
    let working = if disk {
        inspector.max_vram_gib.value().max(3.0)
    } else {
        required - 6.0
    };
    let reference = if inspector.reference_image.text().is_empty() {
        0.0
    } else if disk {
        0.5
    } else {
        1.3
    };
    let required = if required.is_finite() {
        6.0 + working
            + reference
            + (dimensions.0 * dimensions.1 / 262144.0 - 1.0_f64).max(0.0) * 0.5
    } else {
        required
    };
    let model = Path::new(&selected).is_file() && required.is_finite();
    let busy = base.join("generation.lock").exists() || base.join("batch.lock").exists();
    inspector.engine.set_text(&if !connected { "Install the local image engine in Models. Hosted samples have no intermediate frames.".into() }
        else if !model { "Local engine installed. Select a downloaded model in Models and load its starter settings.".into() }
        else { format!("Local engine installed. Selected model needs {required:.1} GiB available, including the desktop reserve. Runs are supervised calibration until qualified.") });
    let ready = connected && model && !busy && host.available_gib() >= required;
    style::set_tone(
        &inspector.engine_dot,
        if busy {
            "is-busy"
        } else if ready {
            "is-ok"
        } else {
            "is-warn"
        },
    );
    inspector.generate.set_sensitive(ready);
    inspector.generate.set_label(if busy {
        "Workflow active…"
    } else {
        "Generate"
    });
    inspector.generate.set_tooltip_text(Some(
        "Run the selected local recipe (Ctrl+Enter). Memory or configuration errors appear on the canvas.",
    ));
    let _ = &inspector.engine_card;
}

fn show_intent_dialog(parent: &gtk::ApplicationWindow, dropdown: &gtk::DropDown, suggestion: &str) {
    let dialog = gtk::Window::builder()
        .title("Review prompt type")
        .transient_for(parent)
        .modal(true)
        .default_width(440)
        .default_height(200)
        .build();
    let body = dialog_body();
    body.append(
        &gtk::Label::builder()
            .label("The local assistant recommends")
            .xalign(0.0)
            .css_classes(["is-caption"])
            .build(),
    );
    body.append(&chip(suggestion, "is-accent", None));
    body.append(
        &gtk::Label::builder()
            .label("Your prompt will stay unchanged. Choose whether to use this type.")
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let keep = gtk::Button::with_label("Keep current type");
    let accept = gtk::Button::with_label("Use suggested type");
    accept.add_css_class("suggested-action");
    let close = dialog.clone();
    keep.connect_clicked(move |_| close.close());
    let close = dialog.clone();
    let dropdown = dropdown.clone();
    let suggestion = suggestion.to_string();
    accept.connect_clicked(move |_| {
        if let Some(index) = INTENTS.iter().position(|intent| *intent == suggestion) {
            dropdown.set_selected(index as u32);
        }
        close.close();
    });
    actions.append(&keep);
    actions.append(&accept);
    body.append(&actions);
    dialog.set_child(Some(&body));
    dialog.present();
}

fn show_prompt_dialog(parent: &gtk::ApplicationWindow, prompt: &gtk::TextView, suggestion: &str) {
    let dialog = gtk::Window::builder()
        .title("Review prompt suggestion")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(380)
        .build();
    let body = dialog_body();
    let original = gtk::Label::new(Some(
        "Your original prompt stays unchanged until you accept this suggestion.",
    ));
    original.set_xalign(0.0);
    original.set_wrap(true);
    body.append(&original);
    let proposal = gtk::TextView::new();
    proposal.set_editable(false);
    proposal.set_cursor_visible(false);
    proposal.set_wrap_mode(gtk::WrapMode::WordChar);
    proposal.set_left_margin(10);
    proposal.set_right_margin(10);
    proposal.set_top_margin(10);
    proposal.set_bottom_margin(10);
    proposal.buffer().set_text(suggestion);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&proposal)
        .min_content_height(170)
        .vexpand(true)
        .build();
    scroller.add_css_class("is-field");
    body.append(&scroller);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let keep = gtk::Button::with_label("Keep original");
    let accept = gtk::Button::with_label("Use suggestion");
    accept.add_css_class("suggested-action");
    let close = dialog.clone();
    keep.connect_clicked(move |_| close.close());
    let close = dialog.clone();
    let prompt = prompt.clone();
    let suggestion = suggestion.to_string();
    accept.connect_clicked(move |_| {
        set_text(&prompt, &suggestion);
        close.close();
    });
    actions.append(&keep);
    actions.append(&accept);
    body.append(&actions);
    dialog.set_child(Some(&body));
    dialog.present();
}

fn dialog_body() -> gtk::Box {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.add_css_class("mseries-image-studio");
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body
}

fn set_text(view: &gtk::TextView, text: &str) {
    view.buffer().set_text(text);
}

fn text_of(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

fn test_hooks(
    application: &gtk::Application,
    window: &gtk::ApplicationWindow,
    stack: &gtk::Stack,
    create: &CreatePage,
    library: &LibraryPage,
    inspector: &Inspector,
    store: &RecipeStore,
) {
    if let Ok(workflow) = std::env::var("M_SERIES_IMAGE_STUDIO_TEST_WORKFLOW") {
        if ["Model readiness", "LoRA comparison", "Prompt type study"].contains(&workflow.as_str())
        {
            apply_workflow(&workflow, create, inspector);
        }
    }
    if let Ok(prompt) = std::env::var("M_SERIES_IMAGE_STUDIO_TEST_PROMPT") {
        set_text(&create.prompt, &prompt);
    }
    if let Ok(page) = std::env::var("M_SERIES_IMAGE_STUDIO_TEST_PAGE") {
        if stack.child_by_name(&page).is_some() {
            stack.set_visible_child_name(&page);
        }
    }
    if let (Ok(scheme), Some(settings)) = (
        std::env::var("M_SERIES_IMAGE_STUDIO_TEST_SCHEME"),
        gtk::Settings::default(),
    ) {
        // Review aid: Granite re-applies the session preference, so set both knobs
        // after startup finishes to see the app in the other scheme.
        let light = scheme == "light";
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            settings.set_gtk_interface_color_scheme(if light {
                gtk::InterfaceColorScheme::Light
            } else {
                gtk::InterfaceColorScheme::Dark
            });
            #[allow(deprecated)]
            settings.set_gtk_application_prefer_dark_theme(!light);
        });
    }
    if std::env::var("M_SERIES_IMAGE_STUDIO_TEST_SAVE_RECIPE")
        .ok()
        .as_deref()
        == Some("1")
    {
        let recipe = current_recipe(create, inspector);
        match store.save(&recipe) {
            Ok(_) => {
                inspector.recipe_status.set_text("Recipe saved to Library.");
                add_recipe_row(&library.recipes, recipe, create, inspector, stack, true);
            }
            Err(error) => inspector
                .recipe_status
                .set_text(&format!("Recipe could not be saved: {error}")),
        }
    }
    if let Ok(index) = std::env::var("M_SERIES_IMAGE_STUDIO_TEST_SELECT_IMAGE") {
        if let Some(child) = index
            .parse::<i32>()
            .ok()
            .and_then(|index| library.flow.child_at_index(index))
        {
            library.flow.select_child(&child);
        }
    }
    if let Ok(screenshot) = std::env::var("M_SERIES_IMAGE_STUDIO_SCREENSHOT") {
        let window = window.clone();
        glib::timeout_add_local_once(Duration::from_millis(2200), move || {
            save_window_png(&window, Path::new(&screenshot))
        });
    }
    if let Ok(seconds) = std::env::var("M_SERIES_IMAGE_STUDIO_EXIT_AFTER") {
        let seconds = seconds.parse::<u32>().unwrap_or(3).max(1);
        let application = application.clone();
        glib::timeout_add_seconds_local(seconds, move || {
            application.quit();
            glib::ControlFlow::Break
        });
    }
}

fn save_window_png(window: &gtk::ApplicationWindow, path: &Path) {
    // An unfocused review window would be drawn dimmed by the theme's backdrop
    // state; clear it so screenshots show the app as a user sees it while working.
    window.unset_state_flags(gtk::StateFlags::BACKDROP);
    let window = window.clone();
    let path = path.to_path_buf();
    glib::timeout_add_local_once(Duration::from_millis(250), move || {
        window.unset_state_flags(gtk::StateFlags::BACKDROP);
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(window.width()),
            f64::from(window.height()),
        );
        let Some(node) = snapshot.to_node() else {
            eprintln!("Screenshot skipped: window has nothing to draw yet");
            return;
        };
        let Some(renderer) = window.native().and_then(|native| native.renderer()) else {
            eprintln!("Screenshot skipped: window has no renderer");
            return;
        };
        if let Err(error) = renderer.render_texture(&node, None).save_to_png(&path) {
            eprintln!("Could not save screenshot: {error}");
        }
    });
}

fn build_model_guide(
    create: &CreatePage,
    inspector: &Inspector,
    stack: &gtk::Stack,
) -> gtk::Widget {
    let (scroller, page) = style::page_body();
    page.append(&style::page_header(
        "Model guide",
        "A short list for Apple Silicon running Linux. Download size is not inference memory. Author guidance and engine support are kept separate from local qualification.",
    ));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for (name, best_for, settings, qualification, tone) in [
        (
            "Realistic Vision 6.0 FP16",
            "Photographic portraits and fashion",
            "512 × 768 · DPM++ 2M · Karras · 30 steps · CFG 5 · external vae-ft-mse-840000-ema-pruned",
            "Installed; quality test pending",
            "is-accent",
        ),
        (
            "SD 1.5 Q4 / Q8",
            "Small portrait and fashion studies",
            "512² · Euler · 20 steps · CFG 7 · seed 424242 · batch 1",
            "Q4: supervised local calibration",
            "is-accent",
        ),
        (
            "SDXL Turbo FP16",
            "Fast composition experiments",
            "512² · 1–4 steps · engine CFG 1 (Diffusers guidance 0); no negative prompt",
            "Download candidate; local render not qualified",
            "is-warn",
        ),
        (
            "Qwen3-VL 2B Q4",
            "Review images against a rubric",
            "Model + matching Q8 vision projector · run after generation unloads",
            "CPU review adapter; download model + projector",
            "is-accent",
        ),
        (
            "LoRA adapters",
            "Style or subject conditioning",
            "Must match base family; start weight 0.7; keep seed fixed",
            "Not qualified in this app",
            "is-warn",
        ),
        (
            "IP-Adapter / inpainting",
            "Reference identity / masked edits",
            "Matching vision encoder or inpainting checkpoint required",
            "Engine capability; app controls pending",
            "is-warn",
        ),
    ] {
        let card = style::padded_card(12);
        let body = style::card_content(&card);
        body.set_spacing(6);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        head.append(
            &gtk::Label::builder()
                .label(name)
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["is-card-title"])
                .build(),
        );
        head.append(&chip(qualification, tone, None));
        body.append(&head);
        body.append(
            &gtk::Label::builder()
                .label(best_for)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
        body.append(&caption(settings));
        list.append(&card);
    }
    page.append(&list);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (label, steps, cfg, primary) in [
        ("Use portrait defaults · 20 steps", 20.0, 7.0, true),
        (
            "Use speed test · 8 steps · reduced quality",
            8.0,
            7.0,
            false,
        ),
    ] {
        let button = gtk::Button::with_label(label);
        button.add_css_class("is-pill");
        if primary {
            button.add_css_class("suggested-action");
        }
        let create = create.clone_refs();
        let inspector = inspector.clone_refs();
        let stack = stack.clone();
        button.connect_clicked(move |_|{inspector.steps.set_value(steps);inspector.cfg.set_value(cfg);inspector.seed.set_value(424242.0);inspector.batch.set_value(1.0);inspector.dimensions.set_selected(0);inspector.lora.set_text("");inspector.model.set_text(&gtk::glib::user_data_dir().join("mseries-image-studio/models/stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf").to_string_lossy());create.workflow.replace("SD 1.5 starter".into());inspector.recipe_status.set_text("Starter defaults loaded. Local calibration and memory admission still apply.");stack.set_visible_child_name("create");});
        actions.append(&button);
    }
    page.append(&actions);
    let review = style::padded_card(12);
    let review_body = style::card_content(&review);
    review_body.append(
        &gtk::Label::builder()
            .label("Compare fairly")
            .xalign(0.0)
            .css_classes(["is-card-title"])
            .build(),
    );
    review_body.append(&gtk::Label::builder().label("Keep seed, dimensions and steps fixed; change one prompt phrase or model at a time. A vision grade should store the rubric, evaluator identity, explanations and run settings. It is advisory, not a guarantee of likeness or commercial image quality.").xalign(0.0).wrap(true).build());
    page.append(&review);
    page.append(&style::section_caption("Sources"));
    let links = gtk::Box::new(gtk::Orientation::Vertical, 2);
    links.set_halign(gtk::Align::Start);
    for (label, url) in [
        (
            "SD 1.5 quantized model files and license",
            "https://huggingface.co/second-state/stable-diffusion-v1-5-GGUF",
        ),
        (
            "SDXL Turbo author settings and license",
            "https://huggingface.co/stabilityai/sdxl-turbo",
        ),
        (
            "Qwen3-VL model and projector instructions",
            "https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF",
        ),
        (
            "Vulkan engine support",
            "https://github.com/leejet/stable-diffusion.cpp",
        ),
    ] {
        let link = gtk::LinkButton::with_label(url, label);
        link.set_halign(gtk::Align::Start);
        links.append(&link);
    }
    page.append(&links);
    scroller.upcast()
}
