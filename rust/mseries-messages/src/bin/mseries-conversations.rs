use gtk::gio;
use gtk::prelude::*;
use mseries_messages::strings::tr;

fn launch(desktop: &str, feedback: &gtk::Label) {
    let result = gio::AppInfo::all()
        .into_iter()
        .find(|app| app.id().as_deref() == Some(desktop))
        .ok_or_else(|| {
            tr("This app is not installed. Install its desktop integration, then try again.")
        })
        .and_then(|app| {
            let context = gtk::gdk::Display::default().map(|display| display.app_launch_context());
            app.launch(&[], context.as_ref())
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => feedback.set_text(&tr(
            "Opening the selected app. Reopen this switcher to choose another.",
        )),
        Err(error) => {
            feedback.set_text(&tr(
                "Could not open this app. Check its installation and try again.",
            ));
            feedback.set_tooltip_text(Some(&error));
        }
    }
}

fn main() -> gtk::glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.elementary.mseries.Conversations")
        .build();
    app.connect_startup(|_| {
        granite::init();
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        let stack = gtk::Stack::builder()
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let feedback = gtk::Label::builder().wrap(true).xalign(0.0).build();
        feedback.add_css_class("dim-label");
        let targets = [
            (
                "messages",
                tr("Messages"),
                "io.elementary.mseries.MessagesRust.desktop",
                tr("Your iPhone conversations"),
                tr("Show Messages"),
                tr("Switch to Messages"),
                "messaging-channel-messages",
            ),
            (
                "signal",
                tr("Signal"),
                "signal-desktop.desktop",
                tr("Your Signal conversations"),
                tr("Show Signal"),
                tr("Switch to Signal"),
                "messaging-channel-signal",
            ),
        ];
        for (name, title, desktop, description, action, accessible_name, class) in targets {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
            page.add_css_class(class);
            mseries_pantheon_kit::set_margins_text_units(&page, 1.0, 1.0, 1.0, 1.0);
            let heading = gtk::Label::builder()
                .label(description)
                .wrap(true)
                .xalign(0.0)
                .build();
            heading.add_css_class("heading");
            page.append(&heading);
            let button = gtk::Button::with_label(&action);
            mseries_pantheon_kit::set_accessible_label(&button, &accessible_name);
            let feedback = feedback.clone();
            button.connect_clicked(move |_| launch(desktop, &feedback));
            page.append(&button);
            stack.add_titled(&page, Some(name), &title);
        }
        let switcher = gtk::StackSwitcher::builder()
            .stack(&stack)
            .halign(gtk::Align::Center)
            .build();
        let header = gtk::HeaderBar::new();
        header.set_title_widget(Some(&switcher));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&stack);
        mseries_pantheon_kit::set_margins_text_units(&feedback, 0.5, 1.0, 0.5, 1.0);
        content.append(&feedback);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("Messages & Signal"))
            .default_width(470)
            .default_height(230)
            .child(&content)
            .build();
        window.add_css_class("mseries-messages");
        window.add_css_class("messaging-channel-messages");
        window.set_titlebar(Some(&header));
        let feedback_for_switch = feedback.clone();
        let weak_window = window.downgrade();
        stack.connect_visible_child_name_notify(move |stack| {
            if let Some(window) = weak_window.upgrade() {
                window.remove_css_class("messaging-channel-messages");
                window.remove_css_class("messaging-channel-signal");
                window.add_css_class(if stack.visible_child_name().as_deref() == Some("signal") {
                    "messaging-channel-signal"
                } else {
                    "messaging-channel-messages"
                });
            }
            match stack.visible_child_name().as_deref() {
                Some("signal") => launch("signal-desktop.desktop", &feedback_for_switch),
                Some("messages") => launch(
                    "io.elementary.mseries.MessagesRust.desktop",
                    &feedback_for_switch,
                ),
                _ => {}
            }
        });
        window.present();
    });
    app.run()
}
