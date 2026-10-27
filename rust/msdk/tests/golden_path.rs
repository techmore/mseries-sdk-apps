use msdk::logic::{Effect, Logic, RulesLogic, initial_store};
use serde_json::{Value, json};

#[test]
fn home_storage_recovers_and_prevents_duplicate_work() {
    let doc: Value = serde_json::from_str(include_str!("../examples/home-storage.ui.json")).unwrap();
    let errors: Vec<_> = msdk::check(&doc).into_iter().filter(|d| d.severity == msdk::Severity::Error).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let mut state = initial_store(&doc);
    let mut logic = RulesLogic::new(&doc);
    let first = logic.update(&mut state, "Refresh", None);
    let [Effect::Call { service, args, then, error }] = first.as_slice() else { panic!("{first:?}") };
    assert_eq!(service, "process.run");
    assert_eq!(then.as_deref(), Some("Loaded"));
    assert_eq!(error.as_deref(), Some("Failed"));
    assert_eq!(args["command"], doc["update"]["Refresh"][0]["do"][5]["args"]["command"]);
    assert_eq!(state["busy"], true);
    assert!(logic.update(&mut state, "Refresh", None).is_empty());
    logic.update(&mut state, "Failed", Some(json!("Synthetic permission denial")));
    assert_eq!(state["failed"], true);
    assert_eq!(state["busy"], false);
    let failed = msdk::inspect::outline(&msdk::inspect::render(&doc, "main", Some(&state)).unwrap());
    assert!(failed.contains("Synthetic permission denial") && failed.contains("Check again"));
    assert_eq!(logic.update(&mut state, "Refresh", None), first);
    assert_eq!(state["result"], "");
    logic.update(&mut state, "Loaded", Some(json!("Filesystem Size Used Avail Capacity\nsynthetic 100G 40G 60G 40%")));
    assert_eq!(state["failed"], false);
    assert_eq!(state["ready"], true);
    let recovered = msdk::inspect::outline(&msdk::inspect::render(&doc, "main", Some(&state)).unwrap());
    assert!(recovered.contains("60G") && !recovered.contains("Synthetic permission denial"));
    assert!(msdk::generate::models(&doc).is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn home_storage_actual_command_fails_then_recovers() {
    let doc: Value = serde_json::from_str(include_str!("../examples/home-storage.ui.json")).unwrap();
    let mut state = initial_store(&doc);
    let mut logic = RulesLogic::new(&doc);
    for missing_home in [true, false] {
        let effects = logic.update(&mut state, "Refresh", None);
        let [Effect::Call { args, then, error, .. }] = effects.as_slice() else { panic!("expected one task") };
        // Change only this child shell's HOME; never mutate the test process's
        // environment. The application command itself stays byte-for-byte exact.
        let command = format!("{}{}", if missing_home { "HOME=''; " } else { "" }, args["command"].as_str().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let task = msdk::process::spawn(command, msdk::process::Limits::default(), 1, move |result| {
            tx.send(result).unwrap();
        })
        .unwrap();
        let result = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        if missing_home {
            let failure = result.unwrap_err();
            assert_eq!(failure.kind, msdk::runtime::FailureKind::ProcessFailed);
            logic.update(&mut state, error.as_deref().unwrap(), Some(json!(failure.to_string())));
            assert_eq!(state["failed"], true);
        } else {
            let output = result.unwrap();
            // Assert useful output without copying a private home path or mount
            // identity into a public receipt or panic message.
            assert!(output.stdout.contains("Available:") && output.stdout.contains("Used space:"));
            assert_eq!(output.stdout.lines().count(), 4);
            logic.update(&mut state, then.as_deref().unwrap(), Some(json!(output.stdout)));
            assert_eq!(state["failed"], false);
        }
        assert!(task.is_finished());
        assert_eq!(state["ready"], true);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn unsupported_storage_report_is_an_error_instead_of_invented_usage() {
    let doc: Value = serde_json::from_str(include_str!("../examples/home-storage.ui.json")).unwrap();
    let command = doc["update"]["Refresh"][0]["do"][5]["args"]["command"].as_str().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    // Child-local function simulates an unsupported report, with no PATH or
    // global environment mutation and no real filesystem side effects.
    let task = msdk::process::spawn(
        format!("df() {{ printf '%s\\n' header unsupported; }}\n{command}"),
        msdk::process::Limits::default(),
        1,
        move |result| {
            tx.send(result).unwrap();
        },
    )
    .unwrap();
    let failure = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().unwrap_err();
    assert_eq!(failure.kind, msdk::runtime::FailureKind::ProcessFailed);
    assert!(failure.detail().unwrap().contains("Storage report format is unavailable"));
    assert!(task.is_finished());
}
