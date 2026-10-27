use msdk::usage::{Coverage, State};
use msdk_gtk::codexbar::{DecodeError, decode_snapshot};
use serde_json::{Value, json};

const NOW: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z
fn fixture() -> Value {
    json!({"schemaVersion":1,"stale":false,"busy":false,"error":"", "entries":[
        {"provider":"codex","updatedAt":"2026-01-01T00:00:00Z","error":"", "windows":[
            {"key":"week","label":"7 day","remaining":75.25,"resetsAt":"2026-01-02T00:00:00Z"},
            {"key":"hour","label":"5 hour","remaining":null,"resetsAt":""}]},
        {"provider":"claude","updatedAt":"","error":"unavailable", "windows":[]}
    ]})
}
fn decode(value: &Value, now: i64) -> Result<msdk::usage::Snapshot, DecodeError> {
    decode_snapshot(&serde_json::to_vec(value).unwrap(), now, 300)
}

#[test]
fn actual_shape_keeps_aggregate_scope_partial_coverage_and_unknown_values() {
    let snapshot = decode(&fixture(), NOW).unwrap();
    assert_eq!(snapshot.coverage, Coverage::Partial);
    assert_eq!(snapshot.observations.len(), 2);
    let codex = &snapshot.observations[0];
    assert_eq!(codex.state, State::Fresh);
    assert!(codex.scope.account.is_none());
    assert_eq!(
        codex.windows[0].quota.unwrap().remaining_basis_points(),
        7525
    );
    assert!(codex.windows[1].quota.is_none());
    assert_eq!(snapshot.observations[1].state, State::Unavailable);
}

#[test]
fn freshness_uses_source_time_and_expired_reset_is_not_current() {
    let snapshot = decode(&fixture(), NOW + 301).unwrap();
    assert_eq!(snapshot.observations[0].state, State::Stale);
    let snapshot = decode(&fixture(), NOW - 1).unwrap();
    assert_eq!(snapshot.observations[0].state, State::Unknown);
    let snapshot = decode_snapshot(
        &serde_json::to_vec(&fixture()).unwrap(),
        NOW + 86400,
        100000,
    )
    .unwrap();
    assert_eq!(
        snapshot.observations[0].windows[0]
            .quota
            .unwrap()
            .state_at(State::Fresh, NOW + 86400),
        State::Expired
    );
}

#[test]
fn malformed_or_future_contracts_are_rejected_without_payload_in_errors() {
    let mut value = fixture();
    value["schemaVersion"] = json!(2);
    assert!(matches!(
        decode(&value, NOW),
        Err(DecodeError::UnsupportedVersion)
    ));
    let mut value = fixture();
    value["entries"][0]["windows"][0]["remaining"] = json!(101);
    assert!(matches!(
        decode(&value, NOW),
        Err(DecodeError::InvalidField)
    ));
    let mut value = fixture();
    value["entries"][0]["updatedAt"] = json!("private-sensitive-invalid");
    let error = match decode(&value, NOW) {
        Err(error) => error,
        Ok(_) => panic!("invalid time was accepted"),
    };
    assert!(!format!("{error:?}").contains("private-sensitive-invalid"));
    let mut value = fixture();
    value["entries"][0]["windows"][0]["remaining"] = json!("0");
    assert!(matches!(
        decode(&value, NOW),
        Err(DecodeError::InvalidField)
    ));
    assert!(matches!(
        decode_snapshot(&vec![b' '; 2 * 1024 * 1024 + 1], NOW, 300),
        Err(DecodeError::TooLarge)
    ));
}

#[test]
fn failed_read_retains_stale_measurements_without_claiming_success() {
    let mut value = fixture();
    value["entries"][0]["error"] = json!("synthetic failure");
    let snapshot = decode(&value, NOW).unwrap();
    assert_eq!(snapshot.observations[0].state, State::Stale);
    assert_eq!(
        snapshot.observations[0].windows[0]
            .quota
            .unwrap()
            .remaining_basis_points(),
        7525
    );
}

#[cfg(unix)]
#[test]
fn bounded_reader_runs_a_synthetic_executable_and_expires_old_replies() {
    use msdk::context::RequestContext;
    use msdk_gtk::codexbar::{SnapshotReadError, read_snapshot_scoped};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("msdk-codexbar-test-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let executable = dir.join("fixture ' ; not-a-shell-command");
    let json = serde_json::to_string(&fixture()).unwrap();
    std::fs::write(
        &executable,
        format!("#!/bin/sh\n[ \"$1\" = --snapshot ] || exit 9\nprintf '%s' '{json}'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RequestContext::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let task = read_snapshot_scoped(
        context.token().unwrap(),
        executable.clone(),
        u64::MAX,
        71,
        move |reply| tx.send(reply).unwrap(),
    )
    .unwrap();
    let snapshot = rx
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .into_current()
        .unwrap()
        .unwrap();
    assert!(task.is_finished());
    assert_eq!(
        snapshot.observations[0].windows[0]
            .quota
            .unwrap()
            .remaining_basis_points(),
        7525
    );
    assert_eq!(snapshot.coverage, Coverage::Partial);

    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf 'synthetic-private-error' >&2\nprintf 'not-json'\n",
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let _task = read_snapshot_scoped(
        context.token().unwrap(),
        executable.clone(),
        300,
        72,
        move |reply| tx.send(reply).unwrap(),
    )
    .unwrap();
    let result = rx
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .into_current()
        .unwrap();
    match result {
        Err(SnapshotReadError::Decode(DecodeError::Malformed)) => (),
        Err(error) => panic!("unexpected error: {error:?}"),
        Ok(_) => panic!("malformed snapshot accepted"),
    }

    let marker = dir.join("started");
    let marker_string = marker.to_str().unwrap().replace('\'', "'\\''");
    std::fs::write(
        &executable,
        format!("#!/bin/sh\nprintf started > '{marker_string}'\nexec sleep 20\n"),
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let task = read_snapshot_scoped(
        context.token().unwrap(),
        executable,
        300,
        73,
        move |reply| tx.send(reply).unwrap(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(marker.exists(), "the owned test executable must start");
    context.advance().unwrap();
    assert!(
        rx.recv_timeout(Duration::from_secs(3))
            .unwrap()
            .into_current()
            .is_none()
    );
    assert!(task.is_finished());
}
