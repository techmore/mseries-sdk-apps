//! Read-only access to Signal Desktop's SQLCipher database.
//!
//! The live files are never opened: `db.sqlite` (and its `-wal`) are copied into a
//! private temporary directory and the copy is read. Only counts and the
//! conversion result leave this module; nothing is printed.

use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct SigConversation {
    pub id: String,
    pub is_group: bool,
    pub e164: Option<String>,
    pub service_id: Option<String>,
    pub group_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SigMessage {
    pub conversation_id: String,
    pub sent_at: i64,
    pub outgoing: bool,
    pub body: String,
    /// Service ids of mentions, in the order their U+FFFC placeholders appear.
    pub mentions: Vec<String>,
    pub attachment_types: Vec<String>,
    pub sticker: bool,
    pub shared_contact: bool,
    pub deleted: bool,
    pub sender_service_id: Option<String>,
    pub sender_e164: Option<String>,
}

#[derive(Debug, Default)]
pub struct SignalData {
    pub conversations: Vec<SigConversation>,
    pub messages: Vec<SigMessage>,
    /// Why messages were left out, by reason.
    pub skipped: BTreeMap<&'static str, usize>,
}

/// A private copy of the Signal database that is removed when dropped.
pub struct DatabaseCopy {
    dir: PathBuf,
    pub db: PathBuf,
}

impl DatabaseCopy {
    pub fn make(signal_dir: &Path, scratch_parent: &Path) -> Result<Self, String> {
        let source = signal_dir.join("sql").join("db.sqlite");
        if !source.is_file() {
            return Err(format!("no Signal database at {}", source.display()));
        }
        let dir = scratch_parent.join(format!("mseries-signal-import-{}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| format!("could not create a scratch directory: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        }
        let copy = DatabaseCopy {
            db: dir.join("db.sqlite"),
            dir,
        };
        // The write-ahead log first: if Signal checkpoints between the two copies
        // the worst case is a few of the newest messages missing, not corruption.
        let wal = signal_dir.join("sql").join("db.sqlite-wal");
        if wal.is_file() {
            fs::copy(&wal, copy.dir.join("db.sqlite-wal")).map_err(|e| format!("could not copy the Signal log: {e}"))?;
        }
        fs::copy(&source, &copy.db).map_err(|e| format!("could not copy the Signal database: {e}"))?;
        Ok(copy)
    }
}

impl Drop for DatabaseCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub fn open_encrypted(path: &Path, key_hex: &str) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("could not open the Signal database copy: {e}"))?;
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
        .map_err(|e| format!("could not apply the Signal key: {e}"))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .map_err(|_| "the Signal database did not open with its key (unsupported SQLCipher settings?)".to_string())?;
    Ok(conn)
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).map_err(|e| e.to_string())?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?;
    names.collect::<Result<HashSet<_>, _>>().map_err(|e| e.to_string())
}

fn text_cell(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8(t.to_vec()).ok(),
    }
}

/// Selects the wanted columns that exist and yields each row as name → text.
fn read_rows(
    conn: &Connection,
    table: &str,
    wanted: &[&str],
    mut each: impl FnMut(HashMap<&str, String>) -> Result<(), String>,
) -> Result<(), String> {
    let present = table_columns(conn, table)?;
    let columns: Vec<&str> = wanted.iter().copied().filter(|c| present.contains(*c)).collect();
    if columns.is_empty() {
        return Err(format!("the Signal {table} table has none of the expected columns"));
    }
    let sql = format!(
        "SELECT {} FROM {table}",
        columns.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(",")
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut map = HashMap::new();
        for (i, column) in columns.iter().enumerate() {
            if let Some(text) = text_cell(row.get_ref(i).map_err(|e| e.to_string())?) {
                map.insert(*column, text);
            }
        }
        each(map)?;
    }
    Ok(())
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

fn json_str(json: &Value, field: &str) -> Option<String> {
    nonempty(json.get(field).and_then(Value::as_str))
}

fn json_i64(json: &Value, field: &str) -> Option<i64> {
    let v = json.get(field)?;
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn json_truthy(json: &Value, field: &str) -> bool {
    match json.get(field) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

fn join_names(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    }
}

fn read_conversations(conn: &Connection) -> Result<Vec<SigConversation>, String> {
    let mut out = Vec::new();
    read_rows(
        conn,
        "conversations",
        &[
            "id",
            "type",
            "e164",
            "serviceId",
            "uuid",
            "groupId",
            "name",
            "profileName",
            "profileFamilyName",
            "json",
        ],
        |row| {
            let json: Value = row.get("json").and_then(|j| serde_json::from_str(j).ok()).unwrap_or(Value::Null);
            let id = match nonempty(row.get("id").map(String::as_str)) {
                Some(id) => id,
                None => return Ok(()),
            };
            let kind = nonempty(row.get("type").map(String::as_str))
                .or_else(|| json_str(&json, "type"))
                .unwrap_or_default();
            let e164 = nonempty(row.get("e164").map(String::as_str))
                .or_else(|| json_str(&json, "e164"))
                .map(|n| {
                    // Numbers are stored with their leading plus; restore it if a
                    // numeric column affinity ever stripped it.
                    if n.bytes().all(|b| b.is_ascii_digit()) {
                        format!("+{n}")
                    } else {
                        n
                    }
                });
            let service_id = nonempty(row.get("serviceId").map(String::as_str))
                .or_else(|| nonempty(row.get("uuid").map(String::as_str)))
                .or_else(|| json_str(&json, "serviceId"))
                .or_else(|| json_str(&json, "uuid"));
            let group_id = nonempty(row.get("groupId").map(String::as_str)).or_else(|| json_str(&json, "groupId"));
            let name = json_str(&json, "nicknameGivenName")
                .or_else(|| join_names(json_str(&json, "systemGivenName"), json_str(&json, "systemFamilyName")))
                .or_else(|| nonempty(row.get("name").map(String::as_str)))
                .or_else(|| json_str(&json, "name"))
                .or_else(|| {
                    join_names(
                        nonempty(row.get("profileName").map(String::as_str)).or_else(|| json_str(&json, "profileName")),
                        nonempty(row.get("profileFamilyName").map(String::as_str)).or_else(|| json_str(&json, "profileFamilyName")),
                    )
                })
                .or_else(|| e164.clone())
                .unwrap_or_default();
            out.push(SigConversation {
                id,
                is_group: kind == "group",
                e164,
                service_id,
                group_id,
                name,
            });
            Ok(())
        },
    )?;
    Ok(out)
}

fn read_messages(conn: &Connection, data: &mut SignalData) -> Result<(), String> {
    let mut messages = Vec::new();
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    read_rows(
        conn,
        "messages",
        &[
            "conversationId",
            "json",
            "sent_at",
            "type",
            "body",
            "source",
            "sourceServiceId",
            "expireTimer",
            "isViewOnce",
            "storyId",
            "isStory",
            "isErased",
        ],
        |row| {
            let json: Value = row.get("json").and_then(|j| serde_json::from_str(j).ok()).unwrap_or(Value::Null);
            let mut note = |reason: &'static str| *skipped.entry(reason).or_insert(0) += 1;
            let kind = nonempty(row.get("type").map(String::as_str))
                .or_else(|| json_str(&json, "type"))
                .unwrap_or_default();
            let outgoing = match kind.as_str() {
                "outgoing" => true,
                "incoming" => false,
                _ => {
                    note("not a chat message");
                    return Ok(());
                }
            };
            let conversation_id =
                match nonempty(row.get("conversationId").map(String::as_str)).or_else(|| json_str(&json, "conversationId")) {
                    Some(id) => id,
                    None => {
                        note("no conversation");
                        return Ok(());
                    }
                };
            let column_flag = |name: &str| row.get(name).map(|v| !v.is_empty() && v != "0" && v != "false").unwrap_or(false);
            if json_truthy(&json, "isStory") || column_flag("isStory") || json_str(&json, "storyId").is_some() || column_flag("storyId") {
                {
                    note("story");
                    return Ok(());
                }
            }
            if json_truthy(&json, "isViewOnce") || column_flag("isViewOnce") {
                {
                    note("view-once");
                    return Ok(());
                }
            }
            let expire = json_i64(&json, "expireTimer")
                .or_else(|| row.get("expireTimer").and_then(|v| v.parse().ok()))
                .unwrap_or(0);
            if expire > 0 {
                {
                    note("disappearing");
                    return Ok(());
                }
            }
            let sent_at = match row
                .get("sent_at")
                .and_then(|v| v.parse::<i64>().ok())
                .or_else(|| json_i64(&json, "sent_at"))
                .or_else(|| json_i64(&json, "timestamp"))
            {
                Some(t) if t > 0 => t,
                _ => {
                    note("no timestamp");
                    return Ok(());
                }
            };
            let deleted = json_truthy(&json, "deletedForEveryone") || json_truthy(&json, "isErased") || column_flag("isErased");
            let body = row
                .get("body")
                .cloned()
                .or_else(|| json.get("body").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            let mut mentions: Vec<(i64, String)> = json
                .get("bodyRanges")
                .and_then(Value::as_array)
                .map(|ranges| {
                    ranges
                        .iter()
                        .filter_map(|r| {
                            Some((
                                r.get("start")?.as_i64()?,
                                json_str(r, "mentionAci").or_else(|| json_str(r, "mentionUuid"))?,
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            mentions.sort_by_key(|(start, _)| *start);
            let attachment_types: Vec<String> = json
                .get("attachments")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|x| json_str(x, "contentType").unwrap_or_else(|| "file".into()))
                        .collect()
                })
                .unwrap_or_default();
            let sticker = json.get("sticker").map(|s| !s.is_null()).unwrap_or(false);
            let shared_contact = json
                .get("contact")
                .and_then(Value::as_array)
                .map(|c| !c.is_empty())
                .unwrap_or(false);
            if !deleted && body.trim().is_empty() && attachment_types.is_empty() && !sticker && !shared_contact {
                {
                    note("no content");
                    return Ok(());
                }
            }
            messages.push(SigMessage {
                conversation_id,
                sent_at,
                outgoing,
                body,
                mentions: mentions.into_iter().map(|(_, aci)| aci).collect(),
                attachment_types,
                sticker,
                shared_contact,
                deleted,
                sender_service_id: json_str(&json, "sourceServiceId").or_else(|| nonempty(row.get("sourceServiceId").map(String::as_str))),
                sender_e164: json_str(&json, "source").or_else(|| nonempty(row.get("source").map(String::as_str))),
            });
            Ok(())
        },
    )?;
    data.messages = messages;
    data.skipped = skipped;
    Ok(())
}

pub fn read_all(conn: &Connection) -> Result<SignalData, String> {
    let mut data = SignalData {
        conversations: read_conversations(conn)?,
        ..SignalData::default()
    };
    read_messages(conn, &mut data)?;
    Ok(data)
}
