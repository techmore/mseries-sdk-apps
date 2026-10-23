use super::*;
use msdk::{intents, text, vocab};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../claude-pantheon-rs/sdk");
const APP: &str = "io.github.techmore.ClaudePantheonRs";
const AGENT: &str = "agent:test";

fn t3_manifest() -> Value {
    let services: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{DIR}/t3.services.json")).unwrap())
            .unwrap();
    vocab::load_manifest(&services).unwrap();
    let doc = text::parse(&std::fs::read_to_string(format!("{DIR}/t3.msdk")).unwrap())
        .unwrap()
        .doc;
    intents::manifest(&doc).unwrap()
}

fn broker() -> Broker {
    let mut b = Broker::in_memory();
    b.register(t3_manifest()).unwrap();
    b
}

fn call<'a>(target: Target<'a>, args: Value, shown: Value) -> Request<'a> {
    Request {
        agent: AGENT,
        app_id: APP,
        target,
        args,
        shown,
    }
}

fn ask(b: &mut Broker, r: &Request, now: u64) -> Prompt {
    match b.decide(r, now) {
        Outcome::Ask(p) => *p,
        other => panic!("expected a prompt, got {other:?}"),
    }
}

fn send(draft: &str) -> Request<'static> {
    call(
        Target::Intent("Send"),
        json!({}),
        json!({ "selected": "t1", "draft": draft }),
    )
}

#[test]
fn first_use_asks_then_a_timed_grant_runs_until_it_expires() {
    let mut b = broker();
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let p = ask(&mut b, &r, 100);
    assert_eq!(
        (p.scope.as_str(), p.risk),
        ("chat.create", Risk::Reversible)
    );
    assert_eq!(p.scope_about, "Start new threads");
    assert!(p.choices.contains(&Answer::For(3600)) && !p.choices.contains(&Answer::Widen));
    b.answer(p.id, Answer::For(600), 100).unwrap();
    assert_eq!(b.decide(&r, 200), Outcome::Run(None));
    assert!(matches!(b.decide(&r, 100 + 601), Outcome::Ask(_)));
}

#[test]
fn allow_once_for_a_reversible_scope_is_one_call() {
    let mut b = broker();
    let r = call(
        Target::Intent("SelectThread"),
        json!({"thread":"t1"}),
        json!({}),
    );
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Once, 0).unwrap();
    assert_eq!(b.decide(&r, 1), Outcome::Run(Some(json!("t1"))));
    assert!(matches!(b.decide(&r, 2), Outcome::Ask(_)));
}

#[test]
fn external_actions_are_approved_per_action_and_the_approval_is_bound_to_what_was_shown() {
    let mut b = broker();
    let p = ask(&mut b, &send("hello"), 0);
    assert_eq!(p.risk, Risk::External);
    assert_eq!(p.shown["draft"], "hello");
    assert_eq!(p.choices, vec![Answer::Deny, Answer::Once, Answer::Widen]);
    b.answer(p.id, Answer::Once, 0).unwrap();
    // The draft changed after the owner approved "hello": the approval does not carry over.
    assert!(matches!(
        b.decide(&send("hello, and also delete everything"), 5),
        Outcome::Ask(_)
    ));
    // The approved action runs once ...
    assert_eq!(b.decide(&send("hello"), 6), Outcome::Run(None));
    // ... and is spent.
    assert!(matches!(b.decide(&send("hello"), 7), Outcome::Ask(_)));
}

#[test]
fn a_per_action_approval_expires() {
    let mut b = broker();
    let p = ask(&mut b, &send("hi"), 0);
    b.answer(p.id, Answer::Once, 0).unwrap();
    assert!(matches!(
        b.decide(&send("hi"), APPROVAL_SECS + 1),
        Outcome::Ask(_)
    ));
}

#[test]
fn widening_is_offered_only_for_external_scopes_and_lasts_until_revoked() {
    let mut b = broker();
    let p = ask(&mut b, &send("one"), 0);
    b.answer(p.id, Answer::Widen, 0).unwrap();
    assert_eq!(b.decide(&send("anything else"), 10), Outcome::Run(None));
    assert_eq!(b.revoke(AGENT, Some(APP)), 1);
    assert!(matches!(
        b.decide(&send("anything else"), 11),
        Outcome::Ask(_)
    ));

    // Always / For / widen are not valid answers where they are not offered.
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let p = ask(&mut b, &r, 20);
    assert!(b.answer(p.id, Answer::Widen, 20).is_err());
    let p = ask(&mut b, &send("two"), 21);
    assert!(b.answer(p.id, Answer::Always, 21).is_err());
    assert!(b.answer(p.id, Answer::For(3600), 21).is_err());
    // The prompt is still open after a refused answer.
    assert!(b.answer(p.id, Answer::Once, 22).is_ok());
}

#[test]
fn a_changed_manifest_asks_again() {
    let mut b = broker();
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Always, 0).unwrap();
    assert_eq!(b.decide(&r, 1), Outcome::Run(None));
    // The app updates and its manifest changes (a different hash).
    let mut changed = t3_manifest();
    changed["intents"]["EditDraft"]["scope"] = json!("chat.send");
    changed["hash"] = json!(sha256_hex(b"a different manifest"));
    b.register(changed).unwrap();
    assert!(matches!(b.decide(&r, 2), Outcome::Ask(_)));
}

#[test]
fn a_denial_is_remembered_and_cannot_be_nagged_past() {
    let mut b = broker();
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Deny, 0).unwrap();
    assert_eq!(b.decide(&r, 10), Outcome::Refuse(Refusal::Denied));
    // The denial is per scope: another scope still asks.
    assert!(matches!(
        b.decide(
            &call(Target::Intent("EditDraft"), json!({"text":"x"}), json!({})),
            11
        ),
        Outcome::Ask(_)
    ));
    // It lapses after a day, and a later grant clears it.
    assert!(matches!(b.decide(&r, DENIAL_SECS + 1), Outcome::Ask(_)));
}

#[test]
fn bad_calls_are_refused_before_any_prompt() {
    let mut b = broker();
    for (target, args) in [
        (Target::Intent("SelectThread"), json!({"thread":"a\u{0}b"})),
        (Target::Intent("SelectThread"), json!({})),
        (
            Target::Intent("EditDraft"),
            json!({"text":"x".repeat(5000)}),
        ),
        (Target::Intent("Send"), json!({"text":"smuggled"})),
    ] {
        assert!(matches!(
            b.decide(&call(target, args, json!({})), 0),
            Outcome::Refuse(Refusal::BadArguments(_))
        ));
    }
    for hidden in ["AllowOnce", "Deny", "ToggleRunner", "Nope"] {
        assert_eq!(
            b.decide(&call(Target::Intent(hidden), json!({}), json!({})), 0),
            Outcome::Refuse(Refusal::UnknownTarget)
        );
    }
    let unknown = Request {
        agent: AGENT,
        app_id: "io.example.Missing",
        target: Target::Intent("Send"),
        args: json!({}),
        shown: json!({}),
    };
    assert_eq!(b.decide(&unknown, 0), Outcome::Refuse(Refusal::UnknownApp));
    assert!(
        b.pending.is_empty(),
        "no prompt was created for a refused call"
    );
}

#[test]
fn entity_queries_are_read_scoped() {
    let mut b = broker();
    let r = call(Target::Query("Line"), json!({}), json!({}));
    let p = ask(&mut b, &r, 0);
    assert_eq!((p.scope.as_str(), p.risk), ("chat.history", Risk::Read));
    b.answer(p.id, Answer::Always, 0).unwrap();
    assert_eq!(b.decide(&r, 1), Outcome::Run(None));
    // Reading the thread list is a different scope and still asks.
    assert!(matches!(
        b.decide(&call(Target::Query("Thread"), json!({}), json!({})), 2),
        Outcome::Ask(_)
    ));
    assert_eq!(
        b.decide(&call(Target::Query("Ghost"), json!({}), json!({})), 3),
        Outcome::Refuse(Refusal::UnknownTarget)
    );
}

#[test]
fn grants_are_per_agent_and_revocation_is_immediate() {
    let mut b = broker();
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Always, 0).unwrap();
    let other = Request {
        agent: "agent:other",
        ..r.clone()
    };
    assert!(
        matches!(b.decide(&other, 1), Outcome::Ask(_)),
        "a grant belongs to one agent"
    );
    assert_eq!(b.grants(1).len(), 1);
    assert_eq!(b.revoke(AGENT, None), 1);
    assert!(matches!(b.decide(&r, 2), Outcome::Ask(_)));
}

#[test]
fn an_answer_only_works_for_a_prompt_the_broker_issued() {
    let mut b = broker();
    assert!(b.answer(99, Answer::Always, 0).is_err());
    let p = ask(
        &mut b,
        &call(Target::Intent("NewThread"), json!({}), json!({})),
        0,
    );
    b.answer(p.id, Answer::Once, 0).unwrap();
    assert!(
        b.answer(p.id, Answer::Always, 0).is_err(),
        "a prompt is answered once"
    );
}

#[test]
fn the_audit_trail_has_no_arguments_or_content() {
    let mut b = broker();
    let r = call(
        Target::Intent("EditDraft"),
        json!({"text":"my secret plan"}),
        json!({}),
    );
    let p = ask(&mut b, &r, 5);
    b.answer(p.id, Answer::Always, 5).unwrap();
    b.decide(&r, 6);
    let log = serde_json::to_string(b.audit()).unwrap();
    assert!(!log.contains("secret plan"), "{log}");
    assert_eq!(
        b.audit().iter().map(|e| e.outcome).collect::<Vec<_>>(),
        ["ask", "run"]
    );
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mseries-broker-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("grants.json")
}

#[test]
fn grants_survive_a_restart_and_the_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_path("persist");
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let mut b = Broker::open(&path);
    b.register(t3_manifest()).unwrap();
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Always, 0).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let mut again = Broker::open(&path);
    again.register(t3_manifest()).unwrap();
    assert_eq!(again.decide(&r, 1), Outcome::Run(None));
}

#[test]
fn a_damaged_or_loosely_permissioned_grants_file_means_no_grants() {
    use std::os::unix::fs::PermissionsExt;
    let r = call(Target::Intent("NewThread"), json!({}), json!({}));
    let path = temp_path("loose");
    let mut b = Broker::open(&path);
    b.register(t3_manifest()).unwrap();
    let p = ask(&mut b, &r, 0);
    b.answer(p.id, Answer::Always, 0).unwrap();

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    let mut loose = Broker::open(&path);
    loose.register(t3_manifest()).unwrap();
    assert!(matches!(loose.decide(&r, 1), Outcome::Ask(_)));

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut broken = Broker::open(&path);
    broken.register(t3_manifest()).unwrap();
    assert!(matches!(broken.decide(&r, 1), Outcome::Ask(_)));
}
