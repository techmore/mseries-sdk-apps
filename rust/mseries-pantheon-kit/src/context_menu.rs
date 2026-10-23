use gtk::prelude::*;

/// Attach an owned context popover with mouse, touch and keyboard access.
/// The caller supplies the popover's actions and an accessible anchor name.
pub fn attach_context_menu<W: IsA<gtk::Widget>>(anchor: &W, popover: &gtk::Popover) {
    let anchor = anchor.as_ref();
    anchor.set_focusable(true);
    anchor.update_property(&[
        gtk::accessible::Property::HasPopup(true),
        gtk::accessible::Property::Description("Press Menu or Shift+F10 for actions."),
    ]);
    popover.set_parent(anchor);
    let click = gtk::GestureClick::builder().button(3).build();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let menu = popover.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        menu.popup();
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    anchor.add_controller(click);
    let touch = gtk::GestureLongPress::new();
    touch.set_propagation_phase(gtk::PropagationPhase::Capture);
    let menu = popover.clone();
    touch.connect_pressed(move |gesture, x, y| {
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        menu.popup();
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    anchor.add_controller(touch);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let menu = popover.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let other_modifiers = gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::ALT_MASK
            | gtk::gdk::ModifierType::SUPER_MASK;
        if !modifiers.intersects(other_modifiers)
            && (key == gtk::gdk::Key::Menu
                || (key == gtk::gdk::Key::F10
                    && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)))
        {
            menu.set_pointing_to(None);
            menu.popup();
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    anchor.add_controller(keys);
    let menu = popover.clone();
    anchor.connect_destroy(move |_| menu.unparent());
}
