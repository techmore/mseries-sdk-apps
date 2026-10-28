//! App-owned gettext: each app has a domain named after its application id and routes its strings through
//! `tr`. Catalogs install to the system locale directory; a missing catalog falls back to the source text.
//! GTK initializes the process locale before any UI string is requested.

pub fn tr(domain: &str, message: &str) -> String {
    gtk::glib::dgettext(Some(domain), message).to_string()
}

/// Plural-aware: `{n}` in the chosen form is replaced by the count.
pub fn trn(domain: &str, singular: &str, plural: &str, n: usize) -> String {
    gtk::glib::dngettext(Some(domain), singular, plural, n as u64).replace("{n}", &n.to_string())
}
