//! Picks the few personal records that matter for one reply, within a hard character budget, locally.
//!
//! Inputs are the app id, the conversation header (the contact) and the last few messages. Output is a short block of
//! single-line entries for the prompt plus counts of what was used. Order of importance when the budget runs out:
//! the matched contact, who the owner is, how they write, ongoing topics, important facts, other saved memories.
//!
//! Nothing here reaches the network, and the block is *data*: every entry is one clean line starting with `- `, under
//! one of a fixed set of headings, so a record cannot add a heading, a role marker or a line of its own.

use crate::persona::{Contact, NOTE_COLUMNS, Note, NoteKind, note_from_row, one_line};
use crate::store::{AiPreferences, AiStore, storage_error};
use rusqlite::params;
use serde::Serialize;

/// Hard limit for everything personal that goes into one prompt, headings included. About 350 tokens.
pub const DEFAULT_BUDGET_CHARS: usize = 1_400;
const MAX_HAYSTACK_CHARS: usize = 4_000;
const MIN_TRUNCATED_CHARS: usize = 40;

const MAX_PROFILE: usize = 4;
const MAX_STYLE: usize = 4;
const MAX_TOPICS: usize = 3;
const MAX_FACTS: usize = 3;
const MAX_MEMORIES: usize = 3;
const MAX_NOTE_ROWS: i64 = 500;

/// Section headings in the order they appear in the prompt.
pub(crate) const HEADINGS: [&str; 6] = [
    "About the owner:",
    "How the owner writes:",
    "The person they are talking to:",
    "Ongoing topics:",
    "Facts that may matter:",
    "Other saved memories:",
];
const PROFILE: usize = 0;
const STYLE: usize = 1;
const CONTACT: usize = 2;
const TOPIC: usize = 3;
const FACT: usize = 4;
const MEMORY: usize = 5;

pub struct RetrievalQuery<'a> {
    pub app_id: &'a str,
    /// The conversation title or header, matched against contact names and aliases.
    pub contact: Option<&'a str>,
    /// The newest message texts, used to find matching topics and facts.
    pub recent: &'a [&'a str],
    pub budget_chars: usize,
    /// Asked about every name of a contact before any of its record is read; `false` leaves that contact out entirely.
    /// Smart Reply passes its rules here so a contact the owner blocked is never read, even under another name.
    pub name_allowed: &'a dyn Fn(&str) -> bool,
}

impl<'a> RetrievalQuery<'a> {
    pub fn new(app_id: &'a str) -> Self {
        Self {
            app_id,
            contact: None,
            recent: &[],
            budget_chars: DEFAULT_BUDGET_CHARS,
            name_allowed: &|_| true,
        }
    }
}

/// How many records of each kind went into a prompt. Counts only: this is what the audit may record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RetrievalCounts {
    pub contact: usize,
    pub profile: usize,
    pub style: usize,
    pub topics: usize,
    pub facts: usize,
    pub memories: usize,
    /// Characters of the block that was added to the prompt.
    pub chars: usize,
}

/// The personal part of one prompt. May be empty.
#[derive(Clone, Debug, Default)]
pub struct PersonalContext {
    block: String,
    pub counts: RetrievalCounts,
}

impl PersonalContext {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The lines to put under the notes heading of the system prompt (empty when nothing was relevant).
    pub fn block(&self) -> &str {
        &self.block
    }
}

// ----- matching helpers -------------------------------------------------------------------------------------------

/// Lowercase word tokens with a light plural strip, so "weddings" meets "wedding".
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            if word.chars().count() > 3 && word.ends_with('s') && !word.ends_with("ss") {
                word[..word.len() - 1].to_string()
            } else {
                word.to_string()
            }
        })
        .collect()
}

/// Space-padded token string so a phrase can be found only on word boundaries.
fn padded(text: &str) -> String {
    let words = tokens(text);
    if words.is_empty() {
        String::new()
    } else {
        format!(" {} ", words.join(" "))
    }
}

fn found(haystack: &str, phrase: &str) -> bool {
    let phrase = padded(phrase);
    !phrase.is_empty() && haystack.contains(&phrase)
}

struct ContactKey {
    id: i64,
    names: Vec<String>,
}

/// The one contact the header is about, or none when it is unknown or ambiguous (two different people match).
fn match_contact(header: &str, keys: &[ContactKey], allowed: &dyn Fn(&str) -> bool) -> Option<i64> {
    let header = padded(header);
    if header.is_empty() {
        return None;
    }
    // (score, matched phrase, id)
    let mut hits: Vec<(usize, String, i64)> = Vec::new();
    for key in keys {
        let mut best: Option<(usize, String)> = None;
        for name in &key.names {
            let phrase = padded(name);
            if phrase.is_empty() || !header.contains(&phrase) {
                continue;
            }
            let score = phrase.len() + if phrase == header { 1_000 } else { 0 };
            if best.as_ref().is_none_or(|(top, _)| score > *top) {
                best = Some((score, phrase));
            }
        }
        let Some((score, phrase)) = best else {
            continue;
        };
        if key.names.iter().all(|name| allowed(name)) {
            hits.push((score, phrase, key.id));
        }
    }
    let (_, best_phrase, best_id) = hits.iter().max_by_key(|(score, _, _)| *score)?.clone();
    // Others may only be sub-names of the best match ("Sam" next to "Sam Example"); a different person is ambiguity.
    hits.iter()
        .all(|(_, phrase, id)| *id == best_id || best_phrase.contains(phrase.as_str()))
        .then_some(best_id)
}

// ----- rendering --------------------------------------------------------------------------------------------------

fn contact_line(contact: &Contact) -> String {
    let mut line = one_line(&contact.name);
    if !contact.relationship.is_empty() {
        line.push_str(&format!(" ({})", one_line(&contact.relationship)));
    }
    line.push('.');
    if !contact.tone.is_empty() {
        line.push_str(&format!(" Tone: {}.", one_line(&contact.tone)));
    }
    if !contact.notes.is_empty() {
        line.push(' ');
        line.push_str(&one_line(&contact.notes));
    }
    line
}

fn note_line(note: &Note) -> String {
    let body = one_line(&note.body);
    let title = one_line(&note.title);
    if title.is_empty() || note.kind == NoteKind::Style || note.kind == NoteKind::Profile {
        body
    } else {
        format!("{title}: {body}")
    }
}

/// Fits entries (given in priority order) into `budget` characters and renders them in heading order.
/// Returns the block and how many entries each section kept.
fn fit(entries: Vec<(usize, String)>, budget: usize) -> (String, [usize; 6]) {
    let mut used = 0usize;
    let mut kept: Vec<(usize, String)> = Vec::new();
    let mut started = [false; 6];
    for (section, text) in entries {
        let text = one_line(&text);
        if text.is_empty() {
            continue;
        }
        // A line costs "- " plus the newline; a section's first line also pays for its heading and newline.
        let overhead = 3 + if started[section] {
            0
        } else {
            HEADINGS[section].chars().count() + 1
        };
        let Some(room) = budget.checked_sub(used + overhead).filter(|room| *room > 0) else {
            continue;
        };
        let length = text.chars().count();
        let text = if length <= room {
            text
        } else if room >= MIN_TRUNCATED_CHARS {
            let mut cut: String = text.chars().take(room - 1).collect();
            cut.push('…');
            cut
        } else {
            continue;
        };
        used += overhead + text.chars().count();
        started[section] = true;
        kept.push((section, text));
    }

    let mut counts = [0usize; 6];
    let mut block = String::new();
    for section in 0..HEADINGS.len() {
        let lines: Vec<&String> = kept
            .iter()
            .filter(|(s, _)| *s == section)
            .map(|(_, text)| text)
            .collect();
        if lines.is_empty() {
            continue;
        }
        counts[section] = lines.len();
        block.push_str(HEADINGS[section]);
        block.push('\n');
        for line in lines {
            block.push_str("- ");
            block.push_str(line);
            block.push('\n');
        }
    }
    (block.trim_end().to_string(), counts)
}

// ----- retrieval --------------------------------------------------------------------------------------------------

impl AiStore {
    pub fn retrieve(&self, query: &RetrievalQuery<'_>) -> Result<PersonalContext, String> {
        // 1. Which contact is this conversation with? Only names are read to find out.
        let contact = match query.contact.filter(|header| !header.trim().is_empty()) {
            Some(header) => {
                let keys = self.contact_keys()?;
                match_contact(header, &keys, query.name_allowed)
                    .map(|id| self.contact(id))
                    .transpose()?
                    .flatten()
            }
            None => None,
        };
        let contact_id = contact.as_ref().map(|contact| contact.id);

        // 2. Notes: global ones, plus those linked to the matched contact. Notes of any other contact are not read.
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {NOTE_COLUMNS} FROM notes
                 WHERE contact_id IS NULL OR contact_id=?1
                 ORDER BY updated_at DESC, id DESC LIMIT ?2"
            ))
            .map_err(storage_error)?;
        let notes = statement
            .query_map(params![contact_id, MAX_NOTE_ROWS], note_from_row)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;

        let mut haystack_text = String::new();
        if let Some(header) = query.contact {
            haystack_text.push_str(header);
        }
        for text in query.recent {
            haystack_text.push(' ');
            haystack_text.push_str(text);
        }
        let haystack_text: String = haystack_text.chars().take(MAX_HAYSTACK_CHARS).collect();
        let haystack = padded(&haystack_text);

        let mut entries: Vec<(usize, String)> = Vec::new();
        if let Some(contact) = &contact {
            entries.push((CONTACT, contact_line(contact)));
        }

        let of_kind = |kind: NoteKind| notes.iter().filter(move |note| note.kind == kind);
        entries.extend(
            of_kind(NoteKind::Profile)
                .take(MAX_PROFILE)
                .map(|note| (PROFILE, note_line(note))),
        );

        let default_style = AiPreferences::default().writing_style;
        let preferences = self.preferences_for(query.app_id)?;
        let app_style = preferences.writing_style.trim();
        if !app_style.is_empty() && app_style != default_style {
            entries.push((STYLE, app_style.to_string()));
        }
        // Style written for this contact comes before general style.
        let (linked, general): (Vec<&Note>, Vec<&Note>) =
            of_kind(NoteKind::Style).partition(|note| note.contact_id.is_some());
        entries.extend(
            linked
                .into_iter()
                .chain(general)
                .take(MAX_STYLE)
                .map(|note| (STYLE, note_line(note))),
        );

        for (kind, section, limit) in [
            (NoteKind::Topic, TOPIC, MAX_TOPICS),
            (NoteKind::Fact, FACT, MAX_FACTS),
        ] {
            let mut scored: Vec<(usize, &Note)> = of_kind(kind)
                .filter_map(|note| {
                    let phrases: Vec<&String> = if note.keywords.is_empty() {
                        vec![&note.title]
                    } else {
                        note.keywords.iter().collect()
                    };
                    let hits = phrases
                        .into_iter()
                        .filter(|phrase| found(&haystack, phrase))
                        .count();
                    let linked =
                        usize::from(note.contact_id.is_some() && note.contact_id == contact_id);
                    let score = hits * 2 + linked * 3;
                    (score > 0).then_some((score, note))
                })
                .collect();
            // Stable sort keeps the newest first among equal scores.
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            entries.extend(
                scored
                    .into_iter()
                    .take(limit)
                    .map(|(_, note)| (section, note_line(note))),
            );
        }

        for memory in self
            .memories_for(query.app_id, preferences.use_shared_memory)?
            .into_iter()
            .take(MAX_MEMORIES)
        {
            entries.push((MEMORY, memory.content));
        }

        let (block, kept) = fit(entries, query.budget_chars);
        let counts = RetrievalCounts {
            contact: kept[CONTACT],
            profile: kept[PROFILE],
            style: kept[STYLE],
            topics: kept[TOPIC],
            facts: kept[FACT],
            memories: kept[MEMORY],
            chars: block.chars().count(),
        };
        Ok(PersonalContext { block, counts })
    }

    /// Names and aliases only: what is needed to decide which contact a conversation is with.
    fn contact_keys(&self) -> Result<Vec<ContactKey>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, aliases FROM contacts")
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                let aliases: String = row.get(2)?;
                let mut names = vec![row.get::<_, String>(1)?];
                names.extend(
                    aliases
                        .split('\n')
                        .filter(|a| !a.is_empty())
                        .map(str::to_string),
                );
                Ok(ContactKey {
                    id: row.get(0)?,
                    names,
                })
            })
            .map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{ContactInput, NoteInput};
    use crate::test_support::TestDatabase;

    fn store() -> (TestDatabase, AiStore) {
        let database = TestDatabase::new();
        let store = AiStore::open_at(&database.path).unwrap();
        (database, store)
    }

    fn contact(store: &AiStore, name: &str, aliases: &[&str], notes: &str) -> i64 {
        store
            .add_contact(&ContactInput {
                name: name.into(),
                aliases: aliases.iter().map(|a| a.to_string()).collect(),
                relationship: "friend".into(),
                tone: "warm".into(),
                notes: notes.into(),
            })
            .unwrap()
    }

    fn note(
        store: &AiStore,
        kind: NoteKind,
        keywords: &[&str],
        body: &str,
        contact: Option<i64>,
    ) -> i64 {
        store
            .add_note(&NoteInput {
                kind,
                title: String::new(),
                keywords: keywords.iter().map(|k| k.to_string()).collect(),
                body: body.into(),
                contact_id: contact,
            })
            .unwrap()
    }

    fn query<'a>(contact: Option<&'a str>, recent: &'a [&'a str]) -> RetrievalQuery<'a> {
        RetrievalQuery {
            contact,
            recent,
            ..RetrievalQuery::new("chat.app")
        }
    }

    #[test]
    fn contact_is_matched_by_name_or_alias_on_word_boundaries() {
        let (_db, store) = store();
        contact(&store, "Sam Example", &["Sammy"], "Likes bikes.");
        for header in ["Sam Example", "sammy", "Chat with Sammy (mobile)"] {
            let got = store.retrieve(&query(Some(header), &[])).unwrap();
            assert_eq!(got.counts.contact, 1, "{header}");
            assert!(got.block().contains("Likes bikes."));
        }
        // "Samantha" is not "Sam", and a missing header reads no contact.
        contact(&store, "Sam", &[], "Other Sam.");
        for header in [Some("Samantha Jones"), None, Some("")] {
            assert_eq!(
                store.retrieve(&query(header, &[])).unwrap().counts.contact,
                0
            );
        }
    }

    #[test]
    fn two_different_people_in_the_header_are_ambiguous_but_a_sub_name_is_not() {
        let (_db, store) = store();
        contact(&store, "Sam", &[], "Short Sam.");
        contact(&store, "Alex", &[], "Alex notes.");
        contact(&store, "Sam Example", &[], "Full Sam.");
        let group = store.retrieve(&query(Some("Sam and Alex"), &[])).unwrap();
        assert_eq!(
            group.counts.contact, 0,
            "a group chat must not pick one person"
        );
        let full = store.retrieve(&query(Some("Sam Example"), &[])).unwrap();
        assert_eq!(full.counts.contact, 1);
        assert!(full.block().contains("Full Sam.") && !full.block().contains("Short Sam."));
    }

    #[test]
    fn topics_and_facts_need_a_keyword_hit_or_a_matching_contact() {
        let (_db, store) = store();
        let sam = contact(&store, "Sam", &[], "");
        let alex = contact(&store, "Alex", &[], "");
        note(
            &store,
            NoteKind::Topic,
            &["wedding"],
            "Planning the June wedding.",
            None,
        );
        note(
            &store,
            NoteKind::Topic,
            &["boat"],
            "Selling the boat.",
            None,
        );
        note(
            &store,
            NoteKind::Fact,
            &[],
            "Never linked, no keywords.",
            None,
        );
        note(&store, NoteKind::Fact, &[], "Sam owes me lunch.", Some(sam));
        note(
            &store,
            NoteKind::Fact,
            &[],
            "Alex is vegetarian.",
            Some(alex),
        );

        let got = store
            .retrieve(&query(Some("Sam"), &["Are the weddings still on?"]))
            .unwrap();
        let block = got.block();
        assert!(block.contains("June wedding"), "{block}");
        assert!(block.contains("Sam owes me lunch."));
        for absent in ["Selling the boat", "no keywords", "vegetarian"] {
            assert!(!block.contains(absent), "{absent} in {block}");
        }
        assert_eq!((got.counts.topics, got.counts.facts), (1, 1));
    }

    #[test]
    fn profile_and_style_are_always_considered_and_contact_style_wins_the_order() {
        let (_db, store) = store();
        let sam = contact(&store, "Sam", &[], "");
        note(
            &store,
            NoteKind::Profile,
            &[],
            "I run a small workshop.",
            None,
        );
        note(
            &store,
            NoteKind::Style,
            &[],
            "Short sentences, no emoji.",
            None,
        );
        note(
            &store,
            NoteKind::Style,
            &[],
            "With Sam I tease a lot.",
            Some(sam),
        );
        let with = store.retrieve(&query(Some("Sam"), &[])).unwrap();
        let block = with.block();
        assert!(block.find("tease").unwrap() < block.find("no emoji").unwrap());
        assert_eq!((with.counts.profile, with.counts.style), (1, 2));
        // Without the contact, its style note is not read.
        let without = store.retrieve(&query(Some("Someone Else"), &[])).unwrap();
        assert!(!without.block().contains("tease") && without.counts.style == 1);
    }

    #[test]
    fn block_never_exceeds_the_budget_and_keeps_the_contact_first() {
        let (_db, store) = store();
        contact(&store, "Sam", &[], &"Likes long walks. ".repeat(25));
        for index in 0..4 {
            note(
                &store,
                NoteKind::Profile,
                &[],
                &format!("Profile {index} {}", "p".repeat(500)),
                None,
            );
            note(
                &store,
                NoteKind::Style,
                &[],
                &format!("Style {index} {}", "s".repeat(500)),
                None,
            );
            note(
                &store,
                NoteKind::Topic,
                &["walks"],
                &format!("Topic {index} {}", "t".repeat(500)),
                None,
            );
            note(
                &store,
                NoteKind::Fact,
                &["walks"],
                &format!("Fact {index} {}", "f".repeat(500)),
                None,
            );
            store
                .save_memory("*", &format!("Memory {index} {}", "m".repeat(500)))
                .unwrap();
        }
        for budget in [0, 30, 120, 400, 1_400, 5_000] {
            let got = store
                .retrieve(&RetrievalQuery {
                    budget_chars: budget,
                    ..query(Some("Sam"), &["long walks today?"])
                })
                .unwrap();
            assert!(
                got.block().chars().count() <= budget,
                "budget {budget}: {}",
                got.block().chars().count()
            );
            assert_eq!(got.counts.chars, got.block().chars().count());
            if budget >= 400 {
                assert_eq!(
                    got.counts.contact, 1,
                    "the contact is the first thing kept at {budget}"
                );
                assert!(
                    got.block().starts_with("About the owner:")
                        || got.block().contains("The person they are talking to:")
                );
            }
        }
        let tight = store
            .retrieve(&RetrievalQuery {
                budget_chars: 400,
                ..query(Some("Sam"), &["long walks today?"])
            })
            .unwrap();
        assert_eq!(tight.counts.contact, 1);
        assert!(
            tight.block().contains('…'),
            "long entries are cut, not dropped"
        );
    }

    #[test]
    fn a_name_the_caller_disallows_leaves_the_contact_and_its_notes_unread() {
        let (_db, store) = store();
        let mother = contact(&store, "Mother", &["Mum"], "Secret mum notes.");
        note(
            &store,
            NoteKind::Fact,
            &[],
            "Mum's surgery is in May.",
            Some(mother),
        );
        let blocked = |name: &str| name != "Mum";
        let got = store
            .retrieve(&RetrievalQuery {
                name_allowed: &blocked,
                ..query(Some("Mother"), &["how are you?"])
            })
            .unwrap();
        assert_eq!(got.counts, RetrievalCounts::default());
        assert!(!got.block().contains("mum") && !got.block().contains("surgery"));
        // Allowed again, the same query finds both.
        let open = store.retrieve(&query(Some("Mother"), &[])).unwrap();
        assert!(open.block().contains("Secret mum notes.") && open.block().contains("surgery"));
    }

    #[test]
    fn app_style_and_saved_memories_still_count() {
        let (_db, store) = store();
        store
            .save_preferences(
                "chat.app",
                &AiPreferences {
                    writing_style: "Warm and brief.".into(),
                    use_shared_memory: true,
                },
            )
            .unwrap();
        store.save_memory("*", "Call me Pat").unwrap();
        store.save_memory("other.app", "Not for this app").unwrap();
        let got = store.retrieve(&query(None, &[])).unwrap();
        assert!(got.block().contains("Warm and brief.") && got.block().contains("Call me Pat"));
        assert!(!got.block().contains("Not for this app"));
        // The stock default style is already in the system prompt, so it is not repeated.
        let fresh_db = TestDatabase::new();
        let fresh = AiStore::open_at(&fresh_db.path).unwrap();
        assert_eq!(fresh.retrieve(&query(None, &[])).unwrap().block(), "");
    }
}
