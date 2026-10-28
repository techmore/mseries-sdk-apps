//! App-owned gettext domain, named after the application id. A missing catalog falls back to the source text.

pub const DOMAIN: &str = "org.techmore.mseries.SystemMonitor";

pub fn tr(text: &str) -> String {
    mseries_pantheon_kit::i18n::tr(DOMAIN, text)
}
