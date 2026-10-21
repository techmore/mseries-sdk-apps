//! Opens the real Disk Space window on a fixture folder, drives the map like a mouse would, checks what is
//! shown, and optionally writes PNGs (light and dark, cards and circle). Needs a display:
//!
//!   dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory GSK_RENDERER=cairo \
//!     xvfb-run -a -s "-screen 0 1400x900x24" cargo test -p mseries-diskspace --test render
//!
//! Set MSERIES_DISKSPACE_SHOTS=<dir> to keep the screenshots. Plain `main`, not libtest: GTK must stay on
//! one thread.

use gtk::prelude::*;
use gtk::{gio, glib};
use mseries_diskspace::app::App;
use mseries_diskspace::model::Mode;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn pump(ms: u64) {
    let ctx = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        while ctx.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pump_until(what: &str, mut done: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < limit, "timed out waiting for {what}");
        pump(20);
    }
}

/// Incompressible bytes, so allocated blocks match the length even on compressing filesystems.
fn noise(len: usize, seed: &mut u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed >> 24) as u8
        })
        .collect()
}

/// Everything the test touches lives here, including a private XDG data dir so "Move to Trash" lands in a
/// Trash folder on the same filesystem as the fixture instead of the real one.
///
/// It sits under the target directory, not /tmp: GLib will not trash from a tmpfs ("system internal mount").
fn scratch() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("diskspace-render-{}", std::process::id()))
}

fn fixture() -> PathBuf {
    let root = scratch().join("data");
    let _ = std::fs::remove_dir_all(&root);
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut put = |rel: &str, kb: usize| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, noise(kb * 1024, &mut seed)).unwrap();
    };
    put("Videos/holiday-2025.mp4", 18_000);
    put("Videos/clip.mkv", 6_000);
    put("Backups/home-backup.tar.zst", 14_000);
    put("Backups/old-photos.zip", 3_000);
    put("VMs/ubuntu.qcow2", 12_000);
    put("Photos/2025/IMG_0001.jpg", 3_000);
    put("Photos/2025/IMG_0002.jpg", 2_000);
    put("Photos/2024/IMG_0003.jpg", 2_000);
    put("Documents/report.pdf", 1_000);
    put("Documents/notes.txt", 100);
    for i in 0..300 {
        put(&format!("cache/entry-{i:03}.tmp"), 8);
    }
    root
}

fn texts(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.text().to_string());
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && let Some(l) = button.label()
    {
        out.push(l.to_string());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        texts(&c, out);
        child = c.next_sibling();
    }
}

fn shown(app: &App) -> Vec<String> {
    let mut out = Vec::new();
    texts(app.window().upcast_ref(), &mut out);
    out
}

fn has(app: &App, needle: &str) -> bool {
    shown(app).iter().any(|t| t.contains(needle))
}

fn save_png(app: &App, path: &Path) {
    let window = app.window();
    let mut node = None;
    for _ in 0..100 {
        pump(50);
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
        node = snapshot.to_node();
        if node.is_some() {
            break;
        }
    }
    let node = node.expect("the window never painted");
    let renderer = window.native().and_then(|n| n.renderer()).expect("renderer");
    renderer.render_texture(&node, None).save_to_png(path).expect("write png");
}

fn main() {
    // SAFETY: nothing else is running yet, so no other thread reads the environment.
    unsafe { std::env::set_var("XDG_DATA_HOME", scratch().join("xdg")) };
    std::fs::create_dir_all(scratch().join("xdg")).unwrap(); // GLib compares devices with this folder
    if gtk::init().is_err() {
        eprintln!("FAIL: no display for GTK (run under xvfb-run)");
        std::process::exit(1);
    }
    granite::init();
    let application = gtk::Application::builder().application_id("org.techmore.mseries.DiskSpace.RenderTest").build();
    application.register(gio::Cancellable::NONE).expect("register");
    let shots = std::env::var_os("MSERIES_DISKSPACE_SHOTS").map(PathBuf::from);
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).unwrap();
    }

    let root = fixture();
    let app = App::build(&application, root.clone());
    pump_until("the scan", || !app.is_scanning());
    pump(200);

    for needle in ["Videos", "Backups", "VMs", "Photos", "scanned in", "Select items to review them"] {
        assert!(has(&app, needle), "expected '{needle}' in {:?}", shown(&app));
    }
    // The 300 tiny cache files sit under the largest-first folders, as the last list row of the root.
    let rows: Vec<String> = shown(&app);
    let videos_at = rows.iter().position(|t| t == "Videos").expect("Videos row");
    let vms_at = rows.iter().position(|t| t == "VMs").expect("VMs row");
    let cache_at = rows.iter().position(|t| t == "cache").expect("cache row");
    assert!(videos_at < vms_at && vms_at < cache_at, "list is largest first");

    for dark in [false, true] {
        // Granite's stylesheet follows this older setting; gtk-interface-color-scheme does not switch it.
        #[allow(deprecated)]
        gtk::Settings::default().expect("settings").set_gtk_application_prefer_dark_theme(dark);
        let tag = if dark { "dark" } else { "light" };

        app.show_mode(Mode::Cards);
        pump(150);
        if let Some(dir) = &shots {
            save_png(&app, &dir.join(format!("cards-{tag}.png")));
        }
        // The largest card is laid out first, in the top-left corner of the map.
        app.press_canvas(1, 40.0, 40.0);
        pump(50);
        assert!(has(&app, "Videos ·"), "clicking the top-left card selects Videos: {:?}", shown(&app));
        if let Some(dir) = &shots {
            save_png(&app, &dir.join(format!("cards-selected-{tag}.png")));
        }
        app.press_canvas(2, 40.0, 40.0);
        pump(100);
        assert!(has(&app, "holiday-2025.mp4") && !has(&app, "VMs"), "double-click enters Videos: {:?}", shown(&app));
        if let Some(dir) = &shots {
            save_png(&app, &dir.join(format!("cards-inside-{tag}.png")));
        }

        app.show_mode(Mode::Circle);
        pump(150);
        if let Some(dir) = &shots {
            save_png(&app, &dir.join(format!("circle-inside-{tag}.png")));
        }
        let (cx, cy) = (f64::from(app.canvas().width()) / 2.0, f64::from(app.canvas().height()) / 2.0);
        app.press_canvas(1, cx, cy);
        pump(100);
        assert!(has(&app, "VMs") && has(&app, "Backups"), "the centre disc goes back up: {:?}", shown(&app));
        if let Some(dir) = &shots {
            save_png(&app, &dir.join(format!("circle-{tag}.png")));
        }
        println!("ok: cards and circle rendered and driven ({tag})");
    }
    #[allow(deprecated)]
    gtk::Settings::default().expect("settings").set_gtk_application_prefer_dark_theme(false);

    // Giving space back: trash one folder, delete another for good, and watch the numbers follow.
    let before = shown(&app).iter().find(|t| t.contains("scanned in")).cloned().unwrap();
    assert!(app.select_by_name("Documents"));
    app.trash_selected();
    pump_until("the trash move", || !app.is_busy());
    pump(100);
    assert!(!root.join("Documents").exists(), "Documents left the folder: {:?}", shown(&app));
    assert!(scratch().join("xdg/Trash/files/Documents").exists(), "and went to the Trash");
    assert!(!shown(&app).iter().any(|t| t == "Documents"), "Documents left the list: {:?}", shown(&app));
    assert!(has(&app, "Moved to the Trash"), "{:?}", shown(&app));
    assert!(has(&app, "Open Trash"), "the toast points at the Trash");

    assert!(app.select_by_name("cache"));
    app.delete_confirmed();
    pump_until("the delete", || !app.is_busy());
    pump(100);
    assert!(!root.join("cache").exists(), "cache is gone from disk");
    assert!(!shown(&app).iter().any(|t| t == "cache"), "cache left the list");
    assert!(has(&app, "Deleted. Freed"), "{:?}", shown(&app));
    let after = shown(&app).iter().find(|t| t.contains("scanned in")).cloned().unwrap();
    assert_ne!(before, after, "the totals changed");
    assert!(after.starts_with("61."), "61 MB left of 67.5 after removing 1.1 + 4.9: {after}");
    println!("ok: trash and delete update the view ({after})");

    app.window().destroy();
    let _ = std::fs::remove_dir_all(scratch());
    println!("PASS: Disk Space render test");
}
