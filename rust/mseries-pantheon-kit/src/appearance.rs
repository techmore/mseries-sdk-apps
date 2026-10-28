//! What every first-party GUI app agrees on: following the Granite colour scheme and the window sizes.

use granite::prelude::SettingsExt;
use gtk::prelude::*;

/// Default and minimum size of a main window that holds a workspace: a sidebar and a content area (Messages,
/// Image Studio, Disk Space, System Monitor, T3, Apple Dev).
pub const WORKSPACE_DEFAULT: (i32, i32) = (1100, 760);
pub const WORKSPACE_MIN: (i32, i32) = (720, 520);

/// Default and minimum size of a single-column utility window (Software Updates, the Signal importer).
pub const UTILITY_DEFAULT: (i32, i32) = (560, 680);
pub const UTILITY_MIN: (i32, i32) = (360, 480);

/// Sets a window's default and minimum size.
pub fn size_window(window: &impl IsA<gtk::Window>, default: (i32, i32), minimum: (i32, i32)) {
    let window = window.as_ref();
    window.set_default_size(default.0, default.1);
    window.set_size_request(minimum.0, minimum.1);
}

/// Makes GTK's dark preference follow Granite's colour-scheme setting (the portal's
/// `prefers-color-scheme`), now and whenever the user changes it. Call once after `granite::init()`.
///
/// `override_var` names an environment variable (`"1"` dark, anything else light) for fixtures and screenshots.
#[allow(deprecated)] // Granite apps still drive GTK's prefer-dark setting.
pub fn follow_color_scheme(override_var: Option<&str>) {
    let Some(gtk_settings) = gtk::Settings::default() else {
        return;
    };
    if let Some(forced) = override_var.and_then(|name| std::env::var(name).ok()) {
        gtk_settings.set_gtk_application_prefer_dark_theme(forced == "1");
        return;
    }
    let granite_settings = granite::Settings::default();
    let apply = |granite_settings: &granite::Settings, gtk_settings: &gtk::Settings| {
        gtk_settings.set_gtk_application_prefer_dark_theme(
            granite_settings.prefers_color_scheme() == granite::SettingsColorScheme::Dark,
        );
    };
    apply(&granite_settings, &gtk_settings);
    granite_settings.connect_prefers_color_scheme_notify(move |granite_settings| {
        apply(granite_settings, &gtk_settings)
    });
}

#[cfg(test)]
mod tests {
    const CSS: &str = include_str!("../data/pantheon-kit.css");
    const TOKENS: &str = include_str!("../../msdk/data/tokens.json");

    fn defined_colours() -> Vec<(String, String)> {
        CSS.lines()
            .filter_map(|line| line.strip_prefix("@define-color pantheon_"))
            .map(|rest| {
                let (name, value) = rest
                    .trim_end_matches(';')
                    .split_once(' ')
                    .expect("name and value");
                (name.to_string(), value.to_string())
            })
            .collect()
    }

    #[test]
    fn the_css_palette_is_the_one_in_tokens_json() {
        let tokens: serde_json::Value = serde_json::from_str(TOKENS).unwrap();
        let palette = tokens["colors"]["pantheon"]
            .as_object()
            .expect("colors.pantheon");
        let defined = defined_colours();
        assert_eq!(
            defined.len(),
            palette.len(),
            "every token is defined in the CSS and vice versa"
        );
        for (name, value) in defined {
            assert_eq!(
                palette[&name].as_str(),
                Some(value.as_str()),
                "pantheon_{name}"
            );
        }
    }

    #[test]
    fn colours_in_the_css_come_from_the_palette_or_the_theme() {
        for (number, line) in CSS.lines().enumerate() {
            if line.starts_with("@define-color") || line.trim_start().starts_with("/*") {
                continue;
            }
            let hex = line.match_indices('#').any(|(at, _)| {
                line[at + 1..]
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .count()
                    >= 3
            });
            assert!(
                !hex,
                "line {}: use a pantheon_* colour or a theme colour, not a hex literal: {line}",
                number + 1
            );
        }
    }

    #[test]
    fn window_sizes_nest() {
        assert!(
            super::WORKSPACE_MIN.0 < super::WORKSPACE_DEFAULT.0
                && super::WORKSPACE_MIN.1 < super::WORKSPACE_DEFAULT.1
        );
        assert!(
            super::UTILITY_MIN.0 < super::UTILITY_DEFAULT.0
                && super::UTILITY_MIN.1 < super::UTILITY_DEFAULT.1
        );
    }
}
