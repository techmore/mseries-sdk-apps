use rusqlite::{Connection, OptionalExtension, params};
use std::env;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8766/v1";
const MAX_MEMORY_CHARS: usize = 600;

#[derive(Clone, Debug)]
pub struct AiPreferences {
    /// Short user-authored style guidance shared by this app's drafts.
    pub writing_style: String,
    /// Allow explicitly saved global memories to be used by this app.
    pub use_shared_memory: bool,
}

impl Default for AiPreferences {
    fn default() -> Self {
        Self {
            writing_style: "Natural and concise. Keep the user's voice.".into(),
            use_shared_memory: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LocalModelSettings {
    pub endpoint: String,
}

impl Default for LocalModelSettings {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AiMemory {
    pub id: i64,
    /// `*` means shared by all apps; otherwise this is an app ID.
    pub scope: String,
    pub content: String,
}

pub struct AiStore {
    connection: Connection,
}

impl AiStore {
    /// Open the user-wide database shared by all M-Series apps.
    pub fn open_default() -> Result<Self, String> {
        Self::open_at(default_database_path()?)
    }

    /// Open a database at a caller-selected path. Apps should use `open_default`.
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| "AI database path has no parent directory".to_string())?;
        fs::create_dir_all(parent).map_err(storage_error)?;
        set_mode(parent, 0o700)?;
        ensure_private_database_file(path)?;

        let connection = Connection::open(path).map_err(storage_error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
            )
            .map_err(storage_error)?;

        let schema_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage_error)?;
        if schema_version > 1 {
            return Err(format!(
                "The AI database schema version {schema_version} is newer than this app supports"
            ));
        }
        if schema_version < 1 {
            connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE IF NOT EXISTS global_settings (
                         key TEXT PRIMARY KEY NOT NULL,
                         value TEXT NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS app_preferences (
                         app_id TEXT PRIMARY KEY NOT NULL,
                         writing_style TEXT NOT NULL CHECK (length(writing_style) <= 600),
                         use_shared_memory INTEGER NOT NULL DEFAULT 1 CHECK (use_shared_memory IN (0, 1)),
                         updated_at INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS memories (
                         id INTEGER PRIMARY KEY,
                         scope TEXT NOT NULL,
                         content TEXT NOT NULL,
                         created_at INTEGER NOT NULL,
                         CHECK (scope = '*' OR length(scope) > 0),
                         CHECK (length(trim(content)) > 0),
                         CHECK (length(content) <= 600)
                     );
                     CREATE INDEX IF NOT EXISTS memories_scope_created
                         ON memories(scope, created_at DESC);
                     PRAGMA user_version=1;
                     COMMIT;",
                )
                .map_err(storage_error)?;
        }
        set_mode(path, 0o600)?;

        Ok(Self { connection })
    }

    pub fn local_model_settings(&self) -> Result<LocalModelSettings, String> {
        let endpoint = self
            .connection
            .query_row(
                "SELECT value FROM global_settings WHERE key='local_model_endpoint'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .unwrap_or_else(|| LocalModelSettings::default().endpoint);
        validate_local_endpoint(&endpoint)?;
        Ok(LocalModelSettings { endpoint })
    }

    pub fn set_local_model_settings(&self, settings: &LocalModelSettings) -> Result<(), String> {
        validate_local_endpoint(&settings.endpoint)?;
        self.connection
            .execute(
                "INSERT INTO global_settings(key, value) VALUES ('local_model_endpoint', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [&settings.endpoint],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn preferences_for(&self, app_id: &str) -> Result<AiPreferences, String> {
        validate_app_id(app_id)?;
        let stored = self
            .connection
            .query_row(
                "SELECT writing_style, use_shared_memory FROM app_preferences WHERE app_id=?1",
                [app_id],
                |row| {
                    Ok(AiPreferences {
                        writing_style: row.get(0)?,
                        use_shared_memory: row.get::<_, i64>(1)? != 0,
                    })
                },
            )
            .optional()
            .map_err(storage_error)?;
        Ok(stored.unwrap_or_default())
    }

    pub fn save_preferences(
        &self,
        app_id: &str,
        preferences: &AiPreferences,
    ) -> Result<(), String> {
        validate_app_id(app_id)?;
        if preferences.writing_style.chars().count() > MAX_MEMORY_CHARS {
            return Err(format!(
                "Writing guidance must be {MAX_MEMORY_CHARS} characters or fewer"
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.connection
            .execute(
                "INSERT INTO app_preferences(app_id, writing_style, use_shared_memory, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(app_id) DO UPDATE SET
                   writing_style=excluded.writing_style,
                   use_shared_memory=excluded.use_shared_memory,
                   updated_at=excluded.updated_at",
                params![
                    app_id,
                    preferences.writing_style.trim(),
                    if preferences.use_shared_memory {
                        1_i64
                    } else {
                        0_i64
                    },
                    now
                ],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    /// Return explicit user-saved shared memories and memories for one app.
    pub fn memories_for(
        &self,
        app_id: &str,
        include_shared: bool,
    ) -> Result<Vec<AiMemory>, String> {
        validate_app_id(app_id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, scope, content FROM memories
                 WHERE scope=?1 OR (?2 AND scope='*')
                 ORDER BY created_at DESC, id DESC LIMIT 8",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![app_id, include_shared], |row| {
                Ok(AiMemory {
                    id: row.get(0)?,
                    scope: row.get(1)?,
                    content: row.get(2)?,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    /// Return shared and app-specific memories for the app's settings UI.
    pub fn memories_for_settings(&self, app_id: &str) -> Result<Vec<AiMemory>, String> {
        validate_app_id(app_id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, scope, content FROM memories
                 WHERE scope='*' OR scope=?1
                 ORDER BY scope, created_at DESC, id DESC",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([app_id], |row| {
                Ok(AiMemory {
                    id: row.get(0)?,
                    scope: row.get(1)?,
                    content: row.get(2)?,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    /// Save a memory only after an explicit user action in an app's memory UI.
    pub fn save_memory(&self, scope: &str, content: &str) -> Result<i64, String> {
        if scope != "*" {
            validate_app_id(scope)?;
        }
        let content = content.trim();
        if content.is_empty() || content.chars().count() > MAX_MEMORY_CHARS {
            return Err(format!(
                "A saved memory must contain 1 to {MAX_MEMORY_CHARS} characters"
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.connection
            .execute(
                "INSERT INTO memories(scope, content, created_at) VALUES (?1, ?2, ?3)",
                params![scope, content, now],
            )
            .map_err(storage_error)?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn delete_memory(&self, id: i64) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM memories WHERE id=?1", [id])
            .map_err(storage_error)?;
        Ok(())
    }
}

fn validate_app_id(app_id: &str) -> Result<(), String> {
    if app_id.is_empty()
        || app_id.len() > 160
        || !app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err("Invalid AI app identifier".into());
    }
    Ok(())
}

fn validate_local_endpoint(endpoint: &str) -> Result<(), String> {
    let url = Url::parse(endpoint).map_err(|_| "Invalid local AI endpoint".to_string())?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || url
            .host()
            .and_then(|host| match host {
                url::Host::Ipv4(address) => Some(address.is_loopback()),
                url::Host::Ipv6(address) => Some(address.is_loopback()),
                url::Host::Domain(_) => None,
            })
            .unwrap_or(false);
    if url.scheme() != "http"
        || !loopback
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("The AI endpoint must be an HTTP loopback URL with no credentials".into());
    }
    Ok(())
}

fn default_database_path() -> Result<PathBuf, String> {
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| {
            "Could not locate the user's data directory for the AI database".to_string()
        })?;
    Ok(base.join("mseries-ai").join("assistant.sqlite3"))
}

fn ensure_private_database_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("The AI database must not be a symbolic link".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(file) => drop(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(storage_error(error)),
            }
        }
        Err(error) => return Err(storage_error(error)),
    }
    set_mode(path, 0o600)
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(storage_error)?;
    if metadata.file_type().is_symlink() {
        return Err("The AI database directory and file must not be symbolic links".into());
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions).map_err(storage_error)
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("AI database error: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct TestDatabase {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let directory =
                env::temp_dir().join(format!("mseries-ai-test-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&directory).expect("create isolated test directory");
            let path = directory.join("assistant.sqlite3");
            Self { directory, path }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn database_is_private_and_preferences_round_trip_by_app() {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).expect("open private test database");

        assert_eq!(
            fs::metadata(&database.directory)
                .expect("stat database directory")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&database.path)
                .expect("stat database file")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            store
                .preferences_for("messages")
                .expect("default preferences")
                .writing_style,
            AiPreferences::default().writing_style
        );

        let preferences = AiPreferences {
            writing_style: "Warm, direct, and brief".into(),
            use_shared_memory: false,
        };
        store
            .save_preferences("messages", &preferences)
            .expect("save app preferences");
        let loaded = store
            .preferences_for("messages")
            .expect("reload app preferences");
        assert_eq!(loaded.writing_style, preferences.writing_style);
        assert!(!loaded.use_shared_memory);
        assert_eq!(
            store
                .preferences_for("other.app")
                .expect("other app defaults")
                .writing_style,
            AiPreferences::default().writing_style
        );
    }

    #[test]
    fn memories_are_filtered_by_app_and_explicit_shared_consent() {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).expect("open test database");
        let shared = store
            .save_memory("*", "Use my first name")
            .expect("save shared");
        store
            .save_memory("messages", "Keep replies concise")
            .expect("save app memory");
        store
            .save_memory("other.app", "Unrelated preference")
            .expect("save other app memory");

        let local_only = store
            .memories_for("messages", false)
            .expect("load app-only memories");
        assert_eq!(local_only.len(), 1);
        assert_eq!(local_only[0].content, "Keep replies concise");

        let with_shared = store
            .memories_for("messages", true)
            .expect("load shared and app memories");
        assert_eq!(with_shared.len(), 2);
        assert!(with_shared.iter().any(|memory| memory.scope == "*"));
        assert!(
            !with_shared
                .iter()
                .any(|memory| memory.content == "Unrelated preference")
        );

        store.delete_memory(shared).expect("delete saved memory");
        assert!(
            !store
                .memories_for("messages", true)
                .expect("reload memories")
                .iter()
                .any(|memory| memory.scope == "*")
        );
    }

    #[test]
    fn endpoint_requires_http_loopback_without_credentials_or_url_extras() {
        for endpoint in [
            "http://localhost:8766/v1",
            "http://127.0.0.1:8766/v1",
            "http://[::1]:8766/v1",
        ] {
            assert!(validate_local_endpoint(endpoint).is_ok(), "{endpoint}");
        }
        for endpoint in [
            "https://127.0.0.1/v1",
            "http://example.com/v1",
            "http://user@127.0.0.1/v1",
            "http://127.0.0.1/v1?token=secret",
            "http://127.0.0.1/v1#fragment",
        ] {
            assert!(validate_local_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }
}
