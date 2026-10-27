use msdk::{Severity, check, gate, templates};

#[test]
fn every_template_is_valid_and_passes_its_own_gates() {
    for t in templates::TEMPLATES {
        let doc: serde_json::Value = serde_json::from_str(t.ui).unwrap_or_else(|e| panic!("{}: {e}", t.name));
        let errors: Vec<String> =
            check(&doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", t.name);
        let gates: serde_json::Value = serde_json::from_str(t.gates).unwrap();
        let failed: Vec<_> = gate::run(&doc, Some(&gates))
            .into_iter()
            .filter(|r| !r.passed)
            .map(|r| format!("{}: {:?}", r.name, r.details))
            .collect();
        assert!(failed.is_empty(), "{}: {failed:?}", t.name);
    }
}

#[test]
fn renamed_templates_stay_valid() {
    for t in templates::TEMPLATES {
        let doc: serde_json::Value = serde_json::from_str(&t.render("com.example.My.App", "My App")).unwrap();
        assert!(check(&doc).iter().all(|d| d.severity != Severity::Error), "{}", t.name);
        assert_eq!(doc["app"]["id"], "com.example.My.App");
        assert_eq!(doc["app"]["name"], "My App");
    }
}

#[test]
fn template_names_are_unique_and_documented() {
    let mut seen = std::collections::BTreeSet::new();
    for t in templates::TEMPLATES {
        assert!(seen.insert(t.name) && !t.covers.is_empty() && !t.about.is_empty());
    }
    assert!(templates::find("settings").is_some() && templates::find("nope").is_none());
}

#[test]
fn custom_names_are_literal_json_strings_in_every_starter() {
    for template in templates::TEMPLATES {
        let name = format!("{} \"Notes\" \\ café\nsecond line", template.id);
        let rendered = template.render("com.example.Notes", &name);
        let doc: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(doc["app"]["name"], name, "{}", template.name);
        assert_eq!(doc["app"]["id"], "com.example.Notes");
        let text = msdk::text::print(&doc).unwrap();
        assert!(!text.is_empty());
    }
}

#[test]
fn replacement_values_are_not_expanded_as_template_placeholders() {
    for template in templates::TEMPLATES {
        let doc: serde_json::Value = serde_json::from_str(&template.render(template.display, template.id)).unwrap();
        assert_eq!(doc["app"]["id"], template.display);
        assert_eq!(doc["app"]["name"], template.id);
    }
}
