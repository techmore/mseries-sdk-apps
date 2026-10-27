//! The AI Providers page: what the T3 server reports for each provider instance (read-only).
//! Credentials and sign-in stay on the server; this only shows state and asks the server to re-probe.

use crate::chat::label;
use crate::t3::model::Provider;
use gtk::prelude::*;

/// One row per provider: name, version, state, models, and the server's message when it has one.
pub fn list(providers: &[Provider]) -> gtk::ListBox {
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    list.add_css_class("boxed-list");
    if providers.is_empty() {
        let hint = label("The server reports no providers.", &["dim-label"]);
        hint.set_margin_top(12);
        hint.set_margin_bottom(12);
        list.append(&hint);
    }
    for p in providers {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let icon = gtk::Image::from_icon_name(if p.ready() { "emblem-ok-symbolic" } else { "dialog-warning-symbolic" });
        icon.add_css_class(if p.ready() { "success" } else { "warning" });
        row.append(&icon);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        let title = match &p.version {
            Some(v) if !v.is_empty() => format!("{} {}", p.name(), v),
            _ => p.name(),
        };
        text.append(&label(&title, &["heading"]));
        let detail = detail(p);
        text.append(&label(&detail, &["caption", "dim-label"]));
        row.append(&text);
        row.append(&label(&p.state_text(), &["dim-label"]));
        list.append(&row);
        if let Some(row) = list.last_child() {
            row.update_property(&[gtk::accessible::Property::Label(&format!("{title}: {}", p.state_text()))]);
        }
    }
    list
}

fn detail(p: &Provider) -> String {
    let models = match p.models.len() {
        0 => "no models".to_string(),
        1 => "1 model".to_string(),
        n => format!("{n} models"),
    };
    match p.message.as_deref().filter(|m| !m.trim().is_empty()) {
        Some(m) => format!("{models} · {m}"),
        None => models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_lists_models_and_the_server_message() {
        let mut p = Provider::default();
        assert_eq!(detail(&p), "no models");
        p.models = vec![Default::default(), Default::default()];
        p.message = Some("Update available".into());
        assert_eq!(detail(&p), "2 models · Update available");
    }
}
