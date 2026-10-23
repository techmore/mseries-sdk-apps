//! App-owned gettext domain. Missing catalogs fall back to the source text.
//! GTK initializes the process locale before any UI strings are requested.
pub fn tr(message: &str) -> String {
    gtk::glib::dgettext(Some("io.elementary.mseries.MessagesRust"), message).to_string()
}
