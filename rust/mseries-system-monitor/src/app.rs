use gtk::{gio, glib, pango, prelude::*};
use mseries_pantheon_kit::{
    LayoutClass, poll_system_metrics, set_accessible_label, set_box_spacing_text_units,
    set_margins_text_units, watch_responsive_width,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
use vte4::prelude::*;

pub fn build(application: &gtk::Application) {
    if let Some(window) = application.active_window() {
        window.present();
        return;
    }

    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("System Monitor")
        .default_width(980)
        .default_height(680)
        .build();

    let stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .build();
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    window.set_titlebar(Some(&header));

    let overview = gtk::Box::new(gtk::Orientation::Vertical, 16);
    overview.add_css_class("pantheon-page");
    set_box_spacing_text_units(&overview, 1.0);
    set_margins_text_units(&overview, 2.0, 2.0, 2.0, 2.0);
    let intro = gtk::Label::builder()
        .label("Live host usage with btop when you need process-level detail.")
        .xalign(0.0)
        .wrap(true)
        .build();
    intro.add_css_class("dim-label");
    overview.append(&intro);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let live = gtk::ToggleButton::with_label("Live updates");
    live.set_active(true);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh CPU and memory now"));
    set_accessible_label(&refresh, "Refresh CPU and memory");
    controls.append(&live);
    controls.append(&refresh);
    overview.append(&controls);

    let cards = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    cards.set_hexpand(true);
    set_box_spacing_text_units(&cards, 1.0);
    let (cpu_card, cpu_value, cpu_bar, cpu_detail) = metric_card("CPU");
    let (memory_card, memory_value, memory_bar, memory_detail) = metric_card("Memory");
    set_accessible_label(&cpu_bar, "CPU usage");
    set_accessible_label(&memory_bar, "Memory used");
    cards.append(&cpu_card);
    cards.append(&memory_card);
    overview.append(&cards);
    let cards_for_width = cards.clone();
    watch_responsive_width(&cards, 38.0, move |class| {
        cards_for_width.set_orientation(match class {
            LayoutClass::Compact => gtk::Orientation::Vertical,
            LayoutClass::Regular => gtk::Orientation::Horizontal,
        });
    });

    let updated = gtk::Label::builder()
        .label("Waiting for the first sample…")
        .xalign(0.0)
        .build();
    updated.add_css_class("dim-label");
    updated.add_css_class("caption");
    overview.append(&updated);
    stack.add_titled(&overview, Some("overview"), "Overview");

    let btop_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    btop_page.add_css_class("pantheon-page");
    set_box_spacing_text_units(&btop_page, 0.75);
    let btop_status = gtk::Label::builder()
        .label("btop starts when this page opens. Press q in the terminal to close it.")
        .xalign(0.0)
        .wrap(true)
        .build();
    btop_status.add_css_class("dim-label");
    let terminal = vte4::Terminal::new();
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);
    terminal.set_scrollback_lines(2000);
    terminal.set_scroll_on_output(false);
    terminal.set_scroll_on_keystroke(true);
    terminal.set_mouse_autohide(true);
    terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::Off);
    terminal.set_size_request(-1, 380);
    set_accessible_label(&terminal, "btop system monitor terminal");
    let terminal_scroll = gtk::ScrolledWindow::builder()
        .child(&terminal)
        .hexpand(true)
        .vexpand(true)
        .build();
    btop_page.append(&btop_status);
    btop_page.append(&terminal_scroll);
    stack.add_titled(&btop_page, Some("btop"), "btop");
    window.set_child(Some(&stack));

    let poller = poll_system_metrics(Duration::from_secs(2), move |result| match result {
        Ok(sample) => {
            if let Some(percent) = sample.cpu.percent {
                cpu_value.set_text(&format!("{percent:.0}%"));
                cpu_bar.set_fraction(percent / 100.0);
                cpu_detail.set_text("Average host CPU utilization since the last sample");
            } else {
                cpu_value.set_text("Sampling…");
                cpu_bar.set_fraction(0.0);
                cpu_detail.set_text("A second sample is needed to calculate CPU use");
            }
            let fraction = if sample.memory.total_bytes == 0 {
                0.0
            } else {
                sample.memory.used_bytes as f64 / sample.memory.total_bytes as f64
            };
            memory_value.set_text(&format!("{}%", (fraction * 100.0).round() as u8));
            memory_bar.set_fraction(fraction.clamp(0.0, 1.0));
            memory_detail.set_text(&format!(
                "{} used · {} available of {}",
                format_bytes(sample.memory.used_bytes),
                format_bytes(sample.memory.available_bytes),
                format_bytes(sample.memory.total_bytes)
            ));
            let time = glib::DateTime::now_local()
                .and_then(|date| date.format("%H:%M:%S"))
                .map(|text| text.to_string())
                .unwrap_or_else(|_| "now".to_owned());
            updated.set_text(&format!("Updated at {time}"));
        }
        Err(_) => {
            cpu_value.set_text("Unavailable");
            cpu_bar.set_fraction(0.0);
            cpu_detail.set_text("Linux CPU counters could not be read");
            memory_value.set_text("Unavailable");
            memory_bar.set_fraction(0.0);
            memory_detail.set_text("Linux memory counters could not be read");
            updated.set_text("Metrics could not be read; check the host procfs");
        }
    });
    poller.bind_to_widget_mapped(&overview);
    let poller_for_toggle = poller.clone();
    live.connect_toggled(move |button| poller_for_toggle.set_enabled(button.is_active()));
    poller.set_enabled(live.is_active());
    let poller_for_refresh = poller.clone();
    refresh.connect_clicked(move |_| poller_for_refresh.poll_now());
    // Keep the timer alive for the window. The handler also makes close an
    // explicit end to polling before GTK tears down the page.
    let poller_for_close = poller.clone();
    window.connect_close_request(move |_| {
        poller_for_close.set_enabled(false);
        glib::Propagation::Proceed
    });
    poller.poll_now();

    let started = Rc::new(Cell::new(false));
    let start_terminal = terminal.clone();
    let started_for_map = started.clone();
    let status_for_map = btop_status.clone();
    btop_page.connect_unmap({
        let terminal = terminal.clone();
        let started = started.clone();
        let status = btop_status.clone();
        move |_| {
            if started.get() {
                terminal.feed_child(b"q");
                status.set_text("Stopping btop while this page is hidden…");
            }
        }
    });
    let page_weak = btop_page.downgrade();
    btop_page.connect_map(move |_| {
        let terminal = start_terminal.clone();
        let started = started_for_map.clone();
        let status = status_for_map.clone();
        let page_weak = page_weak.clone();
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if page_weak.upgrade().is_some_and(|page| page.is_mapped()) {
                start_btop(&terminal, &started, &status);
            }
        });
    });
    let start_terminal = terminal.clone();
    let started_for_button = started.clone();
    let status_for_button = btop_status.clone();
    let restart = gtk::Button::with_label("Start or restart btop");
    restart.connect_clicked(move |_| {
        start_btop(&start_terminal, &started_for_button, &status_for_button);
    });
    btop_page.append(&restart);
    let terminal_started = started.clone();
    let exit_status = btop_status.clone();
    terminal.connect_child_exited(move |_, _| {
        terminal_started.set(false);
        exit_status.set_text("btop has exited. Select Start or restart btop to reopen it.");
    });
    let terminal_to_close = terminal.clone();
    window.connect_close_request(move |_| {
        terminal_to_close.feed_child(b"q");
        glib::Propagation::Proceed
    });

    window.present();
}

fn metric_card(title: &str) -> (gtk::Box, gtk::Label, gtk::ProgressBar, gtk::Label) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.set_hexpand(true);
    card.add_css_class("pantheon-surface");
    set_box_spacing_text_units(&card, 0.75);
    let heading = gtk::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("heading");
    let value = gtk::Label::builder()
        .label("—")
        .xalign(0.0)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    value.add_css_class("title-1");
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(false);
    let detail = gtk::Label::builder()
        .label("Waiting for data")
        .xalign(0.0)
        .wrap(true)
        .build();
    detail.add_css_class("dim-label");
    card.append(&heading);
    card.append(&value);
    card.append(&bar);
    card.append(&detail);
    (card, value, bar, detail)
}

fn start_btop(terminal: &vte4::Terminal, started: &Rc<Cell<bool>>, status: &gtk::Label) {
    if started.replace(true) {
        return;
    }
    status.set_text("Starting btop…");
    let terminal_for_result = terminal.clone();
    let started_for_result = started.clone();
    let status_for_result = status.clone();
    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        None,
        &["btop", "--update", "2000"],
        &[],
        glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        None::<&gio::Cancellable>,
        move |result| match result {
            Ok(pid) => {
                terminal_for_result.watch_child(pid);
                status_for_result.set_text("btop is running. Press q to close it.");
            }
            Err(_) => {
                started_for_result.set(false);
                status_for_result
                    .set_text("Could not start btop. Install the btop package, then retry.");
            }
        },
    );
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
