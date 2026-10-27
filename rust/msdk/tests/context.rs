use msdk::context::RequestContext;

#[test]
fn generations_discard_old_success_and_error_values_at_delivery() {
    let context = RequestContext::new();
    let original = context.token().unwrap();
    let success = original.wrap(Ok::<_, String>("synthetic-private-reply"));
    let error = original.wrap(Err::<String, _>("synthetic-private-error"));
    let other = RequestContext::new();
    let independent = other.token().unwrap().wrap("independent");
    let next = context.advance().unwrap();
    assert!(!original.is_current());
    assert!(success.into_current().is_none());
    assert!(error.into_current().is_none());
    assert_eq!(independent.into_current(), Some("independent"));
    assert_eq!(next.wrap("current").into_current(), Some("current"));
    context.advance().unwrap();
    assert!(!next.is_current());
    assert!(!original.is_current(), "returning to an account must not revive its old requests");
}

#[test]
fn last_owner_drop_and_explicit_close_retire_tokens_without_retaining_the_owner() {
    let context = RequestContext::new();
    let owner = context.clone();
    let token = context.token().unwrap();
    let queued = token.wrap("private");
    drop(context);
    assert!(token.is_current());
    drop(owner);
    assert!(!token.is_current());
    assert!(queued.into_current().is_none());
    let closed = RequestContext::new();
    let token = closed.token().unwrap();
    closed.close();
    closed.close();
    assert!(!token.is_current());
    assert!(closed.token().is_none() && closed.advance().is_none());
}

#[test]
fn replies_crossing_a_queue_are_checked_after_the_context_switch() {
    let context = RequestContext::new();
    let token = context.token().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(token.wrap("synthetic-private-payload")).unwrap()).join().unwrap();
    // The worker already finished. Cancellation at the worker alone would be
    // too late: the queued result must carry freshness through final delivery.
    context.advance().unwrap();
    let queued = rx.recv().unwrap();
    assert!(!format!("{queued:?}").contains("synthetic-private-payload"));
    assert!(queued.into_current().is_none());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_request_in_an_already_closed_context_never_starts_its_command() {
    let context = RequestContext::new();
    let token = context.token().unwrap();
    context.close();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let marker = std::env::temp_dir().join(format!("msdk-closed-context-{}-{stamp}", std::process::id()));
    let quote = marker.to_string_lossy().replace('\'', "'\\''");
    let (tx, rx) = std::sync::mpsc::channel();
    let task = msdk::process::spawn_scoped(
        token,
        format!("printf unexpected > '{quote}'"),
        msdk::process::Limits::default(),
        30,
        move |reply| {
            tx.send(reply).unwrap();
        },
    )
    .unwrap();
    assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap().into_current().is_none());
    assert!(task.is_finished());
    let started = marker.exists();
    let _ = std::fs::remove_file(marker);
    assert!(!started, "obsolete task must not execute");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn switching_context_cancels_and_reaps_real_work_then_allows_a_new_request() {
    use msdk::process::{self, Limits};
    use std::time::{Duration, Instant};
    let context = RequestContext::new();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("msdk-context-{}-{stamp}.pid", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    let quote = path.to_string_lossy().replace('\'', "'\\''");
    let command = format!("printf '%s' \"$$\" > '{quote}'; exec sleep 20");
    let (tx, rx) = std::sync::mpsc::channel();
    let task = process::spawn_scoped(context.token().unwrap(), command, Limits::default(), 31, move |reply| {
        tx.send(reply).unwrap();
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let pid: i32 = loop {
        if let Some(pid) = std::fs::read_to_string(&path).ok().and_then(|text| text.parse().ok()) {
            break pid;
        }
        assert!(Instant::now() < deadline, "owned process did not start");
        std::thread::sleep(Duration::from_millis(2));
    };
    let next = context.advance().unwrap();
    let obsolete = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(obsolete.into_current().is_none());
    assert!(task.is_finished());
    // SAFETY: existence probe for this test's recorded process, never a signal.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "context switch must stop and reap the task");
    let (tx, rx) = std::sync::mpsc::channel();
    let current = process::spawn_scoped(next, "printf recovered".into(), Limits::default(), 32, move |reply| {
        tx.send(reply).unwrap();
    })
    .unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().into_current().unwrap().unwrap().stdout, "recovered");
    assert!(current.is_finished());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn scoped_stream_rejects_queued_chunks_and_cancels_on_context_switch() {
    use msdk::process::{self, Limits};
    use std::time::Duration;
    let context = RequestContext::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let (task, output) = process::spawn_streaming_scoped(
        context.token().unwrap(),
        "printf old; sleep 20".into(),
        Limits::default(),
        40,
        move |result| tx.send(result).unwrap(),
    )
    .unwrap();
    let queued = output.recv_timeout(Duration::from_secs(2)).unwrap();
    let next = context.advance().unwrap();
    assert!(queued.into_current().is_none(), "a chunk already in a UI queue must expire");
    assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().into_current().is_none());
    assert!(task.is_finished());

    let (tx, rx) = std::sync::mpsc::channel();
    let (_task, output) = process::spawn_streaming_scoped(next, "printf new".into(), Limits::default(), 41, move |r| {
        tx.send(r).unwrap();
    })
    .unwrap();
    assert_eq!(output.recv_timeout(Duration::from_secs(2)).unwrap().into_current().unwrap().bytes, b"new");
    assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().into_current().unwrap().is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn scoped_stream_keeps_unread_bytes_and_completion_scoped_after_exit() {
    use msdk::process::{self, Limits};
    use std::time::Duration;
    let context = RequestContext::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let (task, output) = process::spawn_streaming_scoped(
        context.token().unwrap(),
        "printf queued".into(),
        Limits::default(),
        42,
        move |result| tx.send(result).unwrap(),
    )
    .unwrap();
    let completion = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(task.is_finished());
    context.close();
    assert!(completion.into_current().is_none());
    assert!(output.recv_timeout(Duration::from_secs(2)).unwrap().into_current().is_none());
    assert!(matches!(output.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn direct_program_uses_literal_arguments_and_scoped_transform_retains_freshness() {
    use msdk::process::{self, Limits};
    use std::time::Duration;
    let context = RequestContext::new();
    let token = context.token().unwrap();
    assert!(
        process::spawn_program_scoped(token.clone(), "relative".into(), vec![], Limits::default(), 51, |_| {}).is_err()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let task = process::spawn_program_scoped(
        token,
        "/usr/bin/printf".into(),
        vec!["%s".into(), "$(literal);'not-shell'".into()],
        Limits::default(),
        52,
        move |reply| tx.send(reply.map(|result| result.unwrap().stdout)).unwrap(),
    )
    .unwrap();
    let reply = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(task.is_finished());
    assert_eq!(reply.into_current().as_deref(), Some("$(literal);'not-shell'"));
    let old = context.token().unwrap().wrap(3).map(|n| n * 2);
    context.advance().unwrap();
    assert!(old.into_current().is_none());
}
