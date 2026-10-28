//! App-owned gettext domain, named after the application id. Every user-facing string goes through
//! these two functions; a missing catalog falls back to the source text.

pub const DOMAIN: &str = "org.techmore.mseries.Updates";

pub fn tr(text: &str) -> String {
    mseries_pantheon_kit::i18n::tr(DOMAIN, text)
}

/// Plural-aware form; `{n}` is replaced by the count.
pub fn trn(singular: &str, plural: &str, n: usize) -> String {
    mseries_pantheon_kit::i18n::trn(DOMAIN, singular, plural, n)
}
