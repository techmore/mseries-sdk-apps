//! Drives the real Pantheon window through its save/load path under Xvfb.

use gtk::glib;
use gtk::prelude::*;
use mseries_image_studio::{Recipe, RecipeStore, recipe::save_png, ui::ImageStudioWindow};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("image-studio-render-{}", std::process::id()))
}

fn pump(duration: Duration) {
    let end = Instant::now() + duration;
    let context = glib::MainContext::default();
    while Instant::now() < end {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn walk(widget: &gtk::Widget, visit: &mut dyn FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(item) = child {
        walk(&item, visit);
        child = item.next_sibling();
    }
}

/// Find a button by its label, by a label inside its content, or by a tooltip prefix
/// (icon-only header buttons are named by their tooltips).
fn button(window: &gtk::ApplicationWindow, name: &str) -> gtk::Button {
    let mut found = None;
    walk(window.upcast_ref(), &mut |widget| {
        let Some(button) = widget.downcast_ref::<gtk::Button>() else {
            return;
        };
        let mut matches = button.label().is_some_and(|label| label == name)
            || button
                .tooltip_text()
                .is_some_and(|tooltip| tooltip.starts_with(name));
        if !matches {
            if let Some(child) = button.child() {
                walk(&child, &mut |inner| {
                    if let Some(label) = inner.downcast_ref::<gtk::Label>() {
                        matches |= label.text() == name;
                    }
                });
            }
        }
        if matches && found.is_none() || matches && button.is_mapped() {
            found = Some(button.clone());
        }
    });
    found.unwrap_or_else(|| panic!("missing button {name}"))
}

fn select_sidebar_page(window: &gtk::ApplicationWindow, page: &str) {
    let mut navigation = None;
    walk(window.upcast_ref(), &mut |widget| {
        if let Some(list) = widget.downcast_ref::<gtk::ListBox>()
            && list.has_css_class("navigation-sidebar")
        {
            navigation = Some(list.clone());
        }
    });
    let navigation = navigation.expect("Pantheon navigation sidebar");
    let mut row = None;
    let mut child = navigation.first_child();
    while let Some(item) = child {
        if item.widget_name() == page {
            row = item.downcast::<gtk::ListBoxRow>().ok();
            break;
        }
        child = item.next_sibling();
    }
    navigation.select_row(Some(&row.unwrap_or_else(|| panic!("navigation row {page}"))));
}

/// The stage picture that shows the live preview or the latest image.
fn stage_picture(window: &gtk::ApplicationWindow) -> Option<gtk::gdk::Paintable> {
    let mut found = None;
    walk(window.upcast_ref(), &mut |widget| {
        if let Some(picture) = widget.downcast_ref::<gtk::Picture>()
            && picture.has_css_class("is-stage-frame")
            && picture.is_mapped()
        {
            found = picture.paintable();
        }
    });
    found
}

fn mapped_text(window: &gtk::ApplicationWindow) -> Vec<String> {
    let mut values = vec![];
    walk(window.upcast_ref(), &mut |widget| {
        if !widget.is_mapped() {
            return;
        }
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            values.push(label.text().to_string());
        }
        if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
            let buffer = view.buffer();
            values.push(
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string(),
            );
        }
    });
    values
}

fn save_screenshot(window: &gtk::ApplicationWindow, name: &str) {
    let Some(directory) = std::env::var_os("MSERIES_IMAGE_STUDIO_SHOTS").map(PathBuf::from) else {
        return;
    };
    std::fs::create_dir_all(&directory).unwrap();
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let node = snapshot.to_node().expect("mapped app window paint node");
    let renderer = window
        .native()
        .and_then(|native| native.renderer())
        .expect("window renderer");
    renderer
        .render_texture(&node, None)
        .save_to_png(directory.join(name))
        .unwrap();
}

fn main() {
    let data_home = scratch().join("xdg");
    let _ = std::fs::remove_dir_all(scratch());
    std::fs::create_dir_all(&data_home).unwrap();
    // These process-local values are set before GTK/GLib read user paths.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", &data_home);
        std::env::set_var("M_SERIES_IMAGE_STUDIO_TEST_WORKFLOW", "Prompt type study");
        std::env::set_var("M_SERIES_IMAGE_STUDIO_TEST_SAVE_RECIPE", "1");
        std::env::remove_var("M_SERIES_IMAGE_STUDIO_TEST_PAGE");
    }
    let mut generated = Recipe::default();
    generated.prompt = "Test output image with recorded prompt metadata".into();
    generated.intent = "Streamer scene".into();
    generated.model_id = "fixture-model".into();
    generated.tags = vec!["fixture".into(), "metadata".into()];
    let images = data_home.join("mseries-image-studio").join("images");
    let sample = std::env::var_os("MSERIES_IMAGE_STUDIO_PREVIEW_SAMPLE")
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_else(|| include_bytes!("fixtures/tiny.png").to_vec());
    let output = save_png(&sample, &mut generated, &images).unwrap();
    std::fs::write(
        data_home.join("mseries-image-studio/current-job.json"),
        serde_json::to_vec(&serde_json::json!({
            "status": "Complete · sample preview", "engine": "Hosted sample · model unavailable",
            "prompt": generated.prompt, "running": false, "image_path": output,
            "recipe_path": output.with_extension("json")
        }))
        .unwrap(),
    )
    .unwrap();

    gtk::init().expect("GTK display; run under xvfb-run");
    granite::init();
    if let Some(display) = gtk::gdk::Display::default() {
        mseries_pantheon_kit::install_css(&display);
    }
    let application = gtk::Application::builder()
        .application_id("org.elementary.mseries.ImageStudio.RenderTest")
        .build();
    application
        .register(gtk::gio::Cancellable::NONE)
        .expect("register app");
    let ui = ImageStudioWindow::build(&application);
    let window = ui.window();
    let compact = std::env::var_os("M_SERIES_IMAGE_STUDIO_TEST_COMPACT").is_some();
    if compact {
        window.set_default_size(900, 700);
    }
    window.present();
    pump(Duration::from_millis(1400));
    // Generate is enabled only when an engine, model and enough memory exist on the host.
    assert!(button(window, "Generate").is_mapped());
    assert_eq!(window.has_css_class("pantheon-compact"), compact);
    let (minimum_width, ..) = window.measure(gtk::Orientation::Horizontal, -1);
    assert!(
        compact || minimum_width < mseries_pantheon_kit::text_unit(window, 76.0),
        "regular layout minimum width {minimum_width} must stay under the compact breakpoint"
    );
    if compact {
        assert!(!button(window, "Save recipe").is_mapped());
        let mut settings_menu = None;
        walk(window.upcast_ref(), &mut |widget| {
            if let Some(menu) = widget.downcast_ref::<gtk::MenuButton>() {
                if menu.tooltip_text().as_deref() == Some("Generation settings") {
                    settings_menu = Some(menu.clone());
                }
            }
        });
        let settings_menu = settings_menu.expect("compact generation settings popover");
        settings_menu.popup();
        pump(Duration::from_millis(500));
        assert!(button(window, "Save recipe").is_mapped());
        settings_menu.popdown();
        pump(Duration::from_millis(500));
    }
    assert!(mapped_text(window).iter().any(|text| text == "Create"));
    save_screenshot(
        window,
        if compact {
            "image-studio-create-compact.png"
        } else {
            "image-studio-create.png"
        },
    );

    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text.contains("Complete · sample preview")),
        "canvas status not shown; visible text: {:?}",
        mapped_text(window)
    );
    select_sidebar_page(window, "models");
    pump(Duration::from_millis(500));
    assert!(mapped_text(window).iter().any(|text| text == "Models"));
    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text == "Stable Diffusion 1.5 · Q4")
    );
    save_screenshot(window, "image-studio-models.png");
    select_sidebar_page(window, "guide");
    pump(Duration::from_millis(500));
    assert!(mapped_text(window).iter().any(|text| text == "Model guide"));
    save_screenshot(window, "image-studio-guide.png");
    select_sidebar_page(window, "compare");
    pump(Duration::from_millis(500));
    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text == "Compare runs")
    );
    assert!(button(window, "Run sweep").is_sensitive());
    select_sidebar_page(window, "review");
    pump(Duration::from_millis(500));
    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text == "Review an image")
    );
    assert!(button(window, "Review selected image").is_sensitive());
    assert!(button(window, "Stop").is_mapped());
    let recipes = RecipeStore::default_store().load_recent(10).unwrap();
    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].intent, "Streamer scene");
    assert_eq!(recipes[0].workflow, "Prompt type study");
    assert_eq!(
        (
            recipes[0].width,
            recipes[0].height,
            recipes[0].batch_size,
            recipes[0].seed
        ),
        (512, 512, 1, 424242)
    );
    assert!(recipes[0].prompt.contains("original fictional adult"));

    select_sidebar_page(window, "library");
    pump(Duration::from_millis(350));
    assert!(mapped_text(window).iter().any(|text| text == "Library"));
    // Choosing a tile opens its detail pane with the recorded prompt and settings.
    let mut gallery = None;
    walk(window.upcast_ref(), &mut |widget| {
        if let Some(flow) = widget.downcast_ref::<gtk::FlowBox>()
            && flow.has_css_class("is-gallery")
        {
            gallery = Some(flow.clone());
        }
    });
    let gallery = gallery.expect("library gallery");
    let tile = gallery.child_at_index(0).expect("one generated image tile");
    gallery.select_child(&tile);
    pump(Duration::from_millis(350));
    let library_text = mapped_text(window);
    assert!(
        library_text
            .iter()
            .any(|text| text.starts_with("1 image"))
    );
    assert!(
        library_text
            .iter()
            .any(|text| text.contains("Test output image with recorded prompt metadata"))
    );
    save_screenshot(window, "image-studio-library.png");
    button(window, "Use these settings").emit_clicked();
    pump(Duration::from_millis(500));
    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text == "Test output image with recorded prompt metadata")
    );
    select_sidebar_page(window, "library");
    let mut recipes_tab = None;
    walk(window.upcast_ref(), &mut |widget| {
        if let Some(toggle) = widget.downcast_ref::<gtk::ToggleButton>() {
            let mut named = toggle.label().is_some_and(|label| label == "Recipes");
            if let Some(child) = toggle.child() {
                walk(&child, &mut |inner| {
                    if let Some(label) = inner.downcast_ref::<gtk::Label>() {
                        named |= label.text() == "Recipes";
                    }
                });
            }
            if named {
                recipes_tab = Some(toggle.clone());
            }
        }
    });
    recipes_tab.expect("Recipes tab").emit_clicked();
    pump(Duration::from_millis(350));
    button(window, "Load").emit_clicked();
    pump(Duration::from_millis(350));
    let visible = mapped_text(window);
    assert!(visible.iter().any(|text| text == "Create"));
    assert!(
        visible
            .iter()
            .any(|text| text.contains("original fictional adult")
                && text.contains("Three-image editorial series"))
    );
    button(window, "Reload recipe").emit_clicked();
    pump(Duration::from_millis(500));
    assert!(
        mapped_text(window)
            .iter()
            .any(|text| text == "Test output image with recorded prompt metadata")
    );
    let preview_dir = data_home.join("mseries-image-studio/previews");
    std::fs::create_dir_all(&preview_dir).unwrap();
    let mut previous_frame = stage_picture(window);
    for frame in 1..=2 {
        let path = preview_dir.join(format!("fixture-frame-{frame}.png"));
        std::fs::write(&path, include_bytes!("fixtures/tiny.png")).unwrap();
        std::fs::write(data_home.join("mseries-image-studio/current-job.json"),serde_json::to_vec(&serde_json::json!({
            "status":format!("Rendering · fixture frame {frame}"),"engine":"Fixture transport (not AI inference)",
            "prompt":"Preview transport fixture","running":true,"image_path":path
        })).unwrap()).unwrap();
        pump(Duration::from_millis(1400));
        assert!(
            mapped_text(window)
                .iter()
                .any(|text| text == &format!("Rendering · fixture frame {frame}"))
        );
        let shown = stage_picture(window);
        assert!(
            shown.is_some(),
            "active preview did not display the new fixture frame"
        );
        // Every transport tick decodes the frame again, so the paintable must be a fresh one.
        assert!(
            shown != previous_frame,
            "preview frame {frame} did not replace the previous frame"
        );
        previous_frame = shown;
    }
}
