use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> gtk::glib::ExitCode {
    let application = gtk::Application::builder()
        .application_id("io.elementary.mseries.MessagesRust")
        .build();
    application.connect_startup(|_| {
        granite::init();
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
