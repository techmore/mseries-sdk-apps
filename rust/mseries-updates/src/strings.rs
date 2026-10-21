//! Every user-facing string goes through these two functions so it can be extracted with
//! `xgettext -k_ -kn_:1,2` once the app gets a gettext domain. It has none yet: nothing is translated.

pub fn tr(text: &str) -> String {
    text.to_string()
}

/// Plural-aware form; `n` selects the singular only for exactly one (English rules until gettext is wired).
pub fn trn(singular: &str, plural: &str, n: usize) -> String {
    (if n == 1 { singular } else { plural }).replace("{n}", &n.to_string())
}
