use msdk::{Severity, check};
use serde_json::{Value, json};

fn load(name: &str) -> Value {
    let path = format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn codes(doc: &Value) -> Vec<&'static str> {
    check(doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.code).collect()
}

fn base(body: Value) -> Value {
    json!({
        "app": { "id": "io.github.techmore.T", "name": "T", "icon": "x" },
        "state": { "flag": "bool", "name": "text", "n": "number", "rows": "list" },
        "messages": ["Go"],
        "window": { "title": "T", "body": body }
    })
}

#[test]
fn examples_have_no_errors() {
    for f in ["quick-notes.ui.json", "disk-space.ui.json"] {
        assert_eq!(codes(&load(f)), Vec::<&str>::new(), "{f}");
    }
}

#[test]
fn custom_component_is_reported_as_a_gap() {
    let d = check(&load("disk-space.ui.json"));
    assert!(d.iter().any(|d| d.code == "custom-component" && d.severity == Severity::Warning));
}

#[test]
fn unknown_component_suggests_the_nearest() {
    let d = check(&base(json!({ "type": "Buton", "label": "a", "on": "Go" })));
    let e = d.iter().find(|d| d.code == "unknown-component").unwrap();
    assert!(e.fix.as_deref().unwrap().contains("Button"));
    assert_eq!(e.path, "/window/body/type");
}

#[test]
fn missing_label_and_unknown_property() {
    let d = codes(&base(json!({ "type": "Button", "on": "Go", "colour": "red" })));
    assert!(d.contains(&"missing-property"));
    assert!(d.contains(&"unknown-property"));
}

#[test]
fn empty_label_is_rejected() {
    assert!(codes(&base(json!({ "type": "Switch", "label": "", "bind": "flag" }))).contains(&"empty-label"));
}

#[test]
fn bind_must_exist_and_have_the_right_type() {
    assert!(codes(&base(json!({ "type": "Switch", "label": "a", "bind": "nope" }))).contains(&"unknown-state"));
    assert!(codes(&base(json!({ "type": "Switch", "label": "a", "bind": "name" }))).contains(&"state-type-mismatch"));
    assert!(codes(&base(json!({ "type": "Progress", "label": "a", "bind": "n" }))).is_empty());
}

#[test]
fn messages_must_be_declared() {
    assert!(codes(&base(json!({ "type": "Button", "label": "a", "on": "Stop" }))).contains(&"unknown-message"));
}

#[test]
fn list_needs_an_empty_state() {
    let d = codes(&base(json!({ "type": "List", "items": "rows", "row": { "type": "Text", "text": "x" } })));
    assert!(d.contains(&"missing-property"));
}

#[test]
fn bad_enum_value_and_capability() {
    assert!(codes(&base(json!({ "type": "Text", "text": "a", "text_style": "neon" }))).contains(&"bad-value"));
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["capabilities"] = json!(["filesystem"]);
    assert!(codes(&doc).contains(&"unknown-capability"));
}

#[test]
fn duplicate_ids_and_bad_app_id() {
    let body = json!({ "type": "Box", "children": [
        { "type": "Text", "text": "a", "id": "x" }, { "type": "Text", "text": "b", "id": "x" } ] });
    assert!(codes(&base(body)).contains(&"duplicate-id"));
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["app"]["id"] = json!("notes");
    assert!(codes(&doc).contains(&"bad-app-id"));
}

#[test]
fn top_level_typos_and_non_objects() {
    assert_eq!(codes(&json!([])), vec!["not-object"]);
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["windw"] = json!({});
    assert!(codes(&doc).contains(&"unknown-property"));
}
