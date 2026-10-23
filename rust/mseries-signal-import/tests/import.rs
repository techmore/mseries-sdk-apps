//! Importer tests on synthetic data only: invented names, 555 numbers, random
//! keys. No real chat, key or keyring is touched.

#![allow(clippy::type_complexity)]

use mseries_signal_import::{
    chromium,
    history::{HistoryKey, HistoryStore, catalog_key, conversation_key},
    import::{self, Config},
    keyring::Secrets,
    secret::Secret,
};
use openssl::{
    hash::MessageDigest,
    pkcs5::pbkdf2_hmac,
    symm::{Cipher, encrypt},
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

const HISTORY_KEY: [u8; 32] = [7; 32];
const SIGNAL_KEY_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KEYRING_PASSWORD: &[u8] = b"synthetic-safe-storage-password";
const ACCOUNT: &str = "+15550001";
const ALICE: (&str, &str) = ("+15550101", "11111111-1111-4111-8111-111111111111");
const BOB: (&str, &str) = ("+15550102", "22222222-2222-4222-8222-222222222222");
const GROUP: &str = "R3JvdXBJZEZvclRlc3RzMDAwMDAwMDAwMDAwMDAwMDA=";

struct FakeSecrets {
    history_key: Option<Vec<u8>>,
    signal_password: Option<Vec<u8>>,
}

impl Secrets for FakeSecrets {
    fn lookup(&self, attributes: &[(&str, &str)]) -> Result<Option<Secret>, String> {
        let bytes = match attributes {
            [("purpose", "encrypted-message-history")] => self.history_key.as_ref().map(|k| openssl::base64::encode_block(k).into_bytes()),
            [("application", "Signal Beta")] => self.signal_password.clone(),
            other => panic!("unexpected keyring lookup {other:?}"),
        };
        Ok(bytes.map(Secret::new))
    }
}

fn secrets() -> FakeSecrets {
    FakeSecrets {
        history_key: Some(HISTORY_KEY.to_vec()),
        signal_password: Some(KEYRING_PASSWORD.to_vec()),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("import-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn chromium_encrypt(plain: &str) -> String {
    let mut key = [0u8; 16];
    pbkdf2_hmac(KEYRING_PASSWORD, b"saltysalt", 1, MessageDigest::sha1(), &mut key).unwrap();
    let ct = encrypt(Cipher::aes_128_cbc(), &key, Some(&[b' '; 16]), plain.as_bytes()).unwrap();
    let mut blob = b"v11".to_vec();
    blob.extend(ct);
    chromium::encode_hex(&blob)
}

fn message(conv: &str, ty: &str, sent_at: i64, body: &str, extra: Value) -> (String, String, i64, String, String) {
    let mut json = json!({"conversationId": conv, "type": ty, "sent_at": sent_at, "body": body, "timestamp": sent_at});
    if let (Some(obj), Some(more)) = (json.as_object_mut(), extra.as_object()) {
        obj.extend(more.clone());
    }
    (conv.into(), ty.into(), sent_at, body.into(), json.to_string())
}

/// A Signal Desktop profile: SQLCipher database + config.json.
fn build_signal(dir: &Path, messages: Vec<(String, String, i64, String, String)>) {
    fs::create_dir_all(dir.join("sql")).unwrap();
    fs::write(
        dir.join("config.json"),
        json!({"encryptedKey": chromium_encrypt(SIGNAL_KEY_HEX)}).to_string(),
    )
    .unwrap();
    let conn = Connection::open(dir.join("sql/db.sqlite")).unwrap();
    conn.execute_batch(&format!("PRAGMA key = \"x'{SIGNAL_KEY_HEX}'\";")).unwrap();
    conn.execute_batch(
        "CREATE TABLE conversations (id STRING PRIMARY KEY, type STRING, e164 TEXT, serviceId TEXT, groupId TEXT, name TEXT, profileName TEXT, json TEXT);
         CREATE TABLE messages (id STRING PRIMARY KEY, conversationId STRING, json TEXT, sent_at INTEGER, type STRING, body TEXT);",
    )
    .unwrap();
    let conversations: [(&str, &str, Option<&str>, Option<&str>, Option<&str>, &str); 4] = [
        ("c-alice", "private", Some(ALICE.0), Some(ALICE.1), None, "Alice Example"),
        ("c-bob", "private", Some(BOB.0), Some(BOB.1), None, "Bob Example"),
        ("c-group", "group", None, None, Some(GROUP), "Synthetic Group"),
        ("c-empty", "private", Some("+15550109"), None, None, "Nobody Here"),
    ];
    for (id, kind, e164, service, group, name) in conversations {
        conn.execute(
            "INSERT INTO conversations VALUES (?,?,?,?,?,?,NULL,?)",
            rusqlite::params![id, kind, e164, service, group, name, "{}"],
        )
        .unwrap();
    }
    for (n, (conv, ty, sent_at, body, json)) in messages.into_iter().enumerate() {
        conn.execute(
            "INSERT INTO messages VALUES (?,?,?,?,?,?)",
            rusqlite::params![format!("m{n}"), conv, json, sent_at, ty, body],
        )
        .unwrap();
    }
}

fn default_messages() -> Vec<(String, String, i64, String, String)> {
    vec![
        message(
            "c-alice",
            "incoming",
            1_700_000_001_000,
            "Hello from Alice",
            json!({"source": ALICE.0, "sourceServiceId": ALICE.1}),
        ),
        message("c-alice", "outgoing", 1_700_000_002_000, "Hi Alice", json!({})),
        message(
            "c-alice",
            "incoming",
            1_700_000_003_000,
            "",
            json!({"attachments": [{"contentType": "image/jpeg"}], "sourceServiceId": ALICE.1}),
        ),
        message(
            "c-bob",
            "incoming",
            1_700_000_004_000,
            "Bob here",
            json!({"sourceServiceId": BOB.1}),
        ),
        message(
            "c-group",
            "incoming",
            1_700_000_005_000,
            "Group hello",
            json!({"sourceServiceId": BOB.1, "source": BOB.0}),
        ),
        message("c-group", "outgoing", 1_700_000_006_000, "Group reply", json!({})),
        // Left out on purpose.
        message(
            "c-alice",
            "incoming",
            1_700_000_007_000,
            "secret view once",
            json!({"isViewOnce": true}),
        ),
        message(
            "c-alice",
            "incoming",
            1_700_000_008_000,
            "vanishes",
            json!({"expireTimer": 60, "expirationStartTimestamp": 1_700_000_008_000_i64}),
        ),
        message("c-alice", "keychange", 1_700_000_009_000, "", json!({})),
        message("c-alice", "incoming", 1_700_000_010_000, "story reply", json!({"storyId": "x"})),
        message("c-alice", "incoming", 1_700_000_011_000, "", json!({})),
    ]
}

/// A Pantheon profile as the client leaves it: catalog + one conversation.
fn build_pantheon(dir: &Path, existing: Option<(&str, Value)>) {
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(
        dir.join("data/accounts.json"),
        json!({"accounts": [{"path": "248846", "environment": "LIVE", "number": ACCOUNT, "uuid": "33333333-3333-4333-8333-333333333333"}], "version": 2}).to_string(),
    )
    .unwrap();
    let mut store = HistoryStore::open(
        &dir.join("message-history.db"),
        HistoryKey::from_raw(HISTORY_KEY.to_vec()).unwrap(),
        true,
    )
    .unwrap();
    // The client always has a catalog once it has run.
    let mut items = vec![];
    let catalog: Vec<&str> = existing.iter().map(|(id, _)| *id).collect();
    items.push((catalog_key(ACCOUNT), serde_json::to_string(&catalog).unwrap()));
    if let Some((id, snapshot)) = existing {
        items.push((conversation_key(ACCOUNT, id), snapshot.to_string()));
    }
    store.replace_snapshots(&items).unwrap();
}

fn config(name: &str, signal: &Path, pantheon: &Path) -> Config {
    Config {
        signal_dir: signal.to_path_buf(),
        app_name: "Signal Beta".into(),
        history_db: pantheon.join("message-history.db"),
        accounts_json: pantheon.join("data/accounts.json"),
        scratch_dir: scratch(&format!("{name}-scratch")),
        dry_run: false,
        check_running: false,
        account_override: None,
    }
}

fn reopen(pantheon: &Path) -> HistoryStore {
    HistoryStore::open(
        &pantheon.join("message-history.db"),
        HistoryKey::from_raw(HISTORY_KEY.to_vec()).unwrap(),
        false,
    )
    .unwrap()
}

fn snapshot(pantheon: &Path, id: &str) -> Option<Value> {
    reopen(pantheon)
        .read_snapshot(&conversation_key(ACCOUNT, id))
        .unwrap()
        .map(|s| serde_json::from_str(&s).unwrap())
}

fn catalog(pantheon: &Path) -> Vec<String> {
    serde_json::from_str(&reopen(pantheon).read_snapshot(&catalog_key(ACCOUNT)).unwrap().unwrap()).unwrap()
}

fn existing_alice() -> Value {
    json!({
        "version": 1, "id": ALICE.0, "name": "Alice (client)", "group": false, "draft": "keep me", "attachmentDraft": "", "unread": 2,
        "expired": {}, "cleanup": [], "viewOnceDraft": false, "readSyncQueue": [], "clockFloor": 5, "metadata": {"future": "field"},
        "messages": [{
            "text": "Hello from Alice", "author": ALICE.0, "timestamp": 1_700_000_001_000_i64, "modified": 0, "outgoing": false,
            "deleted": false, "viewOnce": false, "viewOpened": false, "localRead": false, "readAt": 0, "expirationSeconds": 0,
            "expirationStart": 0, "status": "sent", "attachments": [], "reactions": {}, "reactionVersions": {}, "clientOnly": 1
        }],
        "pending": [{"kind": "message"}]
    })
}

#[test]
fn independent_vectors_match_the_clients_scheme() {
    // Produced with Python's `cryptography` (HMAC-SHA256 / AES-256-GCM / Chromium
    // v11), so these check the format, not just a round trip of this crate.
    let key = HistoryKey::from_raw((0u8..32).collect()).unwrap();
    assert_eq!(
        chromium::encode_hex(&key.hash("pantheon-v1:+15550100:catalog").unwrap()),
        "a1413a17be33a4368488afc153448cea1db20cf42792f2e0a623f666f9dc3ff0"
    );
    let nonce: [u8; 12] = std::array::from_fn(|i| 100 + i as u8);
    let sealed = key.encrypt_with_nonce(&nonce, br#"{"version":1}"#).unwrap();
    let (ciphertext, tag) = (sealed.ciphertext, sealed.tag);
    assert_eq!(chromium::encode_hex(&ciphertext), "3339a8030b9a3ff1504065d9a7");
    assert_eq!(chromium::encode_hex(&tag), "008729ef17acf2a12ced3608109553d2");
    assert_eq!(key.decrypt(&nonce, &ciphertext, &tag).unwrap(), br#"{"version":1}"#);

    let blob = chromium::decode_hex(
        "7631310e08171e7ef9c9c4659473b8002bb1a8289beec6f1b69beb72db8a53272e1ce5a9326c1dda866288f3f8485834fe6963673f16922e9c874fdc09c9d84f261c295e357114086c0e5a08f0a5025295d335",
    )
    .unwrap();
    let plain = chromium::decrypt_blob(&blob, Some(b"test-password")).unwrap();
    assert_eq!(plain, SIGNAL_KEY_HEX.as_bytes());
    assert!(chromium::decrypt_blob(&blob, Some(b"wrong")).is_err());
}

#[test]
fn imports_into_a_fresh_conversation_set_and_is_idempotent() {
    let signal = scratch("fresh-signal");
    let pantheon = scratch("fresh-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(
        &pantheon,
        Some((
            "+15550777",
            json!({"version":1,"id":"+15550777","name":"Existing","group":false,"draft":"","attachmentDraft":"","unread":0,"expired":{},"cleanup":[],"viewOnceDraft":false,"readSyncQueue":[],"clockFloor":0,"metadata":{},"messages":[],"pending":[]}),
        )),
    );
    let cfg = config("fresh", &signal, &pantheon);

    let report = import::import(&cfg, &secrets()).unwrap();
    assert_eq!(report.conversations_created, 3, "alice, bob and the group");
    assert_eq!(report.messages_added, 6);
    assert_eq!(report.skipped.get("view-once"), Some(&1));
    assert_eq!(report.skipped.get("already expired"), Some(&1));
    assert_eq!(report.skipped.get("story"), Some(&1));
    assert_eq!(report.skipped.get("not a chat message"), Some(&1));
    assert_eq!(report.skipped.get("no content"), Some(&1));
    assert!(report.backup.is_some());

    let alice = snapshot(&pantheon, ALICE.0).expect("alice is imported by phone number");
    let messages = alice["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["text"], "Hello from Alice");
    assert_eq!(messages[0]["author"], ALICE.0);
    assert_eq!(messages[0]["outgoing"], false);
    assert_eq!(messages[1]["outgoing"], true);
    assert_eq!(messages[1]["author"], ACCOUNT, "own messages use the account number");
    assert_eq!(messages[2]["text"], "[Attachment not imported: image/jpeg]");
    assert!(messages.iter().all(|m| m["localRead"] == true && m["expirationSeconds"] == 0));
    assert_eq!(alice["unread"], 0);
    assert_eq!(alice["version"], 1);
    assert_eq!(alice["name"], "Alice Example");

    let group = snapshot(&pantheon, GROUP).unwrap();
    assert_eq!(group["group"], true);
    assert_eq!(group["messages"][0]["author"], BOB.0, "group senders are named like private chats");
    assert_eq!(catalog(&pantheon), vec!["+15550777", ALICE.0, BOB.0, GROUP]);
    assert!(snapshot(&pantheon, "+15550109").is_none(), "an empty chat is not created");

    let rows = reopen(&pantheon).row_count().unwrap();
    let again = import::import(&cfg, &secrets()).unwrap();
    assert_eq!(again.messages_added, 0);
    assert_eq!(again.messages_already_present, 6);
    assert_eq!(again.conversations_unchanged, 3);
    assert_eq!(again.backup, None);
    assert_eq!(reopen(&pantheon).row_count().unwrap(), rows, "a second run writes nothing");
}

#[test]
fn merges_into_an_existing_client_conversation_without_losing_its_data() {
    let signal = scratch("merge-signal");
    let pantheon = scratch("merge-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));

    let report = import::import(&config("merge", &signal, &pantheon), &secrets()).unwrap();
    assert_eq!(report.conversations_merged, 1);
    assert_eq!(report.conversations_created, 2);
    assert_eq!(report.messages_already_present, 1, "the message the client already had");

    let alice = snapshot(&pantheon, ALICE.0).unwrap();
    assert_eq!(alice["name"], "Alice (client)");
    assert_eq!(alice["draft"], "keep me");
    assert_eq!(alice["unread"], 2);
    assert_eq!(alice["clockFloor"], 5);
    assert_eq!(alice["metadata"]["future"], "field");
    assert_eq!(alice["pending"][0]["kind"], "message");
    let messages = alice["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["clientOnly"], 1, "the client's own record is kept untouched");
    assert_eq!(messages[0]["localRead"], false);
    assert_eq!(
        messages[1]["author"], ACCOUNT,
        "no outgoing history: falls back to the account number"
    );
    assert_eq!(catalog(&pantheon), vec![ALICE.0, BOB.0, GROUP]);
}

#[test]
fn follows_the_catalog_id_form_for_existing_chats() {
    let signal = scratch("uuid-signal");
    let pantheon = scratch("uuid-pantheon");
    build_signal(&signal, default_messages());
    let mut existing = existing_alice();
    existing["id"] = json!(ALICE.1);
    build_pantheon(&pantheon, Some((ALICE.1, existing)));

    import::import(&config("uuid", &signal, &pantheon), &secrets()).unwrap();
    assert!(snapshot(&pantheon, ALICE.0).is_none(), "no duplicate by phone number");
    assert_eq!(snapshot(&pantheon, ALICE.1).unwrap()["messages"].as_array().unwrap().len(), 3);
    // The catalog now prefers uuids, so new chats follow it.
    assert!(snapshot(&pantheon, BOB.1).is_some());
}

#[test]
fn dry_run_writes_nothing_and_resume_after_interruption_completes() {
    let signal = scratch("dry-signal");
    let pantheon = scratch("dry-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let before = reopen(&pantheon).row_count().unwrap();

    let mut cfg = config("dry", &signal, &pantheon);
    cfg.dry_run = true;
    let report = import::import(&cfg, &secrets()).unwrap();
    assert_eq!(report.messages_added, 5);
    assert_eq!(reopen(&pantheon).row_count().unwrap(), before);
    assert!(!import::backup_path(&cfg.history_db).exists());

    // An interrupted run leaves whole conversations: simulate by importing only
    // Alice's chat first (drop the others from a first source), then everything.
    let partial = scratch("dry-signal-partial");
    build_signal(&partial, default_messages().into_iter().filter(|m| m.0 == "c-alice").collect());
    cfg.dry_run = false;
    cfg.signal_dir = partial;
    import::import(&cfg, &secrets()).unwrap();
    assert_eq!(catalog(&pantheon), vec![ALICE.0]);
    cfg.signal_dir = signal;
    let resumed = import::import(&cfg, &secrets()).unwrap();
    assert_eq!(resumed.messages_added, 3, "only what the first run had not yet written");
    assert_eq!(catalog(&pantheon), vec![ALICE.0, BOB.0, GROUP]);
}

#[test]
fn source_data_is_never_modified() {
    let signal = scratch("ro-signal");
    let pantheon = scratch("ro-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let digest = |p: &Path| fs::read(p).unwrap();
    let (db, config_json) = (digest(&signal.join("sql/db.sqlite")), digest(&signal.join("config.json")));
    let cfg = config("ro", &signal, &pantheon);
    import::import(&cfg, &secrets()).unwrap();
    assert_eq!(db, digest(&signal.join("sql/db.sqlite")));
    assert_eq!(config_json, digest(&signal.join("config.json")));
    let leftovers = fs::read_dir(&cfg.scratch_dir).unwrap().count();
    assert_eq!(leftovers, 0, "the decrypted-free scratch copy is removed");
}

#[test]
fn refuses_without_keys_or_with_the_wrong_one() {
    let signal = scratch("keys-signal");
    let pantheon = scratch("keys-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let cfg = config("keys", &signal, &pantheon);

    let no_history = FakeSecrets {
        history_key: None,
        signal_password: Some(KEYRING_PASSWORD.to_vec()),
    };
    assert!(import::import(&cfg, &no_history).unwrap_err().contains("history key"));
    let no_signal = FakeSecrets {
        history_key: Some(HISTORY_KEY.to_vec()),
        signal_password: None,
    };
    assert!(import::import(&cfg, &no_signal).unwrap_err().contains("Safe Storage"));
    let wrong_signal = FakeSecrets {
        history_key: Some(HISTORY_KEY.to_vec()),
        signal_password: Some(b"wrong".to_vec()),
    };
    assert!(import::import(&cfg, &wrong_signal).is_err());
    let wrong_history = FakeSecrets {
        history_key: Some(vec![9; 32]),
        signal_password: Some(KEYRING_PASSWORD.to_vec()),
    };
    assert!(
        import::import(&cfg, &wrong_history).is_err(),
        "a different key cannot find the catalog"
    );
    assert_eq!(
        snapshot(&pantheon, ALICE.0).unwrap()["messages"].as_array().unwrap().len(),
        1,
        "nothing was written"
    );
}

#[test]
fn probe_reports_structure_without_content() {
    let signal = scratch("probe-signal");
    let pantheon = scratch("probe-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let text = import::probe(&config("probe", &signal, &pantheon), &secrets()).unwrap();
    assert!(text.contains("id is a phone number, messages: 1"));
    assert!(text.contains("\"clockFloor\""));
    assert!(!text.contains("Hello from Alice") && !text.contains(ALICE.0) && !text.contains("Alice"));
}

struct Recorder {
    seen: Vec<import::Progress>,
    cancel_after_conversations: Option<usize>,
}

impl import::Observer for Recorder {
    fn progress(&mut self, p: &import::Progress) {
        self.seen.push(*p);
    }
    fn cancelled(&self) -> bool {
        self.cancel_after_conversations
            .is_some_and(|n| self.seen.last().is_some_and(|p| p.conversations_done >= n))
    }
}

#[test]
fn progress_reports_stages_and_counts_only() {
    let signal = scratch("prog-signal");
    let pantheon = scratch("prog-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let mut recorder = Recorder {
        seen: vec![],
        cancel_after_conversations: None,
    };
    import::import_with(&config("prog", &signal, &pantheon), &secrets(), &mut recorder).unwrap();

    let stages: Vec<_> = recorder.seen.iter().map(|p| p.stage).collect();
    let first = |s| stages.iter().position(|x| *x == s).unwrap();
    assert!(first(import::Stage::ReadingPantheon) < first(import::Stage::ReadingSignal));
    assert!(first(import::Stage::ReadingSignal) < first(import::Stage::Matching));
    assert!(first(import::Stage::Matching) < first(import::Stage::Writing));
    assert_eq!(stages.last(), Some(&import::Stage::Done));
    let last = recorder.seen.last().unwrap();
    assert_eq!((last.conversations_found, last.conversations_done), (3, 3));
    assert_eq!((last.messages_total, last.messages_done), (6, 6));
    assert_eq!((last.messages_added, last.duplicates_skipped), (5, 1));
    let counts: Vec<_> = recorder.seen.iter().map(|p| p.messages_done).collect();
    assert!(counts.windows(2).all(|w| w[0] <= w[1]), "progress never goes backwards");
    let text = mseries_signal_import::progress::line(last, std::time::Duration::from_secs(75));
    assert_eq!(text, "[done] chats 3/3 · messages 6/6 · added 5 · already there 1 · 1:15");
    assert_eq!(mseries_signal_import::progress::fraction(last), Some(1.0));
}

#[test]
fn cancel_stops_between_conversations_and_the_next_run_finishes() {
    let signal = scratch("cancel-signal");
    let pantheon = scratch("cancel-pantheon");
    build_signal(&signal, default_messages());
    build_pantheon(&pantheon, Some((ALICE.0, existing_alice())));
    let cfg = config("cancel", &signal, &pantheon);

    let mut recorder = Recorder {
        seen: vec![],
        cancel_after_conversations: Some(1),
    };
    let report = import::import_with(&cfg, &secrets(), &mut recorder).unwrap();
    assert!(report.cancelled);
    assert_eq!(recorder.seen.last().unwrap().stage, import::Stage::Cancelled);
    assert_eq!(
        catalog(&pantheon).len(),
        1,
        "stopped after one whole conversation; the rest not started"
    );
    assert_eq!(snapshot(&pantheon, ALICE.0).unwrap()["messages"].as_array().unwrap().len(), 3);

    let finished = import::import(&cfg, &secrets()).unwrap();
    assert!(!finished.cancelled);
    assert_eq!(finished.messages_added, 3, "only what the cancelled run had not written");
    assert_eq!(catalog(&pantheon), vec![ALICE.0, BOB.0, GROUP]);
}

#[test]
fn disappearing_messages_keep_their_timer_and_expired_ones_stay_out() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let signal = scratch("timer-signal");
    let pantheon = scratch("timer-pantheon");
    build_signal(
        &signal,
        vec![
            message(
                "c-alice",
                "incoming",
                now - 5_000,
                "still here",
                json!({"expireTimer": 3600, "expirationStartTimestamp": now - 5_000, "sourceServiceId": ALICE.1}),
            ),
            message(
                "c-alice",
                "incoming",
                now - 4_000,
                "not started",
                json!({"expireTimer": 600, "sourceServiceId": ALICE.1}),
            ),
            message(
                "c-alice",
                "incoming",
                now - 9_000_000,
                "gone",
                json!({"expireTimer": 60, "expirationStartTimestamp": now - 9_000_000}),
            ),
        ],
    );
    build_pantheon(&pantheon, None);
    let report = import::import(&config("timer", &signal, &pantheon), &secrets()).unwrap();
    assert_eq!(report.messages_added, 2);
    assert_eq!(report.skipped.get("already expired"), Some(&1));
    let messages = snapshot(&pantheon, ALICE.0).unwrap()["messages"].clone();
    let messages = messages.as_array().unwrap();
    assert_eq!(
        (messages[0]["expirationSeconds"].as_i64(), messages[0]["expirationStart"].as_i64()),
        (Some(3600), Some(now - 5_000))
    );
    assert_eq!(messages[1]["expirationSeconds"], 600);
    assert!(
        messages[1]["expirationStart"].as_i64().unwrap() >= now,
        "an unstarted timer starts at import"
    );
}
