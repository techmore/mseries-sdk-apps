//! Checks the app conventions in `data/conventions.json` against the source tree (docs/APP-CONVENTIONS.md).
//! Plain file scanning: no GTK, no display.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn rust_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, extension, out);
        } else if path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
}

/// The text of every `.rs` file under `src/` (not tests or examples) and every `.css` under the crate.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    files(&dir.join("src"), "rs", &mut paths);
    files(&dir.join("data"), "css", &mut paths);
    files(&dir.join("src"), "css", &mut paths);
    paths
        .into_iter()
        .filter_map(|p| fs::read_to_string(&p).ok().map(|t| (p, t)))
        .collect()
}

fn has_hex_colour(text: &str) -> bool {
    text.match_indices('#').any(|(at, _)| {
        text[at + 1..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .count()
            == 6
    })
}

fn desktop_value(text: &str, key: &str) -> Option<String> {
    let header = text.split("\n[").next().unwrap_or(text);
    header
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")).map(str::to_string))
}

#[test]
fn every_gui_app_follows_the_conventions() {
    let conventions: Value =
        serde_json::from_str(include_str!("../data/conventions.json")).unwrap();
    let namespace = conventions["namespace"].as_str().unwrap();
    let rules: Vec<&str> = conventions["rules"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut problems: Vec<String> = Vec::new();

    for app in conventions["apps"].as_array().unwrap() {
        let name = app["name"].as_str().unwrap();
        let dir = rust_dir().join(app["dir"].as_str().unwrap());
        let exempt = app["exempt"].as_object().cloned().unwrap_or_default();
        for rule in exempt.keys() {
            if !rules.contains(&rule.as_str()) {
                problems.push(format!("{name}: exempt names an unknown rule `{rule}`"));
            }
        }
        let all = sources(&dir);
        let mut check = |rule: &str, ok: bool, message: String| {
            let exempted = exempt.contains_key(rule);
            if !ok && !exempted {
                problems.push(format!("{name}: [{rule}] {message}"));
            }
            if ok && exempted {
                problems.push(format!(
                    "{name}: [{rule}] now passes; delete its entry under `exempt`"
                ));
            }
        };
        let any = |needle: &str| all.iter().any(|(_, text)| text.contains(needle));

        // id: desktop entries, icons, StartupWMClass, registered application id.
        let mut id_problems = Vec::new();
        for desktop in app["desktop"].as_array().unwrap() {
            let file = desktop.as_str().unwrap();
            let id = file.trim_end_matches(".desktop");
            let valid = id.strip_prefix(namespace).is_some_and(|rest| {
                rest.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && rest.chars().all(|c| c.is_ascii_alphanumeric())
            });
            if !valid {
                id_problems.push(format!("{file} is not {namespace}<Name>.desktop"));
            }
            let path = dir.join("data").join(file);
            let Ok(text) = fs::read_to_string(&path) else {
                id_problems.push(format!("data/{file} is missing"));
                continue;
            };
            // A transitional entry of the same app (the Messages preview) may borrow its sibling's icon.
            let icon = app["icon_alias"][file].as_str().unwrap_or(id);
            if desktop_value(&text, "Icon").as_deref() != Some(icon) {
                id_problems.push(format!("{file}: Icon must be {icon}"));
            }
            if desktop_value(&text, "StartupWMClass").as_deref() != Some(id) {
                id_problems.push(format!("{file}: StartupWMClass must be {id}"));
            }
            if !dir.join("data").join(format!("{icon}.svg")).is_file()
                && !dir.join("data").join(format!("{icon}.png")).is_file()
            {
                id_problems.push(format!("data/{icon}.svg (the icon) is missing"));
            }
            if !any(&format!("\"{id}\"")) {
                id_problems.push(format!("no source registers application id {id}"));
            }
        }
        check("id", id_problems.is_empty(), id_problems.join("; "));

        // colours
        let offenders: Vec<String> = all
            .iter()
            .filter(|(path, text)| !path.ends_with("tests.rs") && has_hex_colour(text))
            .map(|(path, _)| {
                path.strip_prefix(&dir)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect();
        check(
            "colors",
            offenders.is_empty(),
            format!("hex colour literals in {offenders:?}"),
        );

        check(
            "color_scheme",
            any("follow_color_scheme"),
            "does not call appearance::follow_color_scheme".into(),
        );
        check(
            "window_size",
            any("appearance::size_window(")
                || any("appearance::WORKSPACE_MIN")
                || any("appearance::UTILITY_MIN"),
            "does not use the shared window sizes".into(),
        );
        check(
            "accessible_labels",
            any("set_accessible_label"),
            "never calls set_accessible_label".into(),
        );
        check(
            "about",
            any("AboutDialog")
                || any("chrome::install_app_actions")
                || any("chrome::install_about_action"),
            "has no About dialog".into(),
        );
        check(
            "shortcuts",
            any("set_accels_for_action")
                || any("chrome::install_app_actions")
                || any("chrome::install_about_action")
                || any("ShortcutController")
                || any("EventControllerKey"),
            "registers no keyboard shortcuts".into(),
        );
        check(
            "toasts",
            any("Toast") || any("chrome::toast_overlay"),
            "uses no toasts".into(),
        );
        check(
            "gettext",
            any("dgettext") || any("i18n::tr"),
            "binds no gettext domain".into(),
        );
    }
    assert!(
        problems.is_empty(),
        "convention problems:\n{}",
        problems.join("\n")
    );
}
