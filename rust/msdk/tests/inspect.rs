use msdk::inspect::{outline, render};
use msdk::logic::Store;
use serde_json::{Value, json};

fn notes() -> Value {
    serde_json::from_str(include_str!("../examples/quick-notes.ui.json")).unwrap()
}

fn store(v: Value) -> Store {
    v.as_object().unwrap().clone()
}

#[test]
fn hidden_controls_are_left_out_and_shown_when_their_flag_is_true() {
    let doc = notes();
    let before = outline(&render(&doc, "main", None).unwrap());
    assert!(before.contains("New note") && !before.contains("Delete note"), "{before}");
    let after = outline(&render(&doc, "main", Some(&store(json!({ "has_selection": true })))).unwrap());
    assert!(after.contains("Delete note"), "{after}");
}

#[test]
fn list_rows_resolve_templates_and_carry_their_key() {
    let st = store(
        json!({ "notes": [ { "id": "n1", "title": "Milk", "when": "", "body": "" }, { "id": "n2", "title": "Eggs", "when": "", "body": "" } ] }),
    );
    let tree = render(&notes(), "main", Some(&st)).unwrap();
    let text = outline(&tree);
    assert!(text.contains("row n1") && text.contains("\"Milk\"") && text.contains("row n2"), "{text}");
    assert!(!text.contains("(empty)"), "an empty state only shows for an empty list: {text}");
    assert!(outline(&render(&notes(), "main", None).unwrap()).contains("No notes yet"));
}

#[test]
fn secrets_are_masked_and_missing_surfaces_are_errors() {
    let greeter: Value = serde_json::from_str(include_str!("../../mseries-greeter/data/greeter.ui.json")).unwrap();
    let st = store(json!({ "needs_answer": true, "answer": "hunter2" }));
    let text = outline(&render(&greeter, "main", Some(&st)).unwrap());
    assert!(!text.contains("hunter2") && text.contains("••••"), "{text}");
    assert!(render(&greeter, "nope", None).is_err());
}

#[test]
fn row_buttons_show_the_key_they_send() {
    let doc: Value = serde_json::from_str(include_str!("../templates/launcher-panel/app.ui.json")).unwrap();
    let tree = render(&doc, "dock", None).unwrap();
    let text = outline(&tree);
    assert!(text.contains("click: Launch(io.elementary.files.desktop)"), "{text}");
}

#[test]
fn a_stack_shows_only_the_selected_page_and_tabs_report_their_value() {
    let manifest: Value = serde_json::from_str(include_str!("../../claude-pantheon-rs/sdk/t3.services.json")).unwrap();
    msdk::vocab::load_manifest(&manifest).unwrap();
    let t3 = msdk::text::parse(include_str!("../../claude-pantheon-rs/sdk/t3.msdk")).unwrap().doc;
    let initial = outline(&render(&t3, "main", None).unwrap());
    assert!(initial.contains("Tabs \"Pages\" value=Usage"), "{initial}");
    let chat = outline(&render(&t3, "main", Some(&store(json!({ "page": "Chat" })))).unwrap());
    assert!(
        chat.contains("Tabs \"Pages\" value=Chat")
            && chat.contains("Transcript")
            && !chat.contains("Token history, server plan windows"),
        "{chat}"
    );
    let usage = outline(&render(&t3, "main", Some(&store(json!({ "page": "Usage" })))).unwrap());
    assert!(usage.contains("Token history, server plan windows") && !usage.contains("Transcript"), "{usage}");
}
