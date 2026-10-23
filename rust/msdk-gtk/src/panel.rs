//! Attach GTK's existing Wayland surface to the compositor's Pantheon panel protocol.
use gtk::{glib::translate::ToGlibPtr, prelude::*};
unsafe extern "C" {
    fn msdk_panel_attach(
        window: *mut gtk::ffi::GtkWindow,
        anchor: u32,
        hide: u32,
        size: i32,
    ) -> i32;
}
pub fn attach(window: &gtk::ApplicationWindow, surface: &serde_json::Value) {
    let anchor = match surface["anchor"].as_str().unwrap_or("bottom") {
        "top" => 0,
        "left" => 2,
        "right" => 3,
        _ => 1,
    };
    let hide = match surface["hide_mode"].as_str().unwrap_or("never") {
        "maximized_focus_window" => 1,
        "overlapping_focus_window" => 2,
        "overlapping_window" => 3,
        "always" => 4,
        _ => 0,
    };
    let size = surface["size"]
        .as_i64()
        .unwrap_or(64)
        .clamp(1, i32::MAX as i64) as i32;
    // SAFETY: called on GTK's main thread after realization, borrowing the live GtkWindow.
    // The C adapter retains only its own Wayland proxy on the GdkSurface, with a destructor.
    let ok = unsafe {
        msdk_panel_attach(
            window.upcast_ref::<gtk::Window>().to_glib_none().0,
            anchor,
            hide,
            size,
        )
    };
    if ok == 0 {
        eprintln!(
            "msdk: panel placement unavailable: requires Wayland with the Pantheon shell protocol"
        );
    }
}
