//! The owner's personal knowledge: contacts and notes (profile, writing style, ongoing topics, important facts).
//!
//! Everything here is written only by the owner (the `mseries-reply memory` command today, a Settings panel later) and
//! lives in the same owner-only database as the preferences and free-text memories. Text is stored as single clean lines:
//! control characters and bidi overrides are removed and whitespace is collapsed on the way in, and again when it is
//! rendered into a prompt, so a record cannot carry line structure into a prompt.

use crate::store::{AiMemory, AiStore, now_secs, storage_error};
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_TITLE_CHARS: usize = 80;
pub const MAX_RELATIONSHIP_CHARS: usize = 80;
pub const MAX_TONE_CHARS: usize = 160;
pub const MAX_TEXT_CHARS: usize = 600;
pub const MAX_LIST_ITEMS: usize = 12;
pub const MAX_LIST_ITEM_CHARS: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoteKind {
    /// Who the owner is. Always considered.
    Profile,
    /// How the owner writes. Always considered; one linked to a contact only for that contact.
    Style,
    /// Something ongoing (a project, a plan, an idea). Considered when its keywords come up.
    Topic,
    /// An important fact. Considered when its keywords come up.
    Fact,
}

impl NoteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Profile => "profile",
            NoteKind::Style => "style",
            NoteKind::Topic => "topic",
            NoteKind::Fact => "fact",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "profile" => NoteKind::Profile,
            "style" => NoteKind::Style,
            "topic" => NoteKind::Topic,
            "fact" => NoteKind::Fact,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Contact {
    pub id: i64,
    pub name: String,
    /// Other names the conversation might be titled with, besides `name`.
    pub aliases: Vec<String>,
    pub relationship: String,
    pub tone: String,
    pub notes: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub id: i64,
    pub kind: NoteKind,
    pub title: String,
    pub keywords: Vec<String>,
    pub body: String,
    /// When set, the note is only used in conversations with that contact.
    pub contact_id: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default)]
pub struct ContactInput {
    pub name: String,
    pub aliases: Vec<String>,
    pub relationship: String,
    pub tone: String,
    pub notes: String,
}

#[derive(Clone, Debug, Default)]
pub struct ContactPatch {
    pub name: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub relationship: Option<String>,
    pub tone: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NoteInput {
    pub kind: NoteKind,
    pub title: String,
    pub keywords: Vec<String>,
    pub body: String,
    pub contact_id: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub keywords: Option<Vec<String>>,
    pub body: Option<String>,
    /// `Some(None)` unlinks the note from its contact.
    pub contact_id: Option<Option<i64>>,
}

/// Everything the owner has stored, for backup or moving to another machine.
#[derive(Debug, Serialize)]
pub struct Export {
    pub version: u32,
    pub contacts: Vec<Contact>,
    pub notes: Vec<Note>,
    pub memories: Vec<AiMemory>,
}

/// One line of clean text: no control or bidi characters, whitespace collapsed. Never longer than `max` characters;
/// longer input is an error, not silently cut.
pub fn clean_line(text: &str, max: usize, what: &str) -> Result<String, String> {
    let cleaned = one_line(text);
    if cleaned.chars().count() > max {
        return Err(format!("{what} must be {max} characters or fewer"));
    }
    Ok(cleaned)
}

/// Strips characters that could change how text is displayed or structured and collapses whitespace to single spaces.
pub fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .filter(|c| {
            !matches!(*c,
                '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}'..='\u{200f}'
                | '\u{2028}' | '\u{2029}' | '\u{feff}')
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clean_list(items: &[String], what: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let item = clean_line(item, MAX_LIST_ITEM_CHARS, what)?;
        if item.is_empty() || out.iter().any(|seen| seen.eq_ignore_ascii_case(&item)) {
            continue;
        }
        out.push(item);
    }
    if out.len() > MAX_LIST_ITEMS {
        return Err(format!("At most {MAX_LIST_ITEMS} {what} are allowed"));
    }
    Ok(out)
}

fn split_list(stored: &str) -> Vec<String> {
    stored
        .split('\n')
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn contact_from_row(row: &Row<'_>) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        name: row.get(1)?,
        aliases: split_list(&row.get::<_, String>(2)?),
        relationship: row.get(3)?,
        tone: row.get(4)?,
        notes: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

pub(crate) const CONTACT_COLUMNS: &str = "id, name, aliases, relationship, tone, notes, updated_at";
pub(crate) const NOTE_COLUMNS: &str = "id, kind, title, keywords, body, contact_id, updated_at";

pub(crate) fn note_from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    let kind: String = row.get(1)?;
    Ok(Note {
        id: row.get(0)?,
        // The table's CHECK keeps this to the four kinds; anything else is treated as a fact rather than a crash.
        kind: NoteKind::parse(&kind).unwrap_or(NoteKind::Fact),
        title: row.get(2)?,
        keywords: split_list(&row.get::<_, String>(3)?),
        body: row.get(4)?,
        contact_id: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn contact_error(error: rusqlite::Error, name: &str) -> String {
    if let rusqlite::Error::SqliteFailure(failure, _) = &error
        && failure.code == rusqlite::ErrorCode::ConstraintViolation
        && failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    {
        return format!("A contact named {name} already exists");
    }
    storage_error(error)
}

struct CleanContact {
    name: String,
    aliases: Vec<String>,
    relationship: String,
    tone: String,
    notes: String,
}

fn clean_contact(contact: &Contact) -> Result<CleanContact, String> {
    let name = clean_line(&contact.name, MAX_NAME_CHARS, "A contact name")?;
    if name.is_empty() {
        return Err("A contact needs a name".into());
    }
    let aliases = clean_list(&contact.aliases, "aliases")?;
    if aliases.join("\n").chars().count() > 400 {
        return Err("The aliases are too long altogether".into());
    }
    Ok(CleanContact {
        name,
        aliases,
        relationship: clean_line(
            &contact.relationship,
            MAX_RELATIONSHIP_CHARS,
            "The relationship",
        )?,
        tone: clean_line(&contact.tone, MAX_TONE_CHARS, "The tone")?,
        notes: clean_line(&contact.notes, MAX_TEXT_CHARS, "The notes")?,
    })
}

impl AiStore {
    pub fn add_contact(&self, input: &ContactInput) -> Result<i64, String> {
        let draft = Contact {
            id: 0,
            name: input.name.clone(),
            aliases: input.aliases.clone(),
            relationship: input.relationship.clone(),
            tone: input.tone.clone(),
            notes: input.notes.clone(),
            updated_at: 0,
        };
        let clean = clean_contact(&draft)?;
        let now = now_secs();
        self.connection
            .execute(
                "INSERT INTO contacts(name, aliases, relationship, tone, notes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    clean.name,
                    clean.aliases.join("\n"),
                    clean.relationship,
                    clean.tone,
                    clean.notes,
                    now
                ],
            )
            .map_err(|error| contact_error(error, &clean.name))?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn contact(&self, id: i64) -> Result<Option<Contact>, String> {
        self.connection
            .query_row(
                &format!("SELECT {CONTACT_COLUMNS} FROM contacts WHERE id=?1"),
                [id],
                contact_from_row,
            )
            .optional()
            .map_err(storage_error)
    }

    pub fn contacts(&self) -> Result<Vec<Contact>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {CONTACT_COLUMNS} FROM contacts ORDER BY name COLLATE NOCASE"
            ))
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], contact_from_row)
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    /// The contact whose name or alias equals `name` (ignoring case), for `--contact NAME` style references.
    pub fn find_contact(&self, name: &str) -> Result<Option<Contact>, String> {
        let wanted = one_line(name).to_lowercase();
        Ok(self.contacts()?.into_iter().find(|contact| {
            contact.name.to_lowercase() == wanted
                || contact
                    .aliases
                    .iter()
                    .any(|alias| alias.to_lowercase() == wanted)
        }))
    }

    pub fn update_contact(&self, id: i64, patch: &ContactPatch) -> Result<(), String> {
        let mut contact = self
            .contact(id)?
            .ok_or_else(|| format!("There is no contact c{id}"))?;
        if let Some(name) = &patch.name {
            contact.name = name.clone();
        }
        if let Some(aliases) = &patch.aliases {
            contact.aliases = aliases.clone();
        }
        if let Some(relationship) = &patch.relationship {
            contact.relationship = relationship.clone();
        }
        if let Some(tone) = &patch.tone {
            contact.tone = tone.clone();
        }
        if let Some(notes) = &patch.notes {
            contact.notes = notes.clone();
        }
        let clean = clean_contact(&contact)?;
        self.connection
            .execute(
                "UPDATE contacts SET name=?2, aliases=?3, relationship=?4, tone=?5, notes=?6, updated_at=?7
                 WHERE id=?1",
                params![
                    id,
                    clean.name,
                    clean.aliases.join("\n"),
                    clean.relationship,
                    clean.tone,
                    clean.notes,
                    now_secs()
                ],
            )
            .map_err(|error| contact_error(error, &clean.name))?;
        Ok(())
    }

    /// Deletes the contact and every note linked to it. Returns whether there was one.
    pub fn delete_contact(&self, id: i64) -> Result<bool, String> {
        Ok(self
            .connection
            .execute("DELETE FROM contacts WHERE id=?1", [id])
            .map_err(storage_error)?
            > 0)
    }

    fn check_note(
        &self,
        kind: NoteKind,
        title: &str,
        keywords: &[String],
        body: &str,
        contact_id: Option<i64>,
    ) -> Result<(String, Vec<String>, String), String> {
        let title = clean_line(title, MAX_TITLE_CHARS, "The title")?;
        let keywords = clean_list(keywords, "keywords")?;
        if keywords.join("\n").chars().count() > 300 {
            return Err("The keywords are too long altogether".into());
        }
        let body = clean_line(body, MAX_TEXT_CHARS, "The text")?;
        if body.is_empty() {
            return Err("A note needs some text".into());
        }
        if let Some(contact) = contact_id {
            if kind == NoteKind::Profile {
                return Err(
                    "A profile note describes the owner and cannot belong to a contact".into(),
                );
            }
            if self.contact(contact)?.is_none() {
                return Err(format!("There is no contact c{contact}"));
            }
        }
        Ok((title, keywords, body))
    }

    pub fn add_note(&self, input: &NoteInput) -> Result<i64, String> {
        let (title, keywords, body) = self.check_note(
            input.kind,
            &input.title,
            &input.keywords,
            &input.body,
            input.contact_id,
        )?;
        let now = now_secs();
        self.connection
            .execute(
                "INSERT INTO notes(kind, title, keywords, body, contact_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    input.kind.as_str(),
                    title,
                    keywords.join("\n"),
                    body,
                    input.contact_id,
                    now
                ],
            )
            .map_err(storage_error)?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn note(&self, id: i64) -> Result<Option<Note>, String> {
        self.connection
            .query_row(
                &format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id=?1"),
                [id],
                note_from_row,
            )
            .optional()
            .map_err(storage_error)
    }

    pub fn notes(&self, kind: Option<NoteKind>) -> Result<Vec<Note>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {NOTE_COLUMNS} FROM notes WHERE (?1 IS NULL OR kind=?1)
                 ORDER BY kind, updated_at DESC, id DESC"
            ))
            .map_err(storage_error)?;
        let rows = statement
            .query_map([kind.map(NoteKind::as_str)], note_from_row)
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    pub fn update_note(&self, id: i64, patch: &NotePatch) -> Result<(), String> {
        let mut note = self
            .note(id)?
            .ok_or_else(|| format!("There is no note n{id}"))?;
        if let Some(title) = &patch.title {
            note.title = title.clone();
        }
        if let Some(keywords) = &patch.keywords {
            note.keywords = keywords.clone();
        }
        if let Some(body) = &patch.body {
            note.body = body.clone();
        }
        if let Some(contact_id) = patch.contact_id {
            note.contact_id = contact_id;
        }
        let (title, keywords, body) = self.check_note(
            note.kind,
            &note.title,
            &note.keywords,
            &note.body,
            note.contact_id,
        )?;
        self.connection
            .execute(
                "UPDATE notes SET title=?2, keywords=?3, body=?4, contact_id=?5, updated_at=?6 WHERE id=?1",
                params![id, title, keywords.join("\n"), body, note.contact_id, now_secs()],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn delete_note(&self, id: i64) -> Result<bool, String> {
        Ok(self
            .connection
            .execute("DELETE FROM notes WHERE id=?1", [id])
            .map_err(storage_error)?
            > 0)
    }

    pub fn export_all(&self) -> Result<Export, String> {
        Ok(Export {
            version: 1,
            contacts: self.contacts()?,
            notes: self.notes(None)?,
            memories: self.all_memories()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDatabase;

    fn contact(name: &str) -> ContactInput {
        ContactInput {
            name: name.into(),
            ..ContactInput::default()
        }
    }

    #[test]
    fn contacts_and_notes_round_trip_and_clean_their_text() {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).unwrap();
        let sam = store
            .add_contact(&ContactInput {
                name: "  Sam\n Example ".into(),
                aliases: vec!["Sammy".into(), "sammy".into(), "S. E.\u{202e}".into()],
                relationship: "brother".into(),
                tone: "casual, teasing".into(),
                notes: "Lives in Denver.\nLikes\tbikes.".into(),
            })
            .unwrap();
        let loaded = store.contact(sam).unwrap().unwrap();
        assert_eq!(loaded.name, "Sam Example");
        assert_eq!(loaded.aliases, vec!["Sammy", "S. E."]);
        assert_eq!(loaded.notes, "Lives in Denver. Likes bikes.");

        let note = store
            .add_note(&NoteInput {
                kind: NoteKind::Topic,
                title: "Bike trip".into(),
                keywords: vec!["bike".into(), "trip".into()],
                body: "Planning a ride in May.".into(),
                contact_id: Some(sam),
            })
            .unwrap();
        assert_eq!(store.notes(Some(NoteKind::Topic)).unwrap().len(), 1);
        assert!(store.notes(Some(NoteKind::Fact)).unwrap().is_empty());

        store
            .update_note(
                note,
                &NotePatch {
                    body: Some("Ride moved to June.".into()),
                    contact_id: Some(None),
                    ..NotePatch::default()
                },
            )
            .unwrap();
        let edited = store.note(note).unwrap().unwrap();
        assert_eq!(edited.body, "Ride moved to June.");
        assert_eq!(edited.contact_id, None);
        assert_eq!(edited.keywords, vec!["bike", "trip"]);

        let export = serde_json::to_string(&store.export_all().unwrap()).unwrap();
        assert!(export.contains("Sam Example") && export.contains("Ride moved to June."));
    }

    #[test]
    fn validation_rejects_bad_input_and_duplicate_names() {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).unwrap();
        assert!(store.add_contact(&contact("   ")).is_err());
        assert!(store.add_contact(&contact(&"x".repeat(81))).is_err());
        store.add_contact(&contact("Alex")).unwrap();
        assert!(
            store
                .add_contact(&contact("ALEX"))
                .unwrap_err()
                .contains("already exists")
        );
        let note = |kind, body: &str, contact_id| NoteInput {
            kind,
            title: String::new(),
            keywords: vec![],
            body: body.into(),
            contact_id,
        };
        assert!(store.add_note(&note(NoteKind::Fact, " ", None)).is_err());
        assert!(
            store
                .add_note(&note(NoteKind::Fact, &"y".repeat(601), None))
                .is_err()
        );
        assert!(
            store
                .add_note(&note(NoteKind::Fact, "ok", Some(999)))
                .is_err()
        );
        let alex = store.find_contact("alex").unwrap().unwrap();
        assert!(
            store
                .add_note(&note(NoteKind::Profile, "I am Pat", Some(alex.id)))
                .is_err()
        );
    }

    #[test]
    fn deleting_a_contact_deletes_its_notes() {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).unwrap();
        let id = store.add_contact(&contact("Alex")).unwrap();
        store
            .add_note(&NoteInput {
                kind: NoteKind::Fact,
                title: String::new(),
                keywords: vec![],
                body: "Allergic to nuts".into(),
                contact_id: Some(id),
            })
            .unwrap();
        assert!(store.delete_contact(id).unwrap());
        assert!(store.notes(None).unwrap().is_empty());
        assert!(!store.delete_contact(id).unwrap());
    }
}
