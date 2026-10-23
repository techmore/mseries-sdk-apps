use msdk::{Severity, agent, check, gate};
use serde_json::{Value, json};

fn load(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap())
        .unwrap()
}

fn labels(doc: &Value, pointer: &str) -> Vec<String> {
    agent::complete(doc, pointer).into_iter().map(|c| c.label).collect()
}

#[test]
fn every_diagnostic_code_has_an_explanation() {
    // Break a document in many ways and make sure each code the checker emits can be explained.
    let mut codes = std::collections::BTreeSet::new();
    let mut doc = load("quick-notes.ui.json");
    doc["app"]["id"] = json!("x");
    doc["sdk"] = json!(9);
    doc["surfaces"]["main"]["body"]["sidebar"]["children"][1]["row"]["text"] = json!("{nope}");
    doc["update"]["Search"] = json!([{ "set": "query", "to": 5 }, { "call": "nope.run", "args": {} }]);
    doc["feeds"] = json!({ "notes": "windows.list" });
    for d in check(&doc) {
        codes.insert(d.code);
    }
    assert!(codes.len() >= 5, "{codes:?}");
    for c in codes {
        assert!(agent::explain(c).is_some(), "no explanation for {c}");
    }
}

#[test]
fn describe_gives_a_valid_example_for_every_component() {
    for c in msdk::vocab::COMPONENTS {
        let d = agent::describe(c.name).unwrap();
        assert_eq!(d["example"]["type"], c.name);
    }
    assert!(
        agent::describe("process.run").is_some()
            && agent::describe("anchor").is_some()
            && agent::describe("nonsense").is_none()
    );
}

#[test]
fn completion_knows_values_keys_and_row_fields() {
    let notes = load("quick-notes.ui.json");
    let row_text = labels(&notes, "/surfaces/main/body/sidebar/children/1/row/text");
    assert!(row_text.contains(&"{title}".to_string()), "{row_text:?}");
    assert!(labels(&notes, "/surfaces/main/header/0/button_style").contains(&"destructive".to_string()));
    assert!(labels(&notes, "/surfaces/main/header/0/on").contains(&"AddNote".to_string()));
    assert!(labels(&notes, "/state/notes").contains(&"list<Note>".to_string()));
    assert!(labels(&notes, "/update/AddNote/0").contains(&"append".to_string()));
    let dock = load("dock.ui.json");
    assert_eq!(labels(&dock, "/surfaces/dock/body/row/on"), vec!["Activate".to_string()]);
    assert!(labels(&dock, "/surfaces/dock/hide_mode").contains(&"overlapping_focus_window".to_string()));
}

#[test]
fn gates_pass_for_the_example_and_fail_with_a_reason() {
    let doc = load("quick-notes.ui.json");
    let gates = load("quick-notes.gates.json");
    assert!(gate::run(&doc, Some(&gates)).iter().all(|r| r.passed));
    let bad = json!({ "gates": [{ "name": "wrong", "steps": [{ "send": "AddNote" }], "expect": [{ "list": "notes", "length": 5 }] }] });
    let r = gate::run(&doc, Some(&bad));
    let failed: Vec<_> = r.iter().filter(|x| !x.passed).collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0].details[0].contains("has 1 items, expected 5"));
    let mut broken = doc.clone();
    broken["update"].as_object_mut().unwrap().remove("Search");
    assert!(gate::run(&broken, None).iter().any(|r| r.name == "every-message-has-a-rule" && !r.passed));
    assert!(check(&broken).iter().all(|d| d.severity != Severity::Error || d.code != "never"));
}
