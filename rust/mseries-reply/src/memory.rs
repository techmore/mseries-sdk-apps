//! `mseries-reply memory`: the owner's way to add, list, edit, remove and export what Smart Reply knows about them.
//!
//! Nothing here is automatic: every record exists because the owner typed it. Ids are prefixed by table so one number
//! never means two things: `c3` is a contact, `n7` a note (profile, style, topic or fact) and `m2` a saved free-text
//! memory from the older store. The command works on the same database as the drafts (`mseries-ai`) and prints only
//! to the owner's terminal.

use crate::rules::{BlockReason, Decision, FieldKind, Query, Rules};
use mseries_ai::{
    AiStore, ContactInput, ContactPatch, NoteInput, NoteKind, NotePatch, RetrievalQuery,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;

pub const USAGE: &str = "usage:
  mseries-reply memory add contact NAME [--alias NAME]... [--relationship TEXT] [--tone TEXT] [--notes TEXT]
  mseries-reply memory add profile|style TEXT [--contact NAME]      (a profile note cannot belong to a contact)
  mseries-reply memory add topic|fact TEXT [--title TEXT] [--keywords a,b,c] [--contact NAME]
  mseries-reply memory add memory TEXT [--app APP_ID]               (older free-text memory; all apps by default)
  mseries-reply memory list [contact|profile|style|topic|fact|memory]
  mseries-reply memory edit ID [--name|--alias|--relationship|--tone|--notes ...]   (contacts: c1)
  mseries-reply memory edit ID [--text|--title|--keywords|--contact NAME|none]     (notes: n1)
  mseries-reply memory rm ID                                        (removing a contact removes its notes)
  mseries-reply memory export [FILE]                                (JSON; to FILE with owner-only access, or stdout)
  mseries-reply memory preview [--contact NAME] [--app APP_ID] [TEXT...]   (what a draft would be given; applies your contact block rules)
Topics and facts are used when one of their keywords (or, with none, their title) appears in the conversation, or when
they belong to the contact the conversation is with. Profile and style notes are always considered.";

struct Args {
    positionals: Vec<String>,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self, String> {
        let mut positionals = Vec::new();
        let mut flags = Vec::new();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if let Some(flag) = arg.strip_prefix("--") {
                let (name, value) = match flag.split_once('=') {
                    Some((name, value)) => (name.to_string(), value.to_string()),
                    None => {
                        index += 1;
                        let value = args
                            .get(index)
                            .ok_or_else(|| format!("--{flag} needs a value"))?;
                        (flag.to_string(), value.clone())
                    }
                };
                if !allowed.contains(&name.as_str()) {
                    return Err(format!("unknown option --{name}\n{USAGE}"));
                }
                flags.push((name, value));
            } else {
                positionals.push(arg.clone());
            }
            index += 1;
        }
        Ok(Self { positionals, flags })
    }

    fn one(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(flag, _)| flag == name)
            .map(|(_, value)| value.as_str())
    }

    fn many(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|(flag, _)| flag == name)
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(flag, _)| flag == name)
    }

    fn text(&self, what: &str) -> Result<String, String> {
        match self.positionals.as_slice() {
            [text] => Ok(text.clone()),
            [] => Err(format!("{what} is required\n{USAGE}")),
            _ => Err(format!("give {what} as one quoted argument\n{USAGE}")),
        }
    }
}

fn comma_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordId {
    Contact(i64),
    Note(i64),
    Memory(i64),
}

fn parse_id(text: &str) -> Result<RecordId, String> {
    let (prefix, number) = text.split_at(text.chars().next().map_or(0, char::len_utf8));
    let id = number.parse::<i64>().ok().filter(|id| *id > 0);
    match (prefix, id) {
        ("c", Some(id)) => Ok(RecordId::Contact(id)),
        ("n", Some(id)) => Ok(RecordId::Note(id)),
        ("m", Some(id)) => Ok(RecordId::Memory(id)),
        _ => Err(format!(
            "{text:?} is not a record id; ids look like c3 (contact), n7 (note) or m2 (memory); see `memory list`"
        )),
    }
}

fn contact_by_name(store: &AiStore, name: &str) -> Result<i64, String> {
    store
        .find_contact(name)?
        .map(|contact| contact.id)
        .ok_or_else(|| {
            format!("There is no contact named {name}; add one with `memory add contact`")
        })
}

pub fn run(
    args: &[String],
    store: &AiStore,
    rules: &Rules,
    out: &mut dyn Write,
) -> Result<(), String> {
    let write = |error: std::io::Error| format!("could not write output: {error}");
    match args.first().map(String::as_str) {
        Some("add") => add(&args[1..], store, out),
        Some("list") => list(&args[1..], store, out),
        Some("edit") => edit(&args[1..], store, out),
        Some("rm") => {
            let [id] = &args[1..] else {
                return Err(format!("rm takes one id\n{USAGE}"));
            };
            let removed = match parse_id(id)? {
                RecordId::Contact(n) => store.delete_contact(n)?,
                RecordId::Note(n) => store.delete_note(n)?,
                RecordId::Memory(n) => {
                    let exists = store.all_memories()?.iter().any(|memory| memory.id == n);
                    store.delete_memory(n)?;
                    exists
                }
            };
            if !removed {
                return Err(format!("There is no record {id}"));
            }
            writeln!(out, "removed {id}").map_err(write)
        }
        Some("export") => export(&args[1..], store, out),
        Some("preview") => preview(&args[1..], store, rules, out),
        _ => Err(USAGE.to_string()),
    }
}

fn add(args: &[String], store: &AiStore, out: &mut dyn Write) -> Result<(), String> {
    let kind = args.first().map(String::as_str).ok_or(USAGE)?;
    let rest = &args[1..];
    let id = match kind {
        "contact" => {
            let args = Args::parse(rest, &["alias", "aliases", "relationship", "tone", "notes"])?;
            let mut aliases = args.many("alias");
            for list in args.many("aliases") {
                aliases.extend(comma_list(&list));
            }
            let id = store.add_contact(&ContactInput {
                name: args.text("a name")?,
                aliases,
                relationship: args.one("relationship").unwrap_or_default().into(),
                tone: args.one("tone").unwrap_or_default().into(),
                notes: args.one("notes").unwrap_or_default().into(),
            })?;
            format!("c{id}")
        }
        "profile" | "style" | "topic" | "fact" => {
            let note_kind = NoteKind::parse(kind).ok_or(USAGE)?;
            let args = Args::parse(rest, &["title", "keywords", "contact"])?;
            let contact_id = args
                .one("contact")
                .map(|name| contact_by_name(store, name))
                .transpose()?;
            let id = store.add_note(&NoteInput {
                kind: note_kind,
                title: args.one("title").unwrap_or_default().into(),
                keywords: args
                    .many("keywords")
                    .iter()
                    .flat_map(|v| comma_list(v))
                    .collect(),
                body: args.text("the text")?,
                contact_id,
            })?;
            format!("n{id}")
        }
        "memory" => {
            let args = Args::parse(rest, &["app"])?;
            let id = store.save_memory(args.one("app").unwrap_or("*"), &args.text("the text")?)?;
            format!("m{id}")
        }
        other => return Err(format!("unknown kind {other}\n{USAGE}")),
    };
    writeln!(out, "added {id}").map_err(|error| format!("could not write output: {error}"))
}

fn edit(args: &[String], store: &AiStore, out: &mut dyn Write) -> Result<(), String> {
    let (id, rest) = args.split_first().ok_or(USAGE)?;
    match parse_id(id)? {
        RecordId::Contact(n) => {
            let args = Args::parse(
                rest,
                &["name", "alias", "aliases", "relationship", "tone", "notes"],
            )?;
            if !args.positionals.is_empty() {
                return Err(format!("edit takes options only\n{USAGE}"));
            }
            let aliases = (args.has("alias") || args.has("aliases")).then(|| {
                let mut all = args.many("alias");
                for list in args.many("aliases") {
                    all.extend(comma_list(&list));
                }
                all
            });
            store.update_contact(
                n,
                &ContactPatch {
                    name: args.one("name").map(str::to_string),
                    aliases,
                    relationship: args.one("relationship").map(str::to_string),
                    tone: args.one("tone").map(str::to_string),
                    notes: args.one("notes").map(str::to_string),
                },
            )?;
        }
        RecordId::Note(n) => {
            let args = Args::parse(rest, &["text", "title", "keywords", "contact"])?;
            if !args.positionals.is_empty() {
                return Err(format!("edit takes options only\n{USAGE}"));
            }
            let contact_id = match args.one("contact") {
                None => None,
                Some("none") => Some(None),
                Some(name) => Some(Some(contact_by_name(store, name)?)),
            };
            store.update_note(
                n,
                &NotePatch {
                    title: args.one("title").map(str::to_string),
                    keywords: args.has("keywords").then(|| {
                        args.many("keywords")
                            .iter()
                            .flat_map(|v| comma_list(v))
                            .collect()
                    }),
                    body: args.one("text").map(str::to_string),
                    contact_id,
                },
            )?;
        }
        RecordId::Memory(_) => {
            return Err(
                "Saved memories cannot be edited: `memory rm` it and `memory add memory` again"
                    .into(),
            );
        }
    }
    writeln!(out, "updated {id}").map_err(|error| format!("could not write output: {error}"))
}

fn list(args: &[String], store: &AiStore, out: &mut dyn Write) -> Result<(), String> {
    let filter = match args {
        [] => None,
        [kind] => Some(kind.as_str()),
        _ => return Err(USAGE.to_string()),
    };
    let wants = |name: &str| filter.is_none_or(|kind| kind == name);
    if let Some(kind) = filter
        && !["contact", "profile", "style", "topic", "fact", "memory"].contains(&kind)
    {
        return Err(format!("unknown kind {kind}\n{USAGE}"));
    }
    let write = |error: std::io::Error| format!("could not write output: {error}");
    let contacts = store.contacts()?;
    if wants("contact") && !contacts.is_empty() {
        writeln!(out, "Contacts").map_err(write)?;
        for contact in &contacts {
            let mut line = format!("  c{:<4} {}", contact.id, contact.name);
            if !contact.relationship.is_empty() {
                line.push_str(&format!(" ({})", contact.relationship));
            }
            if !contact.aliases.is_empty() {
                line.push_str(&format!("  aliases: {}", contact.aliases.join(", ")));
            }
            if !contact.tone.is_empty() {
                line.push_str(&format!("  tone: {}", contact.tone));
            }
            if !contact.notes.is_empty() {
                line.push_str(&format!("  — {}", contact.notes));
            }
            writeln!(out, "{line}").map_err(write)?;
        }
    }
    for (kind, heading) in [
        (NoteKind::Profile, "Profile"),
        (NoteKind::Style, "Style"),
        (NoteKind::Topic, "Topics"),
        (NoteKind::Fact, "Facts"),
    ] {
        if !wants(kind.as_str()) {
            continue;
        }
        let notes = store.notes(Some(kind))?;
        if notes.is_empty() {
            continue;
        }
        writeln!(out, "{heading}").map_err(write)?;
        for note in notes {
            let mut line = format!("  n{:<4} ", note.id);
            if !note.title.is_empty() {
                line.push_str(&format!("{}: ", note.title));
            }
            line.push_str(&note.body);
            if !note.keywords.is_empty() {
                line.push_str(&format!("  [keywords: {}]", note.keywords.join(", ")));
            }
            if let Some(contact) = note
                .contact_id
                .and_then(|id| contacts.iter().find(|c| c.id == id))
            {
                line.push_str(&format!("  [for {}]", contact.name));
            }
            writeln!(out, "{line}").map_err(write)?;
        }
    }
    if wants("memory") {
        let memories = store.all_memories()?;
        if !memories.is_empty() {
            writeln!(out, "Saved memories").map_err(write)?;
            for memory in memories {
                let scope = if memory.scope == "*" {
                    String::new()
                } else {
                    format!("  [app {}]", memory.scope)
                };
                writeln!(out, "  m{:<4} {}{scope}", memory.id, memory.content).map_err(write)?;
            }
        }
    }
    Ok(())
}

fn export(args: &[String], store: &AiStore, out: &mut dyn Write) -> Result<(), String> {
    let json = serde_json::to_string_pretty(&store.export_all()?).map_err(|e| e.to_string())?;
    match args {
        [] => writeln!(out, "{json}").map_err(|error| format!("could not write output: {error}")),
        [path] => {
            // Owner-only, and never over an existing file: an export is a copy of private notes.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|error| format!("{path}: {error}"))?;
            file.write_all(json.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .map_err(|error| format!("{path}: {error}"))?;
            writeln!(out, "wrote {path}")
                .map_err(|error| format!("could not write output: {error}"))
        }
        _ => Err(USAGE.to_string()),
    }
}

/// Shows what a draft in a conversation with `--contact` would be given, using the owner's current rules.
fn preview(
    args: &[String],
    store: &AiStore,
    rules: &Rules,
    out: &mut dyn Write,
) -> Result<(), String> {
    let args = Args::parse(args, &["contact", "app"])?;
    // Without --app only the owner's contact block rules are applied; the app allow list is not part of the question.
    let app = args.one("app");
    let app_id = app.unwrap_or("preview.app");
    let recent = args.positionals.join(" ");
    let recent_list = [recent.as_str()];
    let allowed = |name: &str| match rules.decide(&Query {
        app_id,
        field: FieldKind::Chat,
        contact: Some(name),
    }) {
        Decision::Allow(_) => true,
        Decision::Block(reason) => app.is_none() && reason != BlockReason::Rule,
    };
    let personal = store.retrieve(&RetrievalQuery {
        contact: args.one("contact"),
        recent: &recent_list,
        name_allowed: &allowed,
        ..RetrievalQuery::new(app_id)
    })?;
    let write = |error: std::io::Error| format!("could not write output: {error}");
    writeln!(out, "{:?}", personal.counts).map_err(write)?;
    if personal.block().is_empty() {
        writeln!(out, "(nothing would be added to the prompt)").map_err(write)
    } else {
        writeln!(out, "{}", personal.block()).map_err(write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Policy, RULES_VERSION, Settings};

    struct Dir(std::path::PathBuf);
    impl Dir {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("mseries-memory-{}-{nonce}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open_rules() -> Rules {
        Rules::new(Settings {
            version: RULES_VERSION,
            enabled: true,
            policy: Policy::EverywhereExceptBlocked,
            hotkey: None,
            rules: vec![],
        })
    }

    fn run_args(store: &AiStore, line: &[&str]) -> Result<String, String> {
        let args: Vec<String> = line.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        run(&args, store, &open_rules(), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn add_list_edit_remove_and_export_round_trip() {
        let dir = Dir::new();
        let store = AiStore::open_at(dir.0.join("assistant.sqlite3")).unwrap();

        assert_eq!(
            run_args(
                &store,
                &[
                    "add",
                    "contact",
                    "Sam Example",
                    "--alias",
                    "Sammy",
                    "--relationship",
                    "brother",
                    "--tone",
                    "teasing"
                ]
            )
            .unwrap(),
            "added c1\n"
        );
        run_args(&store, &["add", "profile", "I run a small workshop."]).unwrap();
        run_args(
            &store,
            &[
                "add",
                "topic",
                "Planning a ride in May",
                "--title",
                "Bike trip",
                "--keywords",
                "bike, ride",
                "--contact",
                "sammy",
            ],
        )
        .unwrap();
        run_args(&store, &["add", "memory", "Call me Pat"]).unwrap();

        let listing = run_args(&store, &["list"]).unwrap();
        for expected in [
            "Contacts",
            "c1",
            "Sam Example (brother)",
            "aliases: Sammy",
            "Profile",
            "n1",
            "Topics",
            "Bike trip: Planning a ride in May",
            "[keywords: bike, ride]",
            "[for Sam Example]",
            "Saved memories",
            "m1",
        ] {
            assert!(
                listing.contains(expected),
                "{expected} missing from\n{listing}"
            );
        }
        assert!(
            !run_args(&store, &["list", "topic"])
                .unwrap()
                .contains("Contacts")
        );

        run_args(
            &store,
            &[
                "edit",
                "n2",
                "--text",
                "Ride moved to June.",
                "--contact",
                "none",
            ],
        )
        .unwrap();
        run_args(
            &store,
            &["edit", "c1", "--tone", "dry humour", "--alias", "Sam E"],
        )
        .unwrap();
        let listing = run_args(&store, &["list"]).unwrap();
        assert!(listing.contains("Ride moved to June.") && !listing.contains("[for Sam Example]"));
        assert!(
            listing.contains("tone: dry humour")
                && listing.contains("aliases: Sam E")
                && !listing.contains("Sammy")
        );

        let preview = run_args(
            &store,
            &["preview", "--contact", "Sam E", "any", "bike", "plans?"],
        )
        .unwrap();
        assert!(
            preview.contains("dry humour") && preview.contains("Ride moved to June."),
            "{preview}"
        );

        let path = dir.0.join("export.json");
        run_args(&store, &["export", path.to_str().unwrap()]).unwrap();
        let exported = std::fs::read_to_string(&path).unwrap();
        assert!(exported.contains("Sam Example") && exported.contains("Call me Pat"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            run_args(&store, &["export", path.to_str().unwrap()]).is_err(),
            "never overwrites"
        );

        run_args(&store, &["rm", "m1"]).unwrap();
        run_args(&store, &["rm", "c1"]).unwrap();
        let after = run_args(&store, &["list"]).unwrap();
        assert!(!after.contains("Sam Example") && !after.contains("Call me Pat"));
        // The topic was unlinked from the contact earlier, so it outlives the contact; the profile note too.
        assert!(after.contains("Ride moved to June.") && after.contains("I run a small workshop."));
    }

    #[test]
    fn mistakes_are_reported_not_guessed() {
        let dir = Dir::new();
        let store = AiStore::open_at(dir.0.join("assistant.sqlite3")).unwrap();
        for line in [
            &["add", "contact"][..],
            &["add", "fact", "x", "--contact", "Nobody"],
            &["add", "fact", "x", "--bogus", "1"],
            &["add", "profile", "x", "--contact", "Nobody"],
            &["add", "banana", "x"],
            &["rm", "7"],
            &["rm", "n7"],
            &["edit", "m1", "--text", "x"],
            &["list", "banana"],
            &[],
        ] {
            assert!(run_args(&store, line).is_err(), "{line:?}");
        }
    }

    #[test]
    fn ids_are_prefixed_by_table() {
        assert_eq!(parse_id("c3").unwrap(), RecordId::Contact(3));
        assert_eq!(parse_id("n12").unwrap(), RecordId::Note(12));
        assert_eq!(parse_id("m2").unwrap(), RecordId::Memory(2));
        for bad in ["3", "x3", "c", "c0", "c-1", ""] {
            assert!(parse_id(bad).is_err(), "{bad:?}");
        }
    }
}
