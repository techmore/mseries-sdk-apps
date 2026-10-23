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
