//! The Signal Pantheon client's encrypted message store (`message-history.db`).
//!
//! This is a byte-compatible re-implementation of the client's own C store
//! (`signal_history_store_*`), recovered from the installed binary:
//!
//! * the 32-byte key is a base64 secret in the login keyring
//!   (`purpose=encrypted-message-history`, schema
//!   `io.github.sdolbec.SignalPantheonClient.MessageHistory`);
//! * `conversation_hash` = HMAC-SHA256(key, key-string);
//! * a row is AES-256-GCM with a random 12-byte nonce, no AAD, a 16-byte tag
//!   stored apart from the ciphertext;
//! * the client persists each conversation as one *snapshot* row
//!   (`timestamp = 0`, `outgoing = 0`) under the key string
//!   `pantheon-v1:<account>:conversation:<id>`, and a catalog snapshot (a JSON
//!   array of conversation ids) under `pantheon-v1:<account>:catalog`.

use crate::secret::Secret;
use openssl::{
    base64,
    hash::MessageDigest,
    pkey::PKey,
    rand::rand_bytes,
    sign::Signer,
    symm::{Cipher, decrypt_aead, encrypt_aead},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS message_history (id INTEGER PRIMARY KEY, \
conversation_hash BLOB NOT NULL, timestamp INTEGER NOT NULL, outgoing INTEGER NOT NULL, \
nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, tag BLOB NOT NULL)";

pub struct HistoryKey(Secret);

/// One encrypted row body: nonce, ciphertext and tag are stored apart.
pub struct Sealed {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub tag: Vec<u8>,
}

impl HistoryKey {
    /// The keyring stores the key as base64 text.
    pub fn from_keyring_secret(secret: &Secret) -> Result<Self, String> {
        let text: Vec<u8> = secret.expose().iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        let raw = base64::decode_block(std::str::from_utf8(&text).map_err(|_| "history key is not text")?)
            .map_err(|_| "history key is not valid base64".to_string())?;
        Self::from_raw(raw)
    }

    pub fn from_raw(raw: Vec<u8>) -> Result<Self, String> {
        if raw.len() != 32 {
            return Err("the saved local history key is invalid".into());
        }
        Ok(HistoryKey(Secret::new(raw)))
    }

    /// `conversation_hash` for a key string.
    pub fn hash(&self, key_string: &str) -> Result<Vec<u8>, String> {
        let pkey = PKey::hmac(self.0.expose()).map_err(|e| e.to_string())?;
        let mut signer = Signer::new(MessageDigest::sha256(), &pkey).map_err(|e| e.to_string())?;
        signer.update(key_string.as_bytes()).map_err(|e| e.to_string())?;
        signer.sign_to_vec().map_err(|e| e.to_string())
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Sealed, String> {
        let mut nonce = [0u8; 12];
        rand_bytes(&mut nonce).map_err(|e| e.to_string())?;
        self.encrypt_with_nonce(&nonce, plaintext)
    }

    pub fn encrypt_with_nonce(&self, nonce: &[u8; 12], plaintext: &[u8]) -> Result<Sealed, String> {
        let mut tag = [0u8; 16];
        let ciphertext =
            encrypt_aead(Cipher::aes_256_gcm(), self.0.expose(), Some(nonce), &[], plaintext, &mut tag).map_err(|e| e.to_string())?;
        Ok(Sealed {
            nonce: nonce.to_vec(),
            ciphertext,
            tag: tag.to_vec(),
        })
    }

    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Result<Vec<u8>, String> {
        if nonce.len() != 12 || tag.len() != 16 {
            return Err("a stored history row has an unexpected shape".into());
        }
        decrypt_aead(Cipher::aes_256_gcm(), self.0.expose(), Some(nonce), &[], ciphertext, tag)
            .map_err(|_| "a local history message could not be authenticated".to_string())
    }
}

pub struct HistoryStore {
    conn: Connection,
    key: HistoryKey,
}

impl HistoryStore {
    /// Opens the store the way the client does (WAL, full sync, secure delete).
    /// A legacy table with a UNIQUE constraint is refused: starting the Pantheon
    /// client once migrates it.
    pub fn open(path: &Path, key: HistoryKey, create: bool) -> Result<Self, String> {
        let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
        let conn = Connection::open_with_flags(path, flags).map_err(|e| format!("could not open local message history: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
        let _: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;")
            .map_err(|e| e.to_string())?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='message_history'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("could not inspect local message history schema: {e}"))?;
        match existing {
            Some(sql) if sql.contains("UNIQUE(conversation_hash,timestamp,outgoing)") => {
                return Err("message-history.db uses the legacy schema; start Signal Pantheon once so it migrates, then retry".into());
            }
            Some(_) => {}
            None => conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?,
        }
        Ok(HistoryStore { conn, key })
    }

    pub fn key(&self) -> &HistoryKey {
        &self.key
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn row_count(&self) -> Result<i64, String> {
        self.conn
            .query_row("SELECT count(*) FROM message_history", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    pub fn has_key_string(&self, key_string: &str) -> Result<bool, String> {
        let hash = self.key.hash(key_string)?;
        self.conn
            .query_row(
                "SELECT 1 FROM message_history WHERE conversation_hash=? LIMIT 1",
                [hash],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(|e| e.to_string())
    }

    /// The decrypted snapshot stored under a key string, if any.
    pub fn read_snapshot(&self, key_string: &str) -> Result<Option<String>, String> {
        let hash = self.key.hash(key_string)?;
        let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT nonce,ciphertext,tag FROM message_history WHERE conversation_hash=? AND timestamp=0 AND outgoing=0 ORDER BY id DESC LIMIT 1",
                [hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            None => Ok(None),
            Some((nonce, ciphertext, tag)) => {
                let plain = self.key.decrypt(&nonce, &ciphertext, &tag)?;
                String::from_utf8(plain)
                    .map(Some)
                    .map_err(|_| "a stored snapshot is not text".to_string())
            }
        }
    }

    /// Replaces several snapshots in one transaction, as the client's
    /// `signal_history_store_pair` does for a conversation and its catalog.
    pub fn replace_snapshots(&mut self, items: &[(String, String)]) -> Result<(), String> {
        let prepared: Vec<(Vec<u8>, Sealed)> = items
            .iter()
            .map(|(key_string, snapshot)| {
                let hash = self.key.hash(key_string)?;
                Ok((hash, self.key.encrypt(snapshot.as_bytes())?))
            })
            .collect::<Result<_, String>>()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("could not begin conversation update: {e}"))?;
        for (hash, sealed) in &prepared {
            tx.execute("DELETE FROM message_history WHERE conversation_hash=?", [hash])
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO message_history(conversation_hash,timestamp,outgoing,nonce,ciphertext,tag) VALUES(?,0,0,?,?,?)",
                params![hash, sealed.nonce, sealed.ciphertext, sealed.tag],
            )
            .map_err(|e| format!("could not save the encrypted message history: {e}"))?;
        }
        tx.commit().map_err(|e| format!("could not commit conversation update: {e}"))
    }

    /// Consistent copy of the store, taken before the first write.
    pub fn backup_to(&self, destination: &Path) -> Result<(), String> {
        let mut target = Connection::open(destination).map_err(|e| e.to_string())?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut target).map_err(|e| e.to_string())?;
        backup
            .run_to_completion(256, std::time::Duration::from_millis(5), None)
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

pub fn catalog_key(account: &str) -> String {
    format!("pantheon-v1:{account}:catalog")
}

pub fn conversation_key(account: &str, id: &str) -> String {
    format!("pantheon-v1:{account}:conversation:{id}")
}
