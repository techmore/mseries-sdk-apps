use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> gtk::glib::ExitCode {
    let application = gtk::Application::builder()
        .application_id("org.techmore.mseries.MessagesPreview")
        .build();
    application.connect_startup(|application| {
        granite::init();
        mseries_pantheon_kit::appearance::follow_color_scheme(None);
        mseries_pantheon_kit::chrome::install_app_actions(
            application,
            mseries_pantheon_kit::chrome::About {
                program_name: mseries_messages::strings::tr("Messages"),
                icon: "org.techmore.mseries.Messages".into(),
                comments: mseries_messages::strings::tr(
                    "Messages and Signal conversations from your iPhone, through BlueFerry.",
                ),
                menu_label: mseries_messages::strings::tr("About Messages"),
                website: None,
            },
        );
        if let Some(display) = gtk::gdk::Display::default() {
            mseries_pantheon_kit::install_css(&display);
        }
    });
    let current: Rc<RefCell<Option<Rc<mseries_messages::App>>>> = Rc::new(RefCell::new(None));
    let slot = current.clone();
    application.connect_activate(move |application| {
        if let Some(window) = application.active_window() {
            window.present();
            return;
        }
        *slot.borrow_mut() = mseries_messages::App::build(application);
    });
    application.run()
}
