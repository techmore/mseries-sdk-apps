//! Live Signal updates: listen to the `receive` notifications of the daemon
//! that owns the account and tell the app when its history may have changed.
//!
//! Only the daemon's own client writes history. While the Signal Pantheon
//! client runs it applies every event (new messages, edits, deletes,
//! reactions, receipts) to its conversations and saves them; this listener
//! just says "something changed", and the app re-reads the saved snapshots a
//! moment later. That keeps one writer and one set of event semantics (see
//! `mseries/signal-pantheon/src/message-event.vala`). Typing indicators and
//! call signalling do not change history and are ignored.

use super::rpc::event_affects_history;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const RETRY: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(500);

/// Background reader of one or more daemon sockets; stops when dropped.
pub struct Listener {
    stop: Arc<AtomicBool>,
    pub events: Receiver<()>,
}

impl Listener {
    /// `sockets` are tried in order on every (re)connect; the first that
    /// accepts is listened to.
    pub fn start(sockets: impl Fn() -> Vec<PathBuf> + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, events) = mpsc::channel();
        let flag = stop.clone();
        std::thread::spawn(move || run(&flag, &sockets, &sender));
        Self { stop, events }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(stop: &AtomicBool, sockets: &dyn Fn() -> Vec<PathBuf>, sender: &Sender<()>) {
    while !stop.load(Ordering::Relaxed) {
        let stream = sockets()
            .iter()
            .find_map(|socket| UnixStream::connect(socket).ok());
        if let Some(stream) = stream {
            // A short read timeout lets the loop notice `stop` without a wake-up.
            let _ = stream.set_read_timeout(Some(POLL));
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while !stop.load(Ordering::Relaxed) {
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if serde_json::from_str::<Value>(line.trim())
                            .is_ok_and(|event| event_affects_history(&event))
                            && sender.send(()).is_err()
                        {
                            return;
                        }
                        line.clear();
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        }
        // Sleep in small steps so dropping the listener is prompt.
        let mut waited = Duration::ZERO;
        while waited < RETRY && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(POLL);
            waited += POLL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn history_events_wake_the_app_and_typing_does_not() {
        let directory = std::env::temp_dir().join(format!("signal-live-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("daemon.sock");
        let server = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let path = socket.clone();
        let listener = Listener::start(move || vec![path.clone()]);
        let (mut stream, _) = server.accept().unwrap();
        writeln!(stream, r#"{{"method":"receive","params":{{"envelope":{{"typingMessage":{{"action":"STARTED"}}}}}}}}"#).unwrap();
        writeln!(stream, r#"{{"method":"callEvent","params":{{}}}}"#).unwrap();
        assert!(
            listener
                .events
                .recv_timeout(Duration::from_millis(900))
                .is_err()
        );
        writeln!(
            stream,
            r#"{{"method":"receive","params":{{"envelope":{{"dataMessage":{{"message":"hi"}}}}}}}}"#
        )
        .unwrap();
        assert!(listener.events.recv_timeout(Duration::from_secs(5)).is_ok());
        drop(listener);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn it_reconnects_after_the_daemon_restarts() {
        let directory = std::env::temp_dir().join(format!("signal-live-re-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let socket = directory.join("daemon.sock");
        let path = socket.clone();
        let listener = Listener::start(move || vec![path.clone()]);
        std::thread::sleep(Duration::from_millis(700)); // first attempts find nothing
        let server = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let (mut stream, _) = server.accept().unwrap();
        writeln!(stream, r#"{{"method":"receive","params":{{"envelope":{{"receiptMessage":{{"isRead":true}}}}}}}}"#).unwrap();
        assert!(listener.events.recv_timeout(Duration::from_secs(5)).is_ok());
        drop(listener);
        std::fs::remove_dir_all(&directory).ok();
    }
}
