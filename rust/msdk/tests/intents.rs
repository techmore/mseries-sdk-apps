//! The agent surface: grammar round trip, checker rules, manifest, MCP tools and argument validation.

use msdk::intents::{self, validate_args};
use msdk::{Severity, check, gate, text, vocab};
use serde_json::{Value, json};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../claude-pantheon-rs/sdk");

fn t3() -> Value {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/t3.services.json")).unwrap()).unwrap();
    vocab::load_manifest(&manifest).unwrap();
    text::parse(&std::fs::read_to_string(format!("{DIR}/t3.msdk")).unwrap()).unwrap().doc
}

fn quick_notes() -> Value {
    let path = format!("{}/examples/quick-notes.ui.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn codes(doc: &Value) -> Vec<&'static str> {
    check(doc).into_iter().map(|d| d.code).collect()
}

/// A small valid agent block for quick-notes, edited by each test.
fn notes_with(agent: Value) -> Value {
    let mut doc = quick_notes();
    doc["agent"] = agent;
    doc
}

fn first_message(doc: &Value) -> String {
    doc["messages"].as_object().unwrap().keys().next().unwrap().clone()
}

#[test]
fn t3_declares_a_clean_agent_surface() {
    let doc = t3();
    assert!(check(&doc).is_empty(), "{:?}", check(&doc).iter().map(|d| d.to_string()).collect::<Vec<_>>());
    let m = intents::manifest(&doc).unwrap();
    assert_eq!(m["intents"].as_object().unwrap().len(), 5);
    // The permission prompts and the unattended runner are not reachable by an agent.
    for hidden in ["AllowOnce", "Deny", "AllowOldest", "ToggleRunner"] {
        assert!(m["intents"].get(hidden).is_none(), "{hidden} must not be exposed");
    }
    assert_eq!(m["scopes"]["chat.send"]["strongest_risk"], "external");
    assert_eq!(m["scopes"]["chat.read"]["strongest_risk"], "read");
    assert_eq!(m["entities"]["Thread"]["lists"], json!(["threads"]));
    assert!(m["entities"]["Thread"]["fields"].get("title").is_some());
    assert_eq!(m["intents"]["Send"]["shows"], json!(["selected", "draft"]));
}

#[test]
fn t3_intents_each_have_a_scenario_and_hostile_arguments_are_refused() {
    let doc = t3();
    let gates: Value = serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/t3.gates.json")).unwrap()).unwrap();
    let results = gate::run(&doc, Some(&gates));
    for name in ["every-intent-has-a-scenario", "intent-arguments-are-bounded"] {
        let r = results.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no gate {name}"));
        assert!(r.passed, "{name}: {:?}", r.details);
    }
    // Remove the scenarios and the coverage gate fails.
    let none = gate::run(&doc, None);
    assert!(!none.iter().find(|r| r.name == "every-intent-has-a-scenario").unwrap().passed);
}

#[test]
fn the_text_form_round_trips_and_is_idempotent() {
    let doc = t3();
    let printed = text::print(&doc).unwrap();
    assert!(printed.contains("intent SelectThread(thread: Text) {") && printed.contains("show selected, draft"));
    let again = text::parse(&printed).unwrap();
    assert_eq!(again.doc, doc);
    assert_eq!(text::print(&again.doc).unwrap(), printed);
    assert!(again.line_for("/agent/intents/Send").is_some());
}

#[test]
fn text_errors_name_the_line() {
    let e = text::parse("app \"A\" id a.b.c icon i\nintent X {\n  risky yes\n}\n").unwrap_err();
    assert!(e.message.contains("unknown intent property"), "{e}");
    let e = text::parse("app \"A\" id a.b.c icon i\nintent X(n: Number) {\n}\n").unwrap_err();
    assert!(e.message.contains("`Text` or `Bool`"), "{e}");
}

#[test]
fn a_small_agent_block_on_quick_notes_checks_clean() {
    let m = first_message(&quick_notes());
    let doc = notes_with(json!({
        "scopes": { "notes.write": "Create and edit notes" },
        "intents": { (m.as_str()): { "scope": "notes.write", "risk": "reversible", "verify": "the note list changes", "about": "Do the first thing." } }
    }));
    let errors: Vec<_> =
        check(&doc).into_iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn every_rule_fires() {
    let doc = t3();
    let mutate = |f: &dyn Fn(&mut Value)| {
        let mut d = doc.clone();
        f(&mut d);
        codes(&d)
    };
    let cases: Vec<(&str, Vec<&'static str>)> = vec![
        (
            "send without confirm",
            mutate(&|d| {
                d["agent"]["intents"]["Send"].as_object_mut().unwrap().remove("confirm");
            }),
        ),
        (
            "send without verify",
            mutate(&|d| {
                d["agent"]["intents"]["Send"].as_object_mut().unwrap().remove("verify");
            }),
        ),
        ("send understated", mutate(&|d| d["agent"]["intents"]["Send"]["risk"] = json!("read"))),
        ("start issue understated", mutate(&|d| d["agent"]["intents"]["StartIssue"]["risk"] = json!("reversible"))),
        ("bad risk", mutate(&|d| d["agent"]["intents"]["Send"]["risk"] = json!("dangerous"))),
        ("undeclared scope", mutate(&|d| d["agent"]["intents"]["Send"]["scope"] = json!("chat.nope"))),
        ("unknown message", mutate(&|d| d["agent"]["intents"]["Send"]["message"] = json!("Sendd"))),
        (
            "param on a no-payload message",
            mutate(&|d| d["agent"]["intents"]["Send"]["param"] = json!({"name":"x","type":"text"})),
        ),
        (
            "missing param",
            mutate(&|d| {
                d["agent"]["intents"]["SelectThread"].as_object_mut().unwrap().remove("param");
            }),
        ),
        ("wrong param type", mutate(&|d| d["agent"]["intents"]["SelectThread"]["param"]["type"] = json!("bool"))),
        (
            "no about",
            mutate(&|d| {
                d["agent"]["intents"]["Send"].as_object_mut().unwrap().remove("about");
            }),
        ),
        ("unknown field", mutate(&|d| d["agent"]["entities"]["Thread"]["expose"] = json!(["titel"]))),
        (
            "secret field",
            mutate(&|d| {
                d["types"]["Thread"]["api_token"] = json!("text");
                d["agent"]["entities"]["Thread"]["expose"] = json!(["api_token"]);
            }),
        ),
        (
            "entity of nothing",
            mutate(&|d| d["agent"]["entities"]["Ghost"] = json!({"scope":"chat.read","expose":["x"]})),
        ),
        ("bad scope name", mutate(&|d| d["agent"]["scopes"]["Chat Read"] = json!("x"))),
        ("scope without description", mutate(&|d| d["agent"]["scopes"]["chat.read"] = json!(""))),
        ("unused scope", mutate(&|d| d["agent"]["scopes"]["chat.unused"] = json!("Unused."))),
        ("show unknown state", mutate(&|d| d["agent"]["intents"]["Send"]["shows"] = json!(["drafft"]))),
        (
            "confirm shows nothing",
            mutate(&|d| {
                d["agent"]["intents"]["Send"].as_object_mut().unwrap().remove("shows");
            }),
        ),
        ("unknown agent key", mutate(&|d| d["agent"]["tools"] = json!({}))),
    ];
    let expect = [
        "agent-needs-confirm",
        "agent-needs-verify",
        "agent-risk-understated",
        "agent-risk-understated",
        "agent-bad-risk",
        "agent-undeclared-scope",
        "agent-unknown-message",
        "agent-param-mismatch",
        "agent-param-mismatch",
        "agent-param-mismatch",
        "agent-needs-about",
        "agent-unknown-field",
        "agent-secret-field",
        "agent-unknown-entity-type",
        "agent-bad-scope-name",
        "agent-scope-needs-description",
        "agent-unused-scope",
        "agent-unknown-state",
        "agent-confirm-shows-nothing",
        "unknown-property",
    ];
    for ((name, got), want) in cases.iter().zip(expect) {
        assert!(got.contains(&want), "{name}: wanted {want}, got {got:?}");
        assert!(msdk::agent::explain(want).is_some(), "no explanation for {want}");
    }
}

#[test]
fn the_manifest_hash_follows_meaning_not_spelling() {
    let doc = t3();
    let a = intents::manifest(&doc).unwrap();
    // Reformatting through the text form must not change it ...
    let again = text::parse(&text::print(&doc).unwrap()).unwrap().doc;
    assert_eq!(intents::manifest(&again).unwrap()["hash"], a["hash"]);
    // ... and widening an intent must.
    let mut wider = doc.clone();
    wider["agent"]["intents"]["EditDraft"]["scope"] = json!("chat.send");
    assert_ne!(intents::manifest(&wider).unwrap()["hash"], a["hash"]);
    assert_eq!(a["hash"].as_str().unwrap().len(), 64);
}

#[test]
fn an_app_with_errors_or_without_an_agent_block_has_no_manifest() {
    let mut bad = t3();
    bad["agent"]["intents"]["Send"]["risk"] = json!("read");
    assert!(intents::manifest(&bad).unwrap_err().contains("agent-risk-understated"));
    assert!(intents::manifest(&quick_notes()).unwrap_err().contains("exposes nothing"));
}

#[test]
fn mcp_tools_carry_risk_scope_and_the_manifest_hash() {
    let doc = t3();
    let hash = intents::manifest(&doc).unwrap()["hash"].clone();
    let tools = intents::mcp_tools(&doc).unwrap();
    let tools = tools["tools"].as_array().unwrap();
    let get = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap_or_else(|| panic!("no tool {n}"));
    let send = get("claudepantheonsdk_send");
    assert_eq!(send["annotations"]["readOnlyHint"], false);
    assert_eq!(send["annotations"]["destructiveHint"], true);
    assert_eq!(send["_meta"]["x-mseries-confirm"], true);
    assert_eq!(send["_meta"]["x-mseries-manifest"], hash);
    let list = get("claudepantheonsdk_list_thread");
    assert_eq!(list["annotations"]["readOnlyHint"], true);
    assert_eq!(list["_meta"]["x-mseries-scope"], "chat.read");
    let select = get("claudepantheonsdk_select_thread");
    assert_eq!(select["inputSchema"]["required"], json!(["thread"]));
}

#[test]
fn arguments_must_match_the_single_typed_parameter() {
    let m = intents::manifest(&t3()).unwrap();
    assert_eq!(validate_args(&m, "SelectThread", &json!({"thread":"t1"})).unwrap(), Some(json!("t1")));
    assert_eq!(validate_args(&m, "Send", &json!({})).unwrap(), None);
    for bad in [
        ("SelectThread", json!({})),
        ("SelectThread", json!({"thread": 7})),
        ("SelectThread", json!({"thread":"t1","also":"x"})),
        ("SelectThread", json!({"thread":"a\u{0}b"})),
        ("SelectThread", json!({"thread":"\u{1b}[2J"})),
        ("EditDraft", json!({"text":"x".repeat(intents::MAX_TEXT_ARG + 1)})),
        ("Send", json!({"text":"smuggled"})),
        ("Nope", json!({})),
        ("Send", json!("not an object")),
    ] {
        assert!(validate_args(&m, bad.0, &bad.1).is_err(), "{bad:?}");
    }
    // Newlines and tabs are ordinary text for a message box.
    assert!(validate_args(&m, "EditDraft", &json!({"text":"line one\nline two\tend"})).is_ok());
}

#[test]
fn the_schema_accepts_an_agent_block() {
    let schema = msdk::schema::schema();
    assert!(schema["properties"]["agent"]["properties"]["intents"].is_object());
}
