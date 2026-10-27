#![cfg(any(target_os = "linux", target_os = "macos"))]
use msdk::process::{self, Limits, Stream};
use msdk::runtime::FailureKind;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn private_partial_output_arrives_before_completion_and_survives_cancel() {
    let (tx, done) = mpsc::channel();
    let (task, output) = process::spawn_streaming(
        "printf 'synthetic-private-progress'; printf 'notice' >&2; sleep 20".into(),
        Limits::default(),
        21,
        move |result| {
            tx.send(result).unwrap();
        },
    )
    .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while stdout.is_empty() || stderr.is_empty() {
        let chunk = output.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!format!("{chunk:?}").contains("synthetic-private-progress"));
        match chunk.stream {
            Stream::Stdout => stdout.extend(chunk.bytes),
            Stream::Stderr => stderr.extend(chunk.bytes),
        }
    }
    assert!(!task.is_finished());
    assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
    task.cancel();
    let failure = done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err();
    assert_eq!(failure.kind, FailureKind::Cancelled);
    assert!(!failure.diagnostic().to_string().contains("synthetic-private-progress"));
    assert_eq!(stdout, b"synthetic-private-progress");
    assert_eq!(stderr, b"notice");
}

#[test]
fn raw_bytes_remain_exact_including_non_utf8_output() {
    let (tx, done) = mpsc::channel();
    let (_task, output) =
        process::spawn_streaming("printf '\\377\\342\\202\\254'".into(), Limits::default(), 22, move |result| {
            tx.send(result).unwrap();
        })
        .unwrap();
    done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    let mut bytes = Vec::new();
    while let Ok(chunk) = output.try_recv() {
        assert_eq!(chunk.stream, Stream::Stdout);
        bytes.extend(chunk.bytes);
    }
    assert_eq!(bytes, [255, 226, 130, 172]);
}

#[test]
fn slow_consumer_has_a_hard_queue_bound_and_a_terminal_failure() {
    let (tx, done) = mpsc::channel();
    let (task, output) =
        process::spawn_streaming("head -c 524288 /dev/zero".into(), Limits::default(), 23, move |result| {
            tx.send(result).unwrap();
        })
        .unwrap();
    // Intentionally do not drain: the reader must never block behind its client.
    assert_eq!(done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind, FailureKind::OutputLimit);
    assert!(task.is_finished());
    let mut chunks = 0;
    let mut bytes = 0;
    while let Ok(chunk) = output.try_recv() {
        chunks += 1;
        bytes += chunk.bytes.len();
    }
    assert_eq!(chunks, process::STREAM_QUEUE_CHUNKS);
    assert!(bytes > 0 && bytes <= process::STREAM_QUEUE_CHUNKS * process::STREAM_CHUNK_BYTES);
}

#[test]
fn dropping_subscription_cancels_a_silent_command() {
    let (tx, done) = mpsc::channel();
    let (task, output) =
        process::spawn_streaming("printf ready; sleep 20".into(), Limits::default(), 24, move |result| {
            tx.send(result).unwrap();
        })
        .unwrap();
    output.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(output);
    assert_eq!(done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind, FailureKind::Cancelled);
    assert!(task.is_finished());
}

#[test]
fn timeout_keeps_already_queued_partial_output() {
    let (tx, done) = mpsc::channel();
    let (_task, output) = process::spawn_streaming(
        "printf partial; sleep 20".into(),
        Limits { timeout: Duration::from_millis(150), ..Limits::default() },
        25,
        move |result| {
            tx.send(result).unwrap();
        },
    )
    .unwrap();
    assert_eq!(done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap_err().kind, FailureKind::TimedOut);
    assert_eq!(output.recv_timeout(Duration::from_secs(1)).unwrap().bytes, b"partial");
    assert!(matches!(output.try_recv(), Err(mpsc::TryRecvError::Disconnected)));
}
