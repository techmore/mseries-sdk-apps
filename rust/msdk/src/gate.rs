//! Acceptance gates: scripted scenarios run headless against the rules engine, with a log a model or CI can audit.
//!
//! Built-in gates always run (the checker is clean; every message has a rule). A gates file adds scenarios:
//! `{"gates":[{"name":"...","steps":[{"send":"AddNote"},{"send":"Search","payload":"x"},{"set":"key","to":1}],
//! "expect":[{"state":"has_selection","equals":true},{"list":"notes","length":1},{"list":"notes","item":{"title":"New note"}},
//! {"toast":"Note deleted"},{"call":"process.run"}]}]}`.

use serde_json::{Value, json};

use crate::logic::{Effect, Logic, RulesLogic, Store};
use crate::{Severity, check};

#[derive(Debug, Clone)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub skipped: bool,
    pub details: Vec<String>,
}

impl GateResult {
    pub fn to_json(&self) -> Value {
        json!({ "gate": self.name, "result": if self.skipped { "skipped" } else if self.passed { "pass" } else { "fail" }, "details": self.details })
    }
}

fn builtin(doc: &Value, gates: Option<&Value>) -> Vec<GateResult> {
    let errors: Vec<String> =
        check(doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
    let mut out =
        vec![GateResult { name: "checker-clean".into(), passed: errors.is_empty(), skipped: false, details: errors }];
    let bad = crate::contrast::failures();
    out.push(GateResult {
        name: "sdk-contrast".into(),
        passed: bad.is_empty(),
        skipped: false,
        details: bad
            .iter()
            .map(|f| {
                format!(
                    "{} ({}{}): {:.2}:1, needs {}",
                    f.pair,
                    f.theme,
                    f.accent.as_deref().map(|a| format!(", {a}")).unwrap_or_default(),
                    f.ratio,
                    f.needed
                )
            })
            .collect(),
    });
    let rust = doc.get("logic").and_then(Value::as_str) == Some("rust");
    let missing: Vec<String> = doc
        .get("messages")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(m, _)| !doc.get("update").and_then(Value::as_object).is_some_and(|u| u.contains_key(*m)))
        .map(|(m, _)| format!("message `{m}` has no update rule"))
        .collect();
    out.push(GateResult {
        name: "every-message-has-a-rule".into(),
        passed: rust || missing.is_empty(),
        skipped: rust,
        details: if rust { vec!["logic is rust".into()] } else { missing },
    });
    out.extend(intent_gates(doc, gates));
    out
}

/// With an `agent` block, every intent needs a scenario that names it (`"intent": "Send"`), and every text parameter must
/// refuse hostile arguments before the message is sent.
fn intent_gates(doc: &Value, gates: Option<&Value>) -> Vec<GateResult> {
    let Some(intents) = doc.pointer("/agent/intents").and_then(Value::as_object).filter(|i| !i.is_empty()) else {
        return vec![];
    };
    let covered: Vec<&str> =
        gates.and_then(|g| g["gates"].as_array()).into_iter().flatten().filter_map(|g| g["intent"].as_str()).collect();
    let missing: Vec<String> = intents
        .keys()
        .filter(|k| !covered.contains(&k.as_str()))
        .map(|k| format!("intent `{k}` has no scenario with \"intent\": \"{k}\""))
        .collect();
    let mut out = vec![GateResult {
        name: "every-intent-has-a-scenario".into(),
        passed: missing.is_empty(),
        skipped: false,
        details: missing,
    }];
    let hostile = ["\u{0}", "\u{1b}[2J", "\u{202e}", &"x".repeat(crate::intents::MAX_TEXT_ARG + 1)];
    let mut details = vec![];
    match crate::intents::manifest(doc) {
        Ok(m) => {
            for (name, i) in m["intents"].as_object().into_iter().flatten() {
                let Some((pname, schema)) = i["input"]["properties"].as_object().and_then(|p| p.iter().next()) else {
                    continue;
                };
                if schema["type"] != "string" {
                    continue;
                }
                for h in hostile {
                    let args = json!({ pname.as_str(): h });
                    if h != "\u{202e}" && crate::intents::validate_args(&m, name, &args).is_ok() {
                        details
                            .push(format!("intent `{name}` accepted a hostile argument ({} chars)", h.chars().count()));
                    }
                }
                if crate::intents::validate_args(&m, name, &json!({ pname.as_str(): "ordinary text" })).is_err() {
                    details.push(format!("intent `{name}` refused ordinary text"));
                }
            }
        }
        Err(e) => details.push(e),
    }
    out.push(GateResult {
        name: "intent-arguments-are-bounded".into(),
        passed: details.is_empty(),
        skipped: false,
        details,
    });
    out
}

fn initial(doc: &Value) -> Store {
    crate::logic::initial_store(doc)
}

fn scenario(doc: &Value, g: &Value) -> GateResult {
    let name = g["name"].as_str().unwrap_or("unnamed").to_string();
    if doc.get("logic").and_then(Value::as_str) == Some("rust") {
        return GateResult {
            name,
            passed: true,
            skipped: true,
            details: vec!["logic is rust; run this scenario against the Rust Logic".into()],
        };
    }
    let (mut store, mut logic, mut effects, mut details) =
        (initial(doc), RulesLogic::new(doc), Vec::<Effect>::new(), Vec::<String>::new());
    for (i, step) in g["steps"].as_array().into_iter().flatten().enumerate() {
        if let Some(m) = step["send"].as_str() {
            effects.extend(logic.update(&mut store, m, step.get("payload").cloned()));
        } else if let Some(k) = step["set"].as_str() {
            store.insert(k.into(), step["to"].clone());
        } else {
            details.push(format!("step {i}: expected {{\"send\":...}} or {{\"set\":...}}"));
        }
    }
    for (i, e) in g["expect"].as_array().into_iter().flatten().enumerate() {
        let at = |msg: String| format!("expect {i}: {msg}");
        if let Some(k) = e["state"].as_str() {
            let got = store.get(k).cloned().unwrap_or(Value::Null);
            if !same(&got, &e["equals"]) {
                details.push(at(format!("state `{k}` is {got}, expected {}", e["equals"])));
            }
        } else if let Some(k) = e["list"].as_str() {
            let list = store.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
            if let Some(n) = e["length"].as_u64()
                && list.len() as u64 != n
            {
                details.push(at(format!("list `{k}` has {} items, expected {n}", list.len())));
            }
            if let Some(want) = e["item"].as_object()
                && !list.iter().any(|it| want.iter().all(|(f, v)| it.get(f) == Some(v)))
            {
                details.push(at(format!("no item in `{k}` matches {}", e["item"])));
            }
        } else if let Some(t) = e["toast"].as_str() {
            if !effects.contains(&Effect::Toast(t.into())) {
                details.push(at(format!("no toast \"{t}\"")));
            }
        } else if let Some(svc) = e["call"].as_str() {
            if !effects.iter().any(|f| matches!(f, Effect::Call { service, .. } if service == svc)) {
                details.push(at(format!("service `{svc}` was not called")));
            }
        } else {
            details.push(at("unknown expectation".into()));
        }
    }
    GateResult { name, passed: details.is_empty(), skipped: false, details }
}

pub fn run(doc: &Value, gates: Option<&Value>) -> Vec<GateResult> {
    let mut out = builtin(doc, gates);
    for g in gates.and_then(|g| g["gates"].as_array()).into_iter().flatten() {
        out.push(scenario(doc, g));
    }
    out
}

/// Equality that treats 1 and 1.0 as the same number.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}
