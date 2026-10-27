#![cfg(any(target_os = "linux", target_os = "macos"))]
use msdk::process::{self, Limits};
use msdk::runtime::FailureKind;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn run(command: &str, limits: Limits) -> Result<process::Output, msdk::runtime::ServiceFailure> {
    let (tx, rx) = mpsc::channel();
    let handle = process::spawn(command.into(), limits, 1, move |result| {
        tx.send(result).unwrap();
    })
    .unwrap();
    let result = rx.recv_timeout(Duration::from_secs(3)).expect("bounded process completion");
    assert!(handle.is_finished());
    result
}
fn limits() -> Limits {
    Limits { timeout: Duration::from_millis(500), output_bytes: 4096 }
}

#[test]
fn real_output_and_nonzero_exit_are_distinct() {
    let output = run("printf 'hello'; printf 'notice' >&2", limits()).unwrap();
    assert_eq!(output.stdout, "hello");
    assert_eq!(output.stderr, "notice");
    let failure = run("printf 'synthetic private detail' >&2; exit 7", limits()).unwrap_err();
    assert_eq!(failure.kind, FailureKind::ProcessFailed);
    assert_eq!(failure.detail(), Some("synthetic private detail"));
    assert!(!format!("{failure:?}").contains("synthetic private detail"));
}
#[test]
fn timeout_stops_a_real_long_running_process() {
    let start = Instant::now();
    let failure = run("sleep 20", limits()).unwrap_err();
    assert_eq!(failure.kind, FailureKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn stdout_and_stderr_share_one_hard_byte_limit() {
    let failure = run("printf '1234'; printf '5678' >&2", Limits { output_bytes: 7, ..limits() }).unwrap_err();
    assert_eq!(failure.kind, FailureKind::OutputLimit);
    let failure = run("while :; do printf 'flood'; done", limits()).unwrap_err();
    assert_eq!(failure.kind, FailureKind::OutputLimit);
}
#[test]
fn inherited_pipes_do_not_stall_after_the_leader_exits() {
    let start = Instant::now();
    let output = run("sleep 20 & printf 'done'; exit 0", limits()).unwrap();
    assert_eq!(output.stdout, "done");
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn dropping_an_owned_handle_cancels_instead_of_detaching_the_command() {
    let (tx, rx) = mpsc::channel();
    let handle = process::spawn("sleep 20".into(), Limits::default(), 8, move |result| {
        tx.send(result).unwrap();
    })
    .unwrap();
    drop(handle);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind, FailureKind::Cancelled);
}
#[test]
fn cancellation_reaps_the_leader_and_stops_its_process_group() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("msdk-process-{}-{stamp}.pid", std::process::id()));
    let quoted = path.to_string_lossy().replace('\'', "'\\''");
    let command = format!("sleep 20 & child=$!; printf '%s %s' \"$$\" \"$child\" > '{quoted}'; wait");
    let (tx, rx) = mpsc::channel();
    let handle = process::spawn(command, Limits::default(), 9, move |result| {
        tx.send(result).unwrap();
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let pids: Vec<i32> =
        std::fs::read_to_string(&path).unwrap().split_whitespace().map(|pid| pid.parse().unwrap()).collect();
    assert_eq!(pids.len(), 2);
    let pid = pids[0];
    handle.cancel();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind, FailureKind::Cancelled);
    // SAFETY: read-only existence probe for our recorded child, never a signal.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "leader must be reaped before completion");
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pids[1], 0) } == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(unsafe { libc::kill(pids[1], 0) }, -1, "same-group descendant must stop");
    std::fs::remove_file(path).unwrap();
}
#[test]
fn invalid_resource_limits_are_refused() {
    assert_eq!(run("bad\0argument", limits()).unwrap_err().kind, FailureKind::InvalidInput);

    assert_eq!(
        run("true", Limits { timeout: Duration::ZERO, ..limits() }).unwrap_err().kind,
        FailureKind::InvalidInput
    );
    assert_eq!(run("true", Limits { output_bytes: 0, ..limits() }).unwrap_err().kind, FailureKind::InvalidInput);
}
