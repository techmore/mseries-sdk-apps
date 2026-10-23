use msdk::{Severity, check};
use serde_json::{Value, json};

fn load(name: &str) -> Value {
    let path = format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn codes(doc: &Value) -> Vec<&'static str> {
    check(doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.code).collect()
}

/// A valid document around one body component; tests mutate it.
fn base(body: Value) -> Value {
    json!({
        "sdk": 1,
        "app": { "id": "io.github.techmore.T", "name": "T", "icon": "x" },
        "capabilities": ["subprocess", "windows"],
        "types": { "Row": { "id": "text", "name": "text", "n": "number" } },
        "state": { "flag": "bool", "name": "text", "n": "number", "rows": "list<Row>" },
        "messages": { "Go": "none", "Said": "text", "Flip": "bool" },
        "update": { "Go": [{ "toggle": "flag" }], "Said": [{ "set": "name", "to": "$payload" }], "Flip": [{ "set": "flag", "to": "$payload" }] },
        "surfaces": { "main": { "kind": "window", "title": "T", "body": body } }
    })
}

fn with_update(update: Value) -> Value {
    let mut d = base(json!({ "type": "Text", "text": "{name}" }));
    d["update"] = update;
    d
}

#[test]
fn examples_have_no_errors() {
    for f in ["quick-notes.ui.json", "disk-space.ui.json", "dock.ui.json"] {
        assert_eq!(codes(&load(f)), Vec::<&str>::new(), "{f}");
    }
}

#[test]
fn custom_component_is_reported_as_a_gap() {
    assert!(
        check(&load("disk-space.ui.json"))
            .iter()
            .any(|d| d.code == "custom-component" && d.severity == Severity::Warning)
    );
}

#[test]
fn unknown_component_suggests_the_nearest() {
    let d = check(&base(json!({ "type": "Buton", "label": "a", "on": "Go" })));
    let e = d.iter().find(|d| d.code == "unknown-component").unwrap();
    assert!(e.fix.as_deref().unwrap().contains("Button"));
    assert_eq!(e.path, "/surfaces/main/body/type");
}

#[test]
fn missing_and_unknown_properties() {
    let d = codes(&base(json!({ "type": "Button", "on": "Go", "colour": "red" })));
    assert!(d.contains(&"missing-property") && d.contains(&"unknown-property"));
}

#[test]
fn empty_label_and_state_binding_types() {
    assert!(codes(&base(json!({ "type": "Switch", "label": "", "bind": "flag" }))).contains(&"empty-label"));
    assert!(codes(&base(json!({ "type": "Switch", "label": "a", "bind": "nope" }))).contains(&"unknown-state"));
    assert!(codes(&base(json!({ "type": "Switch", "label": "a", "bind": "name" }))).contains(&"state-type-mismatch"));
    assert!(codes(&base(json!({ "type": "Progress", "label": "a", "bind": "n" }))).is_empty());
}

#[test]
fn messages_must_be_declared_and_payloads_must_match() {
    assert!(codes(&base(json!({ "type": "Button", "label": "a", "on": "Stop" }))).contains(&"unknown-message"));
    assert!(
        codes(&base(json!({ "type": "Entry", "label": "a", "bind": "name", "on_change": "Go" })))
            .contains(&"message-payload-mismatch")
    );
    assert!(
        codes(&base(json!({ "type": "Switch", "label": "a", "bind": "flag", "on_change": "Said" })))
            .contains(&"message-payload-mismatch")
    );
}

#[test]
fn list_rows_see_record_fields_and_need_key_and_empty() {
    let ok = json!({ "type": "List", "items": "rows", "key": "id", "row": { "type": "Text", "text": "{name}" }, "empty": { "type": "EmptyState", "title": "None" } });
    assert!(codes(&base(ok)).is_empty());
    let bad_field = json!({ "type": "List", "items": "rows", "key": "id", "row": { "type": "Text", "text": "{nope}" }, "empty": { "type": "EmptyState", "title": "None" } });
    assert!(codes(&base(bad_field)).contains(&"unknown-field"));
    let bad_key = json!({ "type": "List", "items": "rows", "key": "n", "row": { "type": "Text", "text": "{name}" }, "empty": { "type": "EmptyState", "title": "None" } });
    assert!(codes(&base(bad_key)).contains(&"bad-key"));
    assert!(
        codes(&base(json!({ "type": "List", "items": "rows", "row": { "type": "Text", "text": "x" } })))
            .contains(&"missing-property")
    );
}

#[test]
fn row_fields_are_not_visible_outside_the_row() {
    assert!(codes(&base(json!({ "type": "Text", "text": "{id}" }))).contains(&"unknown-field"));
}

#[test]
fn design_rules() {
    assert!(
        codes(&base(json!({ "type": "Button", "label": "Delete", "on": "Go", "button_style": "destructive" })))
            .contains(&"destructive-needs-confirm")
    );
    let two = json!({ "type": "Box", "children": [
        { "type": "Button", "label": "a", "on": "Go", "button_style": "suggested" },
        { "type": "Button", "label": "b", "on": "Go", "button_style": "suggested" } ] });
    assert!(check(&base(two)).iter().any(|d| d.code == "multiple-suggested"));
    let pw = json!({ "type": "Entry", "label": "p", "bind": "name", "entry_kind": "password", "on_change": "Said" });
    assert!(check(&base(pw)).iter().any(|d| d.code == "password-change-handler"));
}

#[test]
fn enums_capabilities_ids_and_top_level() {
    assert!(codes(&base(json!({ "type": "Text", "text": "a", "text_style": "neon" }))).contains(&"bad-value"));
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["capabilities"] = json!(["filesystem"]);
    assert!(codes(&doc).contains(&"unknown-capability"));
    let dup = json!({ "type": "Box", "children": [{ "type": "Text", "text": "a", "id": "x" }, { "type": "Text", "text": "b", "id": "x" }] });
    assert!(codes(&base(dup)).contains(&"duplicate-id"));
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["app"]["id"] = json!("notes");
    doc["sdk"] = json!(2);
    doc["windw"] = json!({});
    let c = codes(&doc);
    assert!(c.contains(&"bad-app-id") && c.contains(&"unsupported-sdk-version") && c.contains(&"unknown-property"));
    assert_eq!(codes(&json!([])), vec!["not-object"]);
}

#[test]
fn surfaces_validate_their_own_properties() {
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["surfaces"]["dock"] = json!({ "kind": "panel", "anchor": "middle", "body": { "type": "Text", "text": "a" } });
    assert!(codes(&doc).contains(&"bad-value"));
    doc["surfaces"]["x"] = json!({ "kind": "pannel", "body": { "type": "Text", "text": "a" } });
    assert!(codes(&doc).contains(&"unknown-surface"));
}

#[test]
fn update_rules_are_type_checked() {
    assert!(codes(&with_update(json!({ "Go": [{ "set": "flag", "to": "yes" }] }))).contains(&"state-type-mismatch"));
    assert!(codes(&with_update(json!({ "Go": [{ "set": "name", "to": "$payload" }] }))).contains(&"no-payload"));
    assert!(codes(&with_update(json!({ "Go": [{ "set": "nope", "to": "x" }] }))).contains(&"unknown-state"));
    assert!(codes(&with_update(json!({ "Stop": [{ "toggle": "flag" }] }))).contains(&"unknown-message"));
    assert!(codes(&with_update(json!({ "Go": [{ "toggle": "name" }] }))).contains(&"state-type-mismatch"));
    assert!(
        codes(&with_update(json!({ "Go": [{ "set": "flag", "to": true, "toggle": "flag" }] }))).contains(&"bad-action")
    );
}

#[test]
fn list_actions_check_the_record() {
    let ok = json!({ "Go": [{ "append": "rows", "record": { "id": "$id", "name": "x", "n": 1 } }, { "remove": "rows", "where": "a" }, { "clear": "rows" }] });
    assert!(codes(&with_update(ok)).is_empty());
    assert!(
        codes(&with_update(json!({ "Go": [{ "append": "rows", "record": { "id": "a", "nme": "x" } }] })))
            .contains(&"unknown-field")
    );
    assert!(
        codes(&with_update(json!({ "Go": [{ "append": "rows", "record": { "id": 3 } }] })))
            .contains(&"state-type-mismatch")
    );
    assert!(
        codes(&with_update(json!({ "Go": [{ "patch": "rows", "where": "a", "fields": { "n": "x" } }] })))
            .contains(&"state-type-mismatch")
    );
}

#[test]
fn services_need_capabilities_and_typed_arguments() {
    let call = json!({ "Go": [{ "call": "process.run", "args": { "command": "ls" }, "then": "Said" }] });
    assert!(codes(&with_update(call.clone())).is_empty());
    let mut doc = with_update(call);
    doc["capabilities"] = json!([]);
    assert!(codes(&doc).contains(&"missing-capability"));
    assert!(
        codes(&with_update(json!({ "Go": [{ "call": "process.run", "args": {} }] }))).contains(&"missing-property")
    );
    assert!(codes(&with_update(json!({ "Go": [{ "call": "process.rn", "args": {} }] }))).contains(&"unknown-service"));
    assert!(
        codes(&with_update(json!({ "Go": [{ "call": "process.run", "args": { "command": "ls" }, "then": "Go" }] })))
            .contains(&"message-payload-mismatch")
    );
}

#[test]
fn feeds_must_match_the_record_shape() {
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["feeds"] = json!({ "rows": "windows.list" });
    assert!(codes(&doc).contains(&"feed-record-mismatch"));
}

#[test]
fn rust_logic_and_rules_do_not_mix() {
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc["logic"] = json!("rust");
    assert!(codes(&doc).contains(&"update-with-rust-logic"));
    let mut doc = base(json!({ "type": "Text", "text": "a" }));
    doc.as_object_mut().unwrap().remove("update");
    assert!(codes(&doc).contains(&"missing-property"));
}

#[test]
fn schema_and_tokens_are_derived_from_the_tables() {
    let s = msdk::schema::schema();
    let kinds = s["$defs"]["node"]["oneOf"].as_array().unwrap().len();
    assert_eq!(kinds, msdk::vocab::COMPONENTS.len());
    assert!(msdk::tokens::css().contains(".msdk-title"));
    assert_eq!(msdk::tokens::spacing_px("medium"), 10);
}

#[test]
fn stack_children_need_ids_and_tabs_bind_text() {
    let ok = json!({ "type": "Stack", "bind": "name", "children": [{ "type": "Text", "text": "a", "id": "One" }] });
    assert!(codes(&base(ok)).is_empty());
    let no_id = json!({ "type": "Stack", "bind": "name", "children": [{ "type": "Text", "text": "a" }] });
    assert!(codes(&base(no_id)).contains(&"stack-child-needs-id"));
    let tabs = json!({ "type": "Tabs", "label": "Pages", "bind": "flag", "options": ["A", "B"] });
    assert!(codes(&base(tabs)).contains(&"state-type-mismatch"));
}
