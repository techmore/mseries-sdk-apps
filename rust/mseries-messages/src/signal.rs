//! The Signal channel: Signal Pantheon's conversations as Messages threads.
//!
//! Signal Pantheon keeps each conversation as one encrypted snapshot in
//! `~/.local/share/signal-pantheon/message-history.db`. This module reads those
//! snapshots and presents them as Messages threads, so one conversation list
//! can show iMessage/SMS and Signal side by side. Sending goes through
//! signal-cli (`service`, `rpc`); history is written only under the
//! single-writer rule (`owner`, `write`): never while the client runs.
//!
//! Store format (see `rust/mseries-signal-import`, which can also write it):
//! * the 32-byte key is base64 text in the login keyring
//!   (`purpose=encrypted-message-history`);
//! * `conversation_hash` = HMAC-SHA256(key, key string);
//! * a row is AES-256-GCM with its own 12-byte nonce, no AAD, and a 16-byte
//!   tag stored apart from the ciphertext;
//! * a conversation is the snapshot row (`timestamp = 0`, `outgoing = 0`) under
//!   `pantheon-v1:<account>:conversation:<id>`, and the list of ids is the
//!   snapshot under `pantheon-v1:<account>:catalog`.

use mseries_pantheon_kit::{Message, Thread};
pub use mseries_signal_history::HistoryKey;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

pub mod live;
pub mod owner;
pub mod rpc;
pub mod service;
pub mod write;

/// Prefix that marks a thread as coming from Signal.
pub const THREAD_PREFIX: &str = "signal:";

/// How long the keyring may take. A locked or missing keyring service can leave
/// `secret-tool` waiting indefinitely, and nothing that waits on it may hang.
const KEYRING_TIMEOUT: Duration = Duration::from_secs(10);

/// The login-keyring lookup, through `secret-tool`, so no secret touches disk.
pub fn load_history_key() -> Result<Option<HistoryKey>, String> {
    load_history_key_with("secret-tool", KEYRING_TIMEOUT)
}

fn load_history_key_with(program: &str, timeout: Duration) -> Result<Option<HistoryKey>, String> {
    use std::io::Read;
    let mut child = Command::new(program)
        .args(["lookup", "purpose", "encrypted-message-history"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not run secret-tool: {error}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut secret = Vec::new();
        let _ = stdout.read_to_end(&mut secret);
        secret
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                // Only the process started here is stopped.
                let _ = child.kill();
                let _ = child.wait();
                return Err("the login keyring did not answer in time".into());
            }
        }
    };
    let secret = reader.join().unwrap_or_default();
    if !status.success() || secret.is_empty() {
        return Ok(None);
    }
    HistoryKey::from_keyring_secret(&secret).map(Some)
}

pub fn catalog_key(account: &str) -> String {
    format!("pantheon-v1:{account}:catalog")
}

pub fn conversation_key(account: &str, id: &str) -> String {
    format!("pantheon-v1:{account}:conversation:{id}")
}

/// The read-only encrypted store.
pub struct SignalStore {
    connection: Connection,
    key: HistoryKey,
}

impl SignalStore {
    pub fn open(path: &Path, key: HistoryKey) -> Result<Self, String> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("could not open the Signal history: {error}"))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        Ok(Self { connection, key })
    }

    /// The decrypted snapshot stored under a key string, if there is one.
    pub fn read_snapshot(&self, key_string: &str) -> Result<Option<String>, String> {
        let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT nonce, ciphertext, tag FROM message_history \
                 WHERE conversation_hash = ? AND timestamp = 0 AND outgoing = 0 \
                 ORDER BY id DESC LIMIT 1",
                [self.key.hash(key_string)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("could not read the Signal history: {error}"))?;
        match row {
            None => Ok(None),
            Some((nonce, ciphertext, tag)) => {
                let plain = self.key.decrypt(&nonce, &ciphertext, &tag)?;
                String::from_utf8(plain)
                    .map(Some)
                    .map_err(|_| "a Signal snapshot is not text".to_string())
            }
        }
    }

    /// Ids of the conversations the client has saved for an account.
    pub fn conversation_ids(&self, account: &str) -> Result<Vec<String>, String> {
        let Some(catalog) = self.read_snapshot(&catalog_key(account))? else {
            return Ok(Vec::new());
        };
        let value: Value = serde_json::from_str(&catalog)
            .map_err(|_| "the Signal conversation list is not valid".to_string())?;
        Ok(value
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn conversation(&self, account: &str, id: &str) -> Result<Option<Thread>, String> {
        let Some(snapshot) = self.read_snapshot(&conversation_key(account, id))? else {
            return Ok(None);
        };
        // Messages Signal accepted while the client owned the file are shown too.
        let snapshot = write::PENDING.overlay(account, id, &snapshot);
        thread_from_snapshot(account, &snapshot)
    }
}

/// `2026-10-27T09:01:00Z` from Unix milliseconds, the form the timeline expects.
fn iso8601(milliseconds: i64) -> String {
    gtk::glib::DateTime::from_unix_utc(milliseconds.div_euclid(1000))
        .ok()
        .and_then(|moment| moment.format_iso8601().ok())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

/// One saved conversation as a read-only Messages thread.
pub fn thread_from_snapshot(account: &str, snapshot: &str) -> Result<Option<Thread>, String> {
    let root: Value = serde_json::from_str(snapshot)
        .map_err(|_| "a Signal conversation is not valid".to_string())?;
    let Some(id) = root["id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let name = root["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or(id);
    let is_group = root["group"].as_bool().unwrap_or(false);
    let unread = root["unread"].as_u64().unwrap_or(0) as usize;

    let mut messages = Vec::new();
    for record in root["messages"].as_array().into_iter().flatten() {
        let outgoing = record["outgoing"].as_bool().unwrap_or(false);
        let deleted = record["deleted"].as_bool().unwrap_or(false);
        let view_once = record["viewOnce"].as_bool().unwrap_or(false);
        let timestamp = record["timestamp"].as_i64().unwrap_or(0);
        let attachments = record["attachments"]
            .as_array()
            .map(|list| list.len())
            .unwrap_or(0);
        let mut body = if deleted {
            "This message was deleted".to_string()
        } else if view_once {
            "View-once media".to_string()
        } else {
            record["text"].as_str().unwrap_or("").to_string()
        };
        if body.is_empty() && attachments > 0 && !deleted {
            body = if attachments == 1 {
                "Attachment".to_string()
            } else {
                format!("{attachments} attachments")
            };
        }
        let author = record["author"].as_str().unwrap_or("");
        let status = record["status"].as_str().unwrap_or("sent");
        messages.push(Message {
            handle: format!("{id}:{timestamp}:{author}"),
            body,
            timestamp: iso8601(timestamp),
            outgoing,
            // A message is unread until the client marked it read here.
            read: outgoing || record["localRead"].as_bool().unwrap_or(true) || status == "read",
            sender: if is_group && !outgoing {
                author.to_string()
            } else {
                String::new()
            },
            ..Message::default()
        });
    }
    messages.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    let last_ts = messages
        .last()
        .map(|message| message.timestamp.clone())
        .unwrap_or_default();
    let mut thread = Thread {
        key: format!("{THREAD_PREFIX}{account}:{id}"),
        name: name.to_string(),
        is_group,
        recipients: Vec::new(),
        reply_ready: true,
        messages,
        last_ts,
        unread_count: unread,
        local_only: true,
        search_index: Arc::new(String::new()),
        ..Thread::default()
    };
    thread.refresh_search_index();
    Ok(Some(thread))
}

/// Where the client keeps its data.
pub fn data_directory() -> PathBuf {
    gtk::glib::user_data_dir().join("signal-pantheon")
}

/// The linked account's number from the client's `accounts.json`, if any.
pub fn active_account(directory: &Path) -> Option<String> {
    let text = std::fs::read_to_string(directory.join("data").join("accounts.json")).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    root["accounts"]
        .as_array()?
        .iter()
        .find_map(|account| account["number"].as_str().map(str::to_string))
}

/// Everything Signal contributes to the conversation list: all saved
/// conversations of the linked account, or nothing when Signal Pantheon is not
/// set up on this machine.
pub fn load_threads() -> Result<Vec<Thread>, String> {
    let directory = data_directory();
    let database = directory.join("message-history.db");
    if !database.exists() {
        return Ok(Vec::new());
    }
    let Some(account) = active_account(&directory) else {
        return Ok(Vec::new());
    };
    let Some(key) = load_history_key()? else {
        return Ok(Vec::new());
    };
    // Anything sent while the client was running is written now if it has quit.
    if !write::PENDING.is_empty() {
        let _ = write::flush_pending(&directory, &account, key.clone(), &write::PENDING);
    }
    let store = SignalStore::open(&database, key)?;
    let mut threads = Vec::new();
    for id in store.conversation_ids(&account)? {
        if let Some(thread) = store.conversation(&account, &id)? {
            threads.push(thread);
        }
    }
    Ok(threads)
}

/// `signal:<account>:<conversation>` back into its parts.
pub fn parse_thread_key(key: &str) -> Option<(&str, &str)> {
    key.strip_prefix(THREAD_PREFIX)?
        .split_once(':')
        .filter(|(account, id)| !account.is_empty() && !id.is_empty())
}

/// Why a Signal send did not go through.
#[derive(Clone, Debug)]
pub struct SendFailure {
    pub message: String,
    /// The request left this app but no answer came: Signal may have it.
    pub outcome_unknown: bool,
}

/// Send `text` to a Signal conversation and record it in the history.
///
/// Blocking: call it off the main thread. Once Signal has accepted the message
/// this never reports failure, whatever happens to the local history copy; the
/// result says whether the copy was written or is waiting for the client to quit.
pub fn send_text(
    thread_key: &str,
    is_group: bool,
    text: &str,
) -> Result<write::Saved, SendFailure> {
    let (account, id) = parse_thread_key(thread_key).ok_or_else(|| SendFailure {
        message: "This is not a Signal conversation.".into(),
        outcome_unknown: false,
    })?;
    let layout = service::Layout::detect();
    let mut connection = service::connect(&layout).map_err(|error| SendFailure {
        message: error.message().to_string(),
        outcome_unknown: false,
    })?;
    let result = connection
        .call(
            "send",
            rpc::send_params(account, id, is_group, text),
            SEND_TIMEOUT,
        )
        .map_err(|error| SendFailure {
            outcome_unknown: matches!(error, rpc::RpcError::OutcomeUnknown(_)),
            message: error.message().to_string(),
        })?;
    let timestamp = rpc::confirm_send(&result).map_err(|message| SendFailure {
        message,
        outcome_unknown: true,
    })?;
    let expiration_seconds = result["expirationByRecipient"][id].as_i64().unwrap_or(0);
    write::PENDING.push(
        account,
        id,
        write::SentRecord {
            text: text.to_string(),
            author: account.to_string(),
            timestamp,
            expiration_seconds,
            expiration_start_ms: result["expirationStartTimestamp"]
                .as_i64()
                .unwrap_or(timestamp),
        },
    );
    let saved = match load_history_key() {
        Ok(Some(key)) => {
            write::flush_pending(&layout.data_directory, account, key, &write::PENDING)
                .unwrap_or(write::Saved::Deferred)
        }
        _ => write::Saved::Deferred,
    };
    Ok(saved)
}

const SEND_TIMEOUT: Duration = Duration::from_secs(45);

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent vectors computed with Python's `hmac` and `cryptography`.
    const KEY: [u8; 32] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hash_matches_an_independent_hmac() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        assert_eq!(
            hex(&key.hash("pantheon-v1:+15550000001:catalog")),
            "dc92c17cc98f44e5cb791afee73851033ab45f141ef123a1ceb380ae8f77c094"
        );
    }

    #[test]
    fn decrypts_an_independently_encrypted_row() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let nonce: Vec<u8> = (100..112).collect();
        let plain = key
            .decrypt(
                &nonce,
                &unhex("1339f5574cdc63ae0e526fd8ea5748a0"),
                &unhex("c03967b6e45742ce433662a7724a7d42"),
            )
            .unwrap();
        assert_eq!(plain, br#"["+15550000002"]"#);
    }

    #[test]
    fn a_tampered_row_is_refused() {
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let nonce: Vec<u8> = (100..112).collect();
        let mut ciphertext = unhex("1339f5574cdc63ae0e526fd8ea5748a0");
        ciphertext[0] ^= 1;
        assert!(
            key.decrypt(
                &nonce,
                &ciphertext,
                &unhex("c03967b6e45742ce433662a7724a7d42")
            )
            .is_err()
        );
    }

    #[test]
    fn a_keyring_that_never_answers_cannot_hang_the_caller() {
        let directory = std::env::temp_dir().join(format!("signal-keyring-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("secret-tool");
        std::fs::write(&script, "#!/bin/sh\nsleep 60\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let result = load_history_key_with(script.to_str().unwrap(), Duration::from_millis(300));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(&directory).ok();
    }

    fn seal(key: &HistoryKey, nonce: [u8; 12], plain: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let sealed = key.encrypt_with_nonce(&nonce, plain).unwrap();
        (sealed.ciphertext, sealed.tag)
    }

    #[test]
    fn reads_a_conversation_from_an_encrypted_store() {
        let directory = std::env::temp_dir().join(format!("signal-store-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("message-history.db");
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let account = "+15550000001";
        let conversation = r#"{"version":1,"id":"+15550000002","name":"Alice","group":false,
            "unread":1,"messages":[
              {"text":"later","author":"+15550000002","timestamp":1790000100000,"outgoing":false,"localRead":false,"status":"sent","attachments":[]},
              {"text":"hello","author":"+15550000001","timestamp":1790000000000,"outgoing":true,"status":"delivered","attachments":[]},
              {"text":"","author":"+15550000002","timestamp":1790000200000,"outgoing":false,"localRead":true,"attachments":[{"filename":"a.png"}]}
            ]}"#;
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE message_history (id INTEGER PRIMARY KEY, \
                     conversation_hash BLOB NOT NULL, timestamp INTEGER NOT NULL, \
                     outgoing INTEGER NOT NULL, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, \
                     tag BLOB NOT NULL)",
                )
                .unwrap();
            for (index, (key_string, plain)) in [
                (catalog_key(account), r#"["+15550000002"]"#.to_string()),
                (
                    conversation_key(account, "+15550000002"),
                    conversation.to_string(),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let nonce = [index as u8 + 1; 12];
                let (ciphertext, tag) = seal(&key, nonce, plain.as_bytes());
                connection
                    .execute(
                        "INSERT INTO message_history(conversation_hash, timestamp, outgoing, nonce, ciphertext, tag) VALUES (?,0,0,?,?,?)",
                        rusqlite::params![key.hash(&key_string), nonce.to_vec(), ciphertext, tag],
                    )
                    .unwrap();
            }
        }
        let store = SignalStore::open(&path, HistoryKey::from_raw(&KEY).unwrap()).unwrap();
        assert_eq!(store.conversation_ids(account).unwrap(), ["+15550000002"]);
        assert!(
            store
                .conversation(account, "+19999999999")
                .unwrap()
                .is_none()
        );
        let thread = store
            .conversation(account, "+15550000002")
            .unwrap()
            .expect("saved conversation");
        assert_eq!(thread.key, "signal:+15550000001:+15550000002");
        assert_eq!(thread.name, "Alice");
        assert!(thread.reply_ready);
        assert_eq!(thread.unread_count, 1);
        // Sorted by time, whatever order the snapshot used.
        let bodies: Vec<_> = thread.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["hello", "later", "Attachment"]);
        assert!(thread.messages[0].outgoing);
        assert!(!thread.messages[1].read);
        assert!(thread.messages[2].read);
        assert_eq!(thread.last_ts, thread.messages[2].timestamp);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn deleted_and_view_once_messages_never_show_their_content() {
        let snapshot = r#"{"id":"g1","name":"Team","group":true,"messages":[
            {"text":"secret","author":"a","timestamp":1790000000000,"deleted":true},
            {"text":"","author":"b","timestamp":1790000001000,"viewOnce":true,"attachments":[{"filename":"x"}]}]}"#;
        let thread = thread_from_snapshot("+1", snapshot).unwrap().unwrap();
        assert_eq!(thread.messages[0].body, "This message was deleted");
        assert_eq!(thread.messages[1].body, "View-once media");
        assert!(thread.is_group);
        assert_eq!(thread.messages[1].sender, "b");
    }

    #[test]
    fn malformed_snapshots_are_errors_not_panics() {
        assert!(thread_from_snapshot("+1", "not json").is_err());
        assert!(
            thread_from_snapshot("+1", r#"{"name":"no id"}"#)
                .unwrap()
                .is_none()
        );
    }

    /// Counts only, never content: proves the read path against this machine's real store.
    #[test]
    #[ignore = "reads the real Signal Pantheon store and login keyring"]
    fn reads_the_local_signal_store() {
        let threads = load_threads().expect("readable store");
        let messages: usize = threads.iter().map(|thread| thread.messages.len()).sum();
        eprintln!("signal threads: {}, messages: {messages}", threads.len());
        assert!(
            threads
                .iter()
                .all(|thread| thread.key.starts_with(THREAD_PREFIX))
        );
    }
}
