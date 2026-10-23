//! Signal Desktop → Signal Pantheon history import.
//!
//! The import is a merge, not a replay: every Signal chat becomes (or is merged
//! into) the Pantheon client's own conversation snapshot, and a message that the
//! snapshot already holds, matched on (timestamp, outgoing), is never added a
//! second time. Running it again, or after an interruption, adds nothing twice.

use crate::{
    chromium,
    history::{HistoryKey, HistoryStore, catalog_key, conversation_key},
    keyring::Secrets,
    signal_db::{self, DatabaseCopy, SigConversation, SigMessage},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub struct Config {
    pub signal_dir: PathBuf,
    pub app_name: String,
    pub history_db: PathBuf,
    pub accounts_json: PathBuf,
    pub scratch_dir: PathBuf,
    pub dry_run: bool,
    /// Refuse to run while the Pantheon client is open (it rewrites its
    /// snapshots from memory and would discard the import).
    pub check_running: bool,
    pub account_override: Option<String>,
}

impl Config {
    /// The usual locations for the current user.
    pub fn for_home(home: &Path) -> Config {
        let history_db = home.join(".local/share/signal-pantheon/message-history.db");
        Config {
            signal_dir: home.join(".config/Signal Beta"),
            app_name: "Signal Beta".into(),
            accounts_json: history_db.with_file_name("data").join("accounts.json"),
            history_db,
            scratch_dir: std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            dry_run: false,
            check_running: true,
            account_override: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Starting,
    ReadingPantheon,
    ReadingSignal,
    Matching,
    Writing,
    Done,
    Cancelled,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Starting => "starting",
            Stage::ReadingPantheon => "reading Signal Pantheon",
            Stage::ReadingSignal => "reading Signal Beta data",
            Stage::Matching => "matching conversations",
            Stage::Writing => "writing",
            Stage::Done => "done",
            Stage::Cancelled => "cancelled",
        }
    }
}

/// Counts and a stage name only; never any text, name or key.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub stage: Stage,
    /// Signal chats that have messages to import.
    pub conversations_found: usize,
    pub conversations_done: usize,
    pub messages_total: usize,
    /// Messages looked at so far (added or already present).
    pub messages_done: usize,
    pub messages_added: usize,
    pub duplicates_skipped: usize,
}

pub trait Observer {
    fn progress(&mut self, _progress: &Progress) {}
    /// Polled between conversations; each conversation is written atomically,
    /// so stopping there is always safe and the next run continues.
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct Silent;

impl Observer for Silent {}

#[derive(Debug, Default)]
pub struct Report {
    pub cancelled: bool,
    pub account_form: String,
    pub id_preference: &'static str,
    pub signal_conversations: usize,
    pub conversations_created: usize,
    pub conversations_merged: usize,
    pub conversations_unchanged: usize,
    pub conversations_unmapped: usize,
    pub conversations_failed: usize,
    pub messages_added: usize,
    pub messages_already_present: usize,
    pub skipped: BTreeMap<String, usize>,
    pub backup: Option<PathBuf>,
    pub dry_run: bool,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{}{}Signal chats read: {}\n  new in Pantheon: {}\n  merged into existing: {}\n  already complete: {}\n  not importable (no phone number, id or group id): {}\n  failed: {}\nmessages added: {}\nmessages already present: {}\n",
            if self.dry_run { "DRY RUN (nothing written)\n" } else { "" },
            if self.cancelled { "CANCELLED (run again to continue)\n" } else { "" },
            self.signal_conversations,
            self.conversations_created,
            self.conversations_merged,
            self.conversations_unchanged,
            self.conversations_unmapped,
            self.conversations_failed,
            self.messages_added,
            self.messages_already_present,
        ));
        if !self.skipped.is_empty() {
            out.push_str("left out on purpose:\n");
            for (reason, count) in &self.skipped {
                out.push_str(&format!("  {reason}: {count}\n"));
            }
        }
        out.push_str(&format!(
            "Pantheon store key form: {}; new chats are named by {}\n",
            self.account_form, self.id_preference
        ));
        if let Some(backup) = &self.backup {
            out.push_str(&format!("backup of the previous Pantheon history: {}\n", backup.display()));
        }
        out
    }
}

/// Whether a Signal Pantheon client process is alive.
pub fn pantheon_client_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else { return false };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(cmdline) = fs::read(entry.path().join("cmdline")) {
            let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or(&[]);
            if argv0.rsplit(|b| *b == b'/').next() == Some(b"signal-pantheon-client") {
                return true;
            }
        }
    }
    false
}

struct Account {
    number: Option<String>,
    candidates: Vec<String>,
}

fn read_accounts(path: &Path, override_key: &Option<String>) -> Result<Account, String> {
    let mut candidates = Vec::new();
    let mut number = None;
    if let Ok(text) = fs::read_to_string(path) {
        let json: Value = serde_json::from_str(&text).map_err(|_| "accounts.json is not valid JSON".to_string())?;
        if let Some(accounts) = json.get("accounts").and_then(Value::as_array) {
            for account in accounts {
                let field = |name: &str| account.get(name).and_then(Value::as_str).map(str::to_string);
                if number.is_none() {
                    number = field("number");
                }
                if let Some(n) = field("number") {
                    candidates.push(n.clone());
                    candidates.push(n.trim_start_matches('+').to_string());
                }
                candidates.extend(field("uuid"));
                candidates.extend(field("path"));
            }
        }
    }
    if let Some(key) = override_key {
        candidates = vec![key.clone()];
    }
    Ok(Account { number, candidates })
}

fn shape(id: &str) -> &'static str {
    let bytes = id.as_bytes();
    if bytes.first() == Some(&b'+') && bytes.len() > 4 && bytes[1..].iter().all(u8::is_ascii_digit) {
        "phone number"
    } else if bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|i| bytes[*i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| [8, 13, 18, 23].contains(&i) || b.is_ascii_hexdigit())
    {
        "uuid"
    } else {
        "group or other"
    }
}

fn normalize_group(id: &str) -> String {
    id.trim_end_matches('=').replace('-', "+").replace('_', "/")
}

/// The Pantheon client's view of the account, learned from its own store.
struct Pantheon {
    account: String,
    account_form: String,
    catalog: Vec<String>,
    snapshots: HashMap<String, Value>,
}

fn load_pantheon(store: &HistoryStore, account: &Account) -> Result<Pantheon, String> {
    let labelled: Vec<(&str, &String)> = account
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (["account number", "account number", "account uuid", "account folder"][i.min(3)], c))
        .collect();
    let found = labelled
        .into_iter()
        .find(|(_, c)| store.has_key_string(&catalog_key(c)).unwrap_or(false));
    let (form, account_key) = match found {
        Some((form, key)) => (form.to_string(), key.clone()),
        None => {
            return Err(
                "could not find Signal Pantheon's conversation catalog in message-history.db. Start Signal Pantheon, open a chat, close it, and run the import again (or pass --account-key)."
                    .into(),
            );
        }
    };
    let catalog_text = store
        .read_snapshot(&catalog_key(&account_key))?
        .ok_or("the conversation catalog vanished")?;
    let catalog: Vec<String> = serde_json::from_str::<Vec<Value>>(&catalog_text)
        .map_err(|_| "the Pantheon conversation catalog has an unexpected shape".to_string())?
        .into_iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or("the Pantheon conversation catalog has an unexpected entry".to_string())
        })
        .collect::<Result<_, _>>()?;
    let mut snapshots = HashMap::new();
    for id in &catalog {
        if let Some(text) = store.read_snapshot(&conversation_key(&account_key, id))? {
            let value: Value = serde_json::from_str(&text).map_err(|_| "a Pantheon conversation snapshot is not valid JSON".to_string())?;
            snapshots.insert(id.clone(), value);
        }
    }
    Ok(Pantheon {
        account: account_key,
        account_form: form,
        catalog,
        snapshots,
    })
}

fn id_preference(catalog: &[String]) -> &'static str {
    let phones = catalog.iter().filter(|c| shape(c) == "phone number").count();
    let uuids = catalog.iter().filter(|c| shape(c) == "uuid").count();
    if uuids > phones { "uuid" } else { "phone number" }
}

fn private_id(conv: &SigConversation, catalog: &HashSet<&str>, prefer_uuid: bool) -> Option<String> {
    let candidates = [conv.e164.as_deref(), conv.service_id.as_deref()];
    if let Some(existing) = candidates.iter().flatten().find(|c| catalog.contains(**c)) {
        return Some((*existing).to_string());
    }
    let order = if prefer_uuid {
        [candidates[1], candidates[0]]
    } else {
        [candidates[0], candidates[1]]
    };
    order.into_iter().flatten().next().map(str::to_string)
}

fn conversation_id(conv: &SigConversation, catalog: &[String], prefer_uuid: bool) -> Option<String> {
    if conv.is_group {
        let group = conv.group_id.as_deref()?;
        let wanted = normalize_group(group);
        Some(
            catalog
                .iter()
                .find(|c| normalize_group(c) == wanted)
                .cloned()
                .unwrap_or_else(|| group.to_string()),
        )
    } else {
        let set: HashSet<&str> = catalog.iter().map(String::as_str).collect();
        private_id(conv, &set, prefer_uuid)
    }
}

fn message_text(m: &SigMessage, names_by_service_id: &HashMap<&str, &str>) -> String {
    let mut mentions = m.mentions.iter();
    let mut body = String::with_capacity(m.body.len());
    for ch in m.body.chars() {
        if ch == '\u{fffc}' {
            let name = mentions
                .next()
                .and_then(|aci| names_by_service_id.get(aci.as_str()))
                .copied()
                .unwrap_or("someone");
            body.push('@');
            body.push_str(name);
        } else {
            body.push(ch);
        }
    }
    let mut lines = Vec::new();
    if !body.trim().is_empty() && !m.deleted {
        lines.push(body.trim_end().to_string());
    }
    if !m.deleted {
        for kind in &m.attachment_types {
            lines.push(format!("[Attachment not imported: {kind}]"));
        }
        if m.sticker {
            lines.push("[Sticker not imported]".into());
        }
        if m.shared_contact {
            lines.push("[Shared contact not imported]".into());
        }
    }
    lines.join("\n")
}

fn record(m: &SigMessage, author: &str, text: String) -> Value {
    json!({
        "text": text,
        "author": author,
        "timestamp": m.sent_at,
        "modified": 0,
        "outgoing": m.outgoing,
        "deleted": m.deleted,
        "viewOnce": false,
        "viewOpened": false,
        "localRead": true,
        "readAt": 0,
        "expirationSeconds": 0,
        "expirationStart": 0,
        "status": "sent",
        "attachments": [],
        "reactions": {},
        "reactionVersions": {}
    })
}

fn message_key(message: &Value) -> Option<(i64, bool)> {
    Some((message.get("timestamp")?.as_i64()?, message.get("outgoing")?.as_bool()?))
}

/// Merges imported records into a snapshot. Returns the new snapshot, how many
/// records were added and how many were already there.
fn merge(existing: Option<&Value>, id: &str, name: &str, group: bool, records: Vec<Value>) -> Result<(Value, usize, usize), String> {
    let mut snapshot = match existing {
        Some(value) => {
            let object = value.as_object().ok_or("snapshot is not an object")?;
            if object.get("version").and_then(Value::as_i64) != Some(1) || object.get("id").and_then(Value::as_str) != Some(id) {
                return Err("snapshot has a version or id this importer does not know".into());
            }
            value.clone()
        }
        None => json!({
            "version": 1, "id": id, "name": name, "group": group, "draft": "", "attachmentDraft": "", "unread": 0,
            "expired": {}, "cleanup": [], "viewOnceDraft": false, "readSyncQueue": [], "clockFloor": 0,
            "metadata": {}, "messages": [], "pending": []
        }),
    };
    let messages = snapshot
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .ok_or("snapshot has no message list")?;
    let mut seen: HashSet<(i64, bool)> = messages.iter().filter_map(message_key).collect();
    let (mut added, mut present) = (0, 0);
    for record in records {
        let key = message_key(&record).expect("records always carry a timestamp and direction");
        if seen.insert(key) {
            messages.push(record);
            added += 1;
        } else {
            present += 1;
        }
    }
    if added > 0 {
        messages.sort_by_key(|m| m.get("timestamp").and_then(Value::as_i64).unwrap_or(0));
    }
    Ok((snapshot, added, present))
}

fn most_common_outgoing_author(snapshots: &HashMap<String, Value>) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for snapshot in snapshots.values() {
        for m in snapshot.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let author = m.get("author").and_then(Value::as_str).filter(|a| !a.is_empty());
            if let (Some(true), Some(author)) = (m.get("outgoing").and_then(Value::as_bool), author) {
                *counts.entry(author).or_insert(0) += 1;
            }
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(a, _)| a.to_string())
}

fn signal_database_key(cfg: &Config, secrets: &dyn Secrets) -> Result<String, String> {
    let config_text =
        fs::read_to_string(cfg.signal_dir.join("config.json")).map_err(|e| format!("could not read the Signal Desktop config: {e}"))?;
    let config: Value = serde_json::from_str(&config_text).map_err(|_| "Signal Desktop config.json is not valid JSON".to_string())?;
    if let Some(encrypted) = config.get("encryptedKey").and_then(Value::as_str) {
        let blob = chromium::decode_hex(encrypted)?;
        // The keyring is read only when the key is actually protected by it, and
        // only for this one entry; the password stays in memory and is wiped.
        let password = if chromium::needs_keyring(&blob) {
            Some(
                secrets
                    .lookup(&[("application", cfg.app_name.as_str())])?
                    .ok_or_else(|| format!("no \"{}\" Safe Storage entry in the login keyring (is it unlocked?)", cfg.app_name))?,
            )
        } else {
            None
        };
        let plain = chromium::decrypt_blob(&blob, password.as_ref().map(|p| p.expose()))?;
        let key = String::from_utf8(plain).map_err(|_| "the decrypted Signal key is not text".to_string())?;
        validate_key(&key)?;
        Ok(key)
    } else if let Some(key) = config.get("key").and_then(Value::as_str) {
        validate_key(key)?;
        Ok(key.to_string())
    } else {
        Err("Signal Desktop config.json has no database key".into())
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("the Signal database key has an unexpected form".into())
    }
}

fn open_history(cfg: &Config, secrets: &dyn Secrets) -> Result<HistoryStore, String> {
    let secret = secrets
        .lookup(&[("purpose", "encrypted-message-history")])?
        .ok_or("no Signal Pantheon history key in the keyring; start Signal Pantheon once first")?;
    let key = HistoryKey::from_keyring_secret(&secret)?;
    HistoryStore::open(&cfg.history_db, key, false)
}

pub fn import(cfg: &Config, secrets: &dyn Secrets) -> Result<Report, String> {
    import_with(cfg, secrets, &mut Silent)
}

pub fn import_with(cfg: &Config, secrets: &dyn Secrets, observer: &mut dyn Observer) -> Result<Report, String> {
    let mut progress = Progress {
        stage: Stage::Starting,
        conversations_found: 0,
        conversations_done: 0,
        messages_total: 0,
        messages_done: 0,
        messages_added: 0,
        duplicates_skipped: 0,
    };
    observer.progress(&progress);
    if cfg.check_running && !cfg.dry_run && pantheon_client_running() {
        return Err(
            "Signal Pantheon is running; close it first (it would overwrite the imported history from memory), then run the import again"
                .into(),
        );
    }
    let mut report = Report {
        dry_run: cfg.dry_run,
        ..Report::default()
    };
    progress.stage = Stage::ReadingPantheon;
    observer.progress(&progress);
    let mut store = open_history(cfg, secrets)?;
    let account = read_accounts(&cfg.accounts_json, &cfg.account_override)?;
    let pantheon = load_pantheon(&store, &account)?;
    report.account_form = pantheon.account_form.clone();
    report.id_preference = id_preference(&pantheon.catalog);
    let prefer_uuid = report.id_preference == "uuid";
    let own_author = most_common_outgoing_author(&pantheon.snapshots)
        .or(account.number.clone())
        .unwrap_or_default();

    progress.stage = Stage::ReadingSignal;
    observer.progress(&progress);
    let data = {
        let key_hex = signal_database_key(cfg, secrets)?;
        let copy = DatabaseCopy::make(&cfg.signal_dir, &cfg.scratch_dir)?;
        let conn = signal_db::open_encrypted(&copy.db, &key_hex)?;
        signal_db::read_all(&conn)?
    };
    report.skipped = data.skipped.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    report.signal_conversations = data.conversations.len();

    let by_id: HashMap<&str, &SigConversation> = data.conversations.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut names_by_service_id: HashMap<&str, &str> = HashMap::new();
    let mut by_service_id: HashMap<&str, &SigConversation> = HashMap::new();
    for conv in data.conversations.iter().filter(|c| !c.is_group) {
        if let Some(service_id) = conv.service_id.as_deref() {
            names_by_service_id.insert(service_id, conv.name.as_str());
            by_service_id.insert(service_id, conv);
        }
    }
    let mut grouped: BTreeMap<&str, Vec<&SigMessage>> = BTreeMap::new();
    for message in &data.messages {
        grouped.entry(message.conversation_id.as_str()).or_default().push(message);
    }

    progress.stage = Stage::Matching;
    progress.conversations_found = grouped.len();
    progress.messages_total = grouped.values().map(Vec::len).sum();
    observer.progress(&progress);
    if observer.cancelled() {
        report.cancelled = true;
        progress.stage = Stage::Cancelled;
        observer.progress(&progress);
        return Ok(report);
    }

    progress.stage = Stage::Writing;
    observer.progress(&progress);
    let mut catalog = pantheon.catalog.clone();
    let mut backed_up = false;
    for (conversation, messages) in grouped {
        if observer.cancelled() {
            report.cancelled = true;
            break;
        }
        let message_count = messages.len();
        'one: {
            let Some(conv) = by_id.get(conversation) else { break 'one };
            let Some(id) = conversation_id(conv, &catalog, prefer_uuid) else {
                report.conversations_unmapped += 1;
                break 'one;
            };
            let records: Vec<Value> = messages
                .iter()
                .map(|m| {
                    let author = if m.outgoing {
                        own_author.clone()
                    } else if !conv.is_group {
                        id.clone()
                    } else {
                        let sender = m.sender_service_id.as_deref().and_then(|s| by_service_id.get(s)).copied();
                        sender
                            .and_then(|c| conversation_id(c, &catalog, prefer_uuid))
                            .or_else(|| m.sender_e164.clone())
                            .or_else(|| m.sender_service_id.clone())
                            .unwrap_or_default()
                    };
                    record(m, &author, message_text(m, &names_by_service_id))
                })
                .collect();
            let existing = pantheon.snapshots.get(&id);
            let name = if conv.name.is_empty() { id.as_str() } else { conv.name.as_str() };
            let (snapshot, added, present) = match merge(existing, &id, name, conv.is_group, records) {
                Ok(result) => result,
                Err(_) => {
                    report.conversations_failed += 1;
                    break 'one;
                }
            };
            report.messages_added += added;
            report.messages_already_present += present;
            let in_catalog = catalog.contains(&id);
            if added == 0 && existing.is_some() && in_catalog {
                report.conversations_unchanged += 1;
                break 'one;
            }
            if existing.is_some() {
                report.conversations_merged += 1
            } else {
                report.conversations_created += 1
            }
            if cfg.dry_run {
                if !in_catalog {
                    catalog.push(id);
                }
                break 'one;
            }
            if !backed_up {
                let backup = backup_path(&cfg.history_db);
                if !backup.exists() {
                    store.backup_to(&backup)?;
                    report.backup = Some(backup);
                }
                backed_up = true;
            }
            let snapshot_text = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
            let mut items = vec![(conversation_key(&pantheon.account, &id), snapshot_text.clone())];
            if !in_catalog {
                catalog.push(id.clone());
                items.push((
                    catalog_key(&pantheon.account),
                    serde_json::to_string(&catalog).map_err(|e| e.to_string())?,
                ));
            }
            store.replace_snapshots(&items)?;
            // Read it back through the same path the client uses before moving on.
            if store.read_snapshot(&items[0].0)?.as_deref() != Some(snapshot_text.as_str()) {
                return Err("a written conversation did not read back identically; stopping".into());
            }
        }
        progress.conversations_done += 1;
        progress.messages_done += message_count;
        progress.messages_added = report.messages_added;
        progress.duplicates_skipped = report.messages_already_present;
        observer.progress(&progress);
    }
    progress.stage = if report.cancelled { Stage::Cancelled } else { Stage::Done };
    observer.progress(&progress);
    Ok(report)
}

pub fn backup_path(history_db: &Path) -> PathBuf {
    let mut name = history_db.file_name().unwrap_or_default().to_os_string();
    name.push(".pre-signal-import");
    history_db.with_file_name(name)
}

/// Structure only: how the Pantheon store is laid out, never any text or key.
pub fn probe(cfg: &Config, secrets: &dyn Secrets) -> Result<String, String> {
    let store = open_history(cfg, secrets)?;
    let account = read_accounts(&cfg.accounts_json, &cfg.account_override)?;
    let pantheon = load_pantheon(&store, &account)?;
    let mut out = format!(
        "rows in message-history.db: {}\nstore key form: {}\nconversations in catalog: {}\n",
        store.row_count()?,
        pantheon.account_form,
        pantheon.catalog.len()
    );
    for (n, id) in pantheon.catalog.iter().enumerate() {
        let count = pantheon
            .snapshots
            .get(id)
            .and_then(|s| s.get("messages"))
            .and_then(Value::as_array)
            .map(Vec::len);
        out.push_str(&format!(
            "  #{n}: id is a {}, messages: {}\n",
            shape(id),
            count.map_or("no snapshot".into(), |c| c.to_string())
        ));
    }
    let with_messages = |s: &&Value| s.get("messages").and_then(Value::as_array).is_some_and(|m| !m.is_empty());
    if let Some(snapshot) = pantheon
        .snapshots
        .values()
        .find(with_messages)
        .or(pantheon.snapshots.values().next())
    {
        let members: Vec<&String> = snapshot.as_object().map(|o| o.keys().collect()).unwrap_or_default();
        out.push_str(&format!("snapshot members: {members:?}\n"));
        if let Some(message) = snapshot
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|m| m.first())
            .and_then(Value::as_object)
        {
            out.push_str(&format!("message members: {:?}\n", message.keys().collect::<Vec<_>>()));
        }
    }
    Ok(out)
}
