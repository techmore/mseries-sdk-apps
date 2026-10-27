use msdk::vocab;
use serde_json::{Value, json};

fn service(name: &str) -> Value {
    json!({"name":name,"capability":"subprocess","then":"text","args":{"first":"text","second":"bool"}})
}

#[test]
fn malformed_later_declarations_cannot_publish_earlier_services() {
    let manifest = json!({"services":[service("atomic.invalid.first")],"feeds":[{"name":"atomic.invalid.feed","capability":"network","fields":{"id":"invalid"}}]});
    assert!(vocab::load_manifest(&manifest).is_err());
    assert!(vocab::service("atomic.invalid.first").is_none());
    assert!(vocab::feed("atomic.invalid.feed").is_none());
}

#[test]
fn identical_reload_is_idempotent_even_when_argument_order_changes() {
    let manifest = json!({"manifest_version":1,"services":[service("atomic.reload")]});
    assert_eq!(vocab::load_manifest(&manifest).unwrap(), 1);
    let before = vocab::service("atomic.reload").unwrap() as *const _;
    let mut reordered = manifest;
    reordered["services"][0]["args"] = serde_json::from_str(r#"{"second":"bool","first":"text"}"#).unwrap();
    assert_eq!(vocab::load_manifest(&reordered).unwrap(), 1);
    assert_eq!(vocab::service("atomic.reload").unwrap() as *const _, before);
}

#[test]
fn conflict_does_not_replace_a_contract_or_publish_other_entries() {
    vocab::load_manifest(&json!({"services":[service("atomic.conflict.original")]})).unwrap();
    let mut conflicting = service("atomic.conflict.original");
    conflicting["then"] = json!("bool");
    let error = vocab::load_manifest(&json!({"services":[service("atomic.conflict.new"), conflicting]})).unwrap_err();
    assert!(error.contains("conflicting service"));
    assert!(vocab::service("atomic.conflict.new").is_none());
    assert_eq!(vocab::service("atomic.conflict.original").unwrap().then, vocab::Payload::Text);
}

#[test]
fn feed_shape_conflicts_are_atomic_and_identical_reloads_are_safe() {
    let mut manifest =
        json!({"feeds":[{"name":"atomic.feed","capability":"network","fields":{"id":"text","active":"bool"}}]});
    assert_eq!(vocab::load_manifest(&manifest).unwrap(), 1);
    manifest["feeds"][0]["fields"] = serde_json::from_str(r#"{"active":"bool","id":"text"}"#).unwrap();
    assert_eq!(vocab::load_manifest(&manifest).unwrap(), 1);
    manifest["feeds"][0]["fields"]["active"] = json!("text");
    manifest["services"] = json!([service("atomic.feed.rejected.service")]);
    assert!(vocab::load_manifest(&manifest).unwrap_err().contains("conflicting feed"));
    assert!(vocab::service("atomic.feed.rejected.service").is_none());
    assert!(vocab::feed("atomic.feed").unwrap().fields.contains(&("active", "bool")));
}

#[test]
fn malformed_shapes_unknown_fields_and_duplicate_names_are_rejected() {
    for manifest in [
        json!(null),
        json!([]),
        json!({"services":{}}),
        json!({"feeds":null}),
        json!({"servcies":[]}),
        json!({"services":[{"name":"x","capability":"subprocess","args":[]}]}),
        json!({"services":[{"name":"x","capability":"subprocess","then":17}]}),
        json!({"services":[{"name":"x","capability":"subprocess","about":false}]}),
        json!({"feeds":[{"name":"x","capability":"network","fields":false}]}),
        json!({"services":[service("atomic.duplicate"),service("atomic.duplicate")]}),
    ] {
        assert!(vocab::load_manifest(&manifest).is_err(), "accepted {manifest}");
    }
    assert!(vocab::service("atomic.duplicate").is_none());
}

#[test]
fn unsupported_manifest_versions_are_refused_before_registration() {
    for version in [json!(2), json!(0), json!("1"), json!(1.5), json!(null)] {
        let error = vocab::load_manifest(&json!({"manifest_version":version,"services":[service("atomic.future")]}))
            .unwrap_err();
        assert!(error.contains("unsupported service manifest version"));
        assert!(vocab::service("atomic.future").is_none());
    }
    assert_eq!(vocab::load_manifest(&json!({"services":[service("atomic.legacy")]})).unwrap(), 1);
}

#[test]
fn concurrent_conflicting_loads_have_one_winner_and_no_partial_publication() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs: Vec<_> = ["text", "bool"]
        .into_iter()
        .map(|payload| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut shared = service("atomic.concurrent.shared");
                shared["then"] = json!(payload);
                let own = format!("atomic.concurrent.{payload}");
                barrier.wait();
                let result = vocab::load_manifest(&json!({"services":[service(&own),shared]}));
                (own, result.is_ok())
            })
        })
        .collect();
    let results: Vec<_> = jobs.into_iter().map(|job| job.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|(_, success)| *success).count(), 1);
    for (name, success) in results {
        assert_eq!(vocab::service(&name).is_some(), success);
    }
}

#[test]
fn incompatible_documents_get_migration_guidance_without_mutation() {
    let mut doc: Value = serde_json::from_str(include_str!("../examples/home-storage.ui.json")).unwrap();
    doc["sdk"] = json!(2);
    let original = doc.clone();
    let diagnostics = msdk::check(&doc);
    let version = diagnostics.iter().find(|d| d.code == "unsupported-sdk-version").unwrap();
    assert!(version.fix.as_deref().unwrap().contains("migrate"));
    assert_eq!(doc, original);
    assert!(msdk::generate::models(&doc).is_err());
    assert!(msdk::agent::explain("unsupported-sdk-version").unwrap().contains("migrate"));
}
