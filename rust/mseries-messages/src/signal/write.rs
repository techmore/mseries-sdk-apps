//! Writing sent messages into the Signal Pantheon history.
//!
//! Snapshots are written in exactly the client's format (see the module docs of
//! `signal.rs`; the importer's `replace_snapshots` is the model): per key
//! string the old row is deleted and a new one inserted in one transaction,
//! with a fresh random nonce.
//!
//! This is only ever done under the single-writer rule of `owner.rs`: not while
//! the Pantheon client runs, and only with the writer lock held. While the
//! client runs, sent messages wait in [`Pending`] and are merged into what is
//! shown; they are written the next time the client is not running.

use super::owner::{self, WriterLock};
use super::{HistoryKey, SignalStore, conversation_key};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// A message that Signal accepted, as the history records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentRecord {
    pub text: String,
    pub author: String,
    pub timestamp: i64,
    pub expiration_seconds: i64,
    pub expiration_start_ms: i64,
}

impl SentRecord {
    fn to_json(&self) -> Value {
        json!({
            "text": self.text, "author": self.author, "timestamp": self.timestamp,
            "modified": 0, "outgoing": true, "deleted": false, "viewOnce": false,
            "viewOpened": false, "localRead": true, "readAt": 0,
            "expirationSeconds": self.expiration_seconds,
            "expirationStart": if self.expiration_seconds > 0 { self.expiration_start_ms } else { 0 },
            "status": "sent", "attachments": [], "reactions": {}, "reactionVersions": {}
        })
    }
}

/// `snapshot` with `record` appended, or `None` when it is already there
/// (matched on timestamp and direction, as the importer does).
pub fn append_sent(snapshot: &str, record: &SentRecord) -> Result<Option<String>, String> {
    let mut root: Value = serde_json::from_str(snapshot)
        .map_err(|_| "a Signal conversation is not valid".to_string())?;
    let Some(object) = root.as_object_mut() else {
        return Err("a Signal conversation is not valid".into());
    };
    let messages = object
        .entry("messages")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| "a Signal conversation is not valid".to_string())?;
    let exists = messages.iter().any(|message| {
        message["timestamp"].as_i64() == Some(record.timestamp)
            && message["outgoing"].as_bool() == Some(true)
    });
    if exists {
        return Ok(None);
    }
    messages.push(record.to_json());
    // The client clears the composer draft once a message is accepted.
    object.insert("draft".into(), json!(""));
    serde_json::to_string(&root)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Messages Signal accepted that are not in the history file yet.
#[derive(Default)]
pub struct Pending {
    by_conversation: Mutex<HashMap<(String, String), Vec<SentRecord>>>,
}

impl Pending {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Vec<SentRecord>>> {
        self.by_conversation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn push(&self, account: &str, id: &str, record: SentRecord) {
        self.lock()
            .entry((account.to_string(), id.to_string()))
            .or_default()
            .push(record);
    }

    pub fn is_empty(&self) -> bool {
        self.lock().values().all(Vec::is_empty)
    }

    /// `snapshot` with this conversation's waiting messages merged in.
    pub fn overlay(&self, account: &str, id: &str, snapshot: &str) -> String {
        let waiting = self
            .lock()
            .get(&(account.to_string(), id.to_string()))
            .cloned()
            .unwrap_or_default();
        let mut text = snapshot.to_string();
        for record in &waiting {
            if let Ok(Some(next)) = append_sent(&text, record) {
                text = next;
            }
        }
        text
    }

    fn take_all(&self) -> HashMap<(String, String), Vec<SentRecord>> {
        std::mem::take(&mut *self.lock())
    }

    fn restore(&self, items: HashMap<(String, String), Vec<SentRecord>>) {
        let mut guard = self.lock();
        for (conversation, mut records) in items {
            let entry = guard.entry(conversation).or_default();
            records.append(entry);
            *entry = records;
        }
    }
}

/// The process-wide queue used by the app.
pub static PENDING: std::sync::LazyLock<Pending> = std::sync::LazyLock::new(Pending::default);

/// The writable store; opened only after the single-writer checks.
pub struct HistoryWriter {
    connection: Connection,
    key: HistoryKey,
}

impl HistoryWriter {
    /// Opens the way the client does (WAL, full sync, secure delete). A legacy
    /// table is refused: starting the Pantheon client once migrates it.
    pub fn open(path: &Path, key: HistoryKey) -> Result<Self, String> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("could not open the Signal history for writing: {error}"))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(10))
            .map_err(|error| error.to_string())?;
        let _: String = connection
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        connection
            .execute_batch("PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;")
            .map_err(|error| error.to_string())?;
        let schema: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='message_history'",
                [],
                |row| row.get(0),
            )
            .ok();
        match schema {
            Some(sql) if sql.contains("UNIQUE(conversation_hash,timestamp,outgoing)") => {
                return Err("The Signal history uses an old format. Start Signal Pantheon once to update it.".into());
            }
            Some(_) => {}
            None => return Err("The Signal history has no messages table yet.".into()),
        }
        Ok(Self { connection, key })
    }

    pub fn replace_snapshots(&mut self, items: &[(String, String)]) -> Result<(), String> {
        let mut prepared = Vec::new();
        for (key_string, snapshot) in items {
            let sealed = self.key.encrypt(snapshot.as_bytes())?;
            prepared.push((
                self.key.hash(key_string),
                sealed.nonce,
                sealed.ciphertext,
                sealed.tag,
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("could not begin the Signal history update: {error}"))?;
        for (hash, nonce, ciphertext, tag) in &prepared {
            transaction
                .execute(
                    "DELETE FROM message_history WHERE conversation_hash=?",
                    [hash],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO message_history(conversation_hash,timestamp,outgoing,nonce,ciphertext,tag) VALUES(?,0,0,?,?,?)",
                    params![hash, nonce, ciphertext, tag],
                )
                .map_err(|error| format!("could not save the Signal history: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("could not commit the Signal history update: {error}"))
    }

    /// Consistent copy taken before the first write this app ever makes.
    pub fn backup_once(&self, destination: &Path) -> Result<(), String> {
        if destination.exists() {
            return Ok(());
        }
        let mut target = Connection::open(destination).map_err(|error| error.to_string())?;
        rusqlite::backup::Backup::new(&self.connection, &mut target)
            .and_then(|backup| {
                backup.run_to_completion(256, std::time::Duration::from_millis(5), None)
            })
            .map_err(|error| error.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600)).ok();
        Ok(())
    }
}

/// What happened to a batch of sent messages.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// Written to the history file.
    Written,
    /// The Pantheon client is running (or another writer is active): the
    /// messages are kept in memory and shown, and written later.
    Deferred,
}

/// Write everything in `PENDING` if (and only if) this process may write.
/// Never touches the file while the Pantheon client runs.
pub fn flush_pending(
    data_directory: &Path,
    account: &str,
    key: HistoryKey,
    pending: &Pending,
) -> Result<Saved, String> {
    flush_pending_with(data_directory, account, key, pending, owner::client_running)
}

pub(crate) fn flush_pending_with(
    data_directory: &Path,
    account: &str,
    key: HistoryKey,
    pending: &Pending,
    client_running: impl Fn() -> bool,
) -> Result<Saved, String> {
    if pending.is_empty() {
        return Ok(Saved::Written);
    }
    if client_running() {
        return Ok(Saved::Deferred);
    }
    let Some(_lock) = WriterLock::try_acquire(data_directory)? else {
        return Ok(Saved::Deferred);
    };
    // The client could have started between the check and the lock.
    if client_running() {
        return Ok(Saved::Deferred);
    }
    let database = data_directory.join("message-history.db");
    if !database.exists() {
        return Ok(Saved::Deferred);
    }
    let items = pending.take_all();
    match write_items(data_directory, &database, account, key, &items) {
        Ok(leftover) => {
            // Conversations missing from the history stay queued rather than vanish.
            let waiting = !leftover.is_empty();
            pending.restore(leftover);
            Ok(if waiting {
                Saved::Deferred
            } else {
                Saved::Written
            })
        }
        Err(error) => {
            pending.restore(items);
            Err(error)
        }
    }
}

fn write_items(
    data_directory: &Path,
    database: &Path,
    account: &str,
    key: HistoryKey,
    items: &HashMap<(String, String), Vec<SentRecord>>,
) -> Result<HashMap<(String, String), Vec<SentRecord>>, String> {
    let mut writer = HistoryWriter::open(database, key)?;
    writer.backup_once(&data_directory.join("message-history.db.pre-messages-send"))?;
    // Read what is stored now (inside the lock), never what we cached earlier.
    let reader = SignalStore::open(database, writer.key.clone())?;
    let mut updates = Vec::new();
    let mut leftover = HashMap::new();
    for ((item_account, id), records) in items {
        let snapshot = if item_account == account {
            reader.read_snapshot(&conversation_key(account, id))?
        } else {
            None
        };
        let Some(mut snapshot) = snapshot else {
            leftover.insert((item_account.clone(), id.clone()), records.clone());
            continue;
        };
        let mut changed = false;
        for record in records {
            if let Some(next) = append_sent(&snapshot, record)? {
                snapshot = next;
                changed = true;
            }
        }
        if changed {
            updates.push((conversation_key(account, id), snapshot));
        }
    }
    if !updates.is_empty() {
        writer.replace_snapshots(&updates)?;
    }
    Ok(leftover)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::{catalog_key, conversation_key};

    const KEY: [u8; 32] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];
    const ACCOUNT: &str = "+15550000001";
    const CONVERSATION: &str = r#"{"version":1,"id":"+15550000002","name":"Alice","group":false,"draft":"typing","unread":0,"clockFloor":7,"future":{"kept":true},"messages":[{"text":"hello","author":"+15550000002","timestamp":1790000000000,"outgoing":false,"localRead":true,"unknownMember":1}]}"#;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("signal-write-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();
        let connection = Connection::open(directory.join("message-history.db")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE message_history (id INTEGER PRIMARY KEY, \
                 conversation_hash BLOB NOT NULL, timestamp INTEGER NOT NULL, \
                 outgoing INTEGER NOT NULL, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, \
                 tag BLOB NOT NULL)",
            )
            .unwrap();
        drop(connection);
        let mut writer = HistoryWriter::open(
            &directory.join("message-history.db"),
            HistoryKey::from_raw(&KEY).unwrap(),
        )
        .unwrap();
        writer
            .replace_snapshots(&[
                (catalog_key(ACCOUNT), r#"["+15550000002"]"#.to_string()),
                (
                    conversation_key(ACCOUNT, "+15550000002"),
                    CONVERSATION.to_string(),
                ),
            ])
            .unwrap();
        directory
    }

    fn record(timestamp: i64) -> SentRecord {
        SentRecord {
            text: "on my way".into(),
            author: ACCOUNT.into(),
            timestamp,
            expiration_seconds: 0,
            expiration_start_ms: 0,
        }
    }

    fn reader(directory: &Path) -> SignalStore {
        SignalStore::open(
            &directory.join("message-history.db"),
            HistoryKey::from_raw(&KEY).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn encryption_reproduces_an_independently_computed_row() {
        // Same vector as signal.rs: computed with Python's `cryptography`.
        let key = HistoryKey::from_raw(&KEY).unwrap();
        let nonce: [u8; 12] = std::array::from_fn(|index| 100 + index as u8);
        let sealed = key
            .encrypt_with_nonce(&nonce, br#"["+15550000002"]"#)
            .unwrap();
        assert_eq!(sealed.ciphertext, unhex("1339f5574cdc63ae0e526fd8ea5748a0"));
        assert_eq!(sealed.tag, unhex("c03967b6e45742ce433662a7724a7d42"));
    }

    #[test]
    fn appending_keeps_everything_else_and_never_twice() {
        let first = append_sent(CONVERSATION, &record(1790000100000))
            .unwrap()
            .unwrap();
        let root: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(root["future"]["kept"], true);
        assert_eq!(root["clockFloor"], 7);
        assert_eq!(root["draft"], "");
        assert_eq!(root["messages"][0]["unknownMember"], 1);
        let sent = &root["messages"][1];
        assert_eq!(sent["text"], "on my way");
        assert_eq!(sent["outgoing"], true);
        assert_eq!(sent["author"], ACCOUNT);
        assert_eq!(sent["status"], "sent");
        assert_eq!(sent["localRead"], true);
        assert!(
            append_sent(&first, &record(1790000100000))
                .unwrap()
                .is_none()
        );
        assert!(append_sent("not json", &record(1)).is_err());
    }

    #[test]
    fn a_written_message_reads_back_through_the_reader() {
        let directory = fixture("roundtrip");
        let pending = Pending::default();
        pending.push(ACCOUNT, "+15550000002", record(1790000100000));
        let saved = flush_pending_with(
            &directory,
            ACCOUNT,
            HistoryKey::from_raw(&KEY).unwrap(),
            &pending,
            || false,
        )
        .unwrap();
        assert_eq!(saved, Saved::Written);
        assert!(pending.is_empty());
        let thread = reader(&directory)
            .conversation(ACCOUNT, "+15550000002")
            .unwrap()
            .unwrap();
        let bodies: Vec<_> = thread.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["hello", "on my way"]);
        assert!(thread.messages[1].outgoing);
        // The catalog is untouched and the one-time backup exists.
        assert_eq!(
            reader(&directory).conversation_ids(ACCOUNT).unwrap(),
            ["+15550000002"]
        );
        assert!(
            directory
                .join("message-history.db.pre-messages-send")
                .exists()
        );
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn nothing_is_written_while_the_client_runs() {
        let directory = fixture("running");
        let before = std::fs::read(directory.join("message-history.db")).unwrap();
        let pending = Pending::default();
        pending.push(ACCOUNT, "+15550000002", record(1790000100000));
        let saved = flush_pending_with(
            &directory,
            ACCOUNT,
            HistoryKey::from_raw(&KEY).unwrap(),
            &pending,
            || true,
        )
        .unwrap();
        assert_eq!(saved, Saved::Deferred);
        assert!(!pending.is_empty());
        assert_eq!(
            std::fs::read(directory.join("message-history.db")).unwrap(),
            before
        );
        assert!(
            !directory
                .join("message-history.db.pre-messages-send")
                .exists()
        );
        // The message is still shown, merged from memory.
        let shown = pending.overlay(ACCOUNT, "+15550000002", CONVERSATION);
        assert!(shown.contains("on my way"));
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_second_writer_defers_instead_of_interleaving() {
        let directory = fixture("locked");
        let _held = WriterLock::try_acquire(&directory).unwrap().unwrap();
        let pending = Pending::default();
        pending.push(ACCOUNT, "+15550000002", record(1790000100000));
        let saved = flush_pending_with(
            &directory,
            ACCOUNT,
            HistoryKey::from_raw(&KEY).unwrap(),
            &pending,
            || false,
        )
        .unwrap();
        assert_eq!(saved, Saved::Deferred);
        assert!(!pending.is_empty());
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_failed_write_keeps_the_messages_queued() {
        let directory = fixture("failed");
        let pending = Pending::default();
        pending.push(ACCOUNT, "+15550000002", record(1790000100000));
        // A key that cannot decrypt the stored snapshot makes the write fail.
        let wrong = HistoryKey::from_raw(&[9; 32]).unwrap();
        assert_eq!(
            flush_pending_with(&directory, ACCOUNT, wrong, &pending, || false),
            Ok(Saved::Deferred)
        );
        assert!(!pending.is_empty());
        std::fs::remove_dir_all(&directory).ok();
    }
}
