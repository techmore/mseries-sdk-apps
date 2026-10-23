//! The T3 port written in the SDK's text form (claude-pantheon-rs/sdk): valid, gated, and stable under the formatter.

use msdk::{Severity, check, gate, text, vocab};
use serde_json::Value;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../claude-pantheon-rs/sdk");

fn load() -> text::Parsed {
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/t3.services.json")).unwrap()).unwrap();
    vocab::load_manifest(&manifest).unwrap();
    text::parse(&std::fs::read_to_string(format!("{DIR}/t3.msdk")).unwrap()).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn t3_checks_clean_with_its_service_manifest() {
    let p = load();
    let errors: Vec<String> =
        check(&p.doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(check(&p.doc).is_empty(), "t3.msdk should have no warnings either");
}

#[test]
fn t3_passes_its_gates() {
    let p = load();
    let gates: Value = serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/t3.gates.json")).unwrap()).unwrap();
    let results = gate::run(&p.doc, Some(&gates));
    assert!(results.len() >= 10);
    let failed: Vec<_> = results.iter().filter(|r| !r.passed).map(|r| format!("{}: {:?}", r.name, r.details)).collect();
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn t3_is_stable_under_the_formatter() {
    let p = load();
    let once = text::print(&p.doc).unwrap();
    let again = text::parse(&once).unwrap();
    assert_eq!(again.doc, p.doc);
    assert_eq!(text::print(&again.doc).unwrap(), once);
}

#[test]
fn the_registry_knows_the_plug_in_services() {
    load();
    assert!(vocab::service("t3.send").is_some() && vocab::feed("t3.threads").is_some());
    assert!(msdk::agent::describe("t3.send").is_some());
}
