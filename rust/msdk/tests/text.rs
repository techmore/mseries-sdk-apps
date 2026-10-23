use msdk::text::{self, parse, print};
use msdk::{Severity, check, templates};
use serde_json::Value;

fn json(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn all_docs() -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = templates::TEMPLATES.iter().map(|t| (t.name.to_string(), json(t.ui))).collect();
    for f in ["quick-notes", "disk-space", "dock"] {
        let path = format!("{}/examples/{f}.ui.json", env!("CARGO_MANIFEST_DIR"));
        v.push((f.to_string(), json(&std::fs::read_to_string(path).unwrap())));
    }
    v.push(("greeter".into(), json(include_str!("../../mseries-greeter/data/greeter.ui.json"))));
    v
}

#[test]
fn every_example_round_trips_through_the_text_form() {
    for (name, doc) in all_docs() {
        let text = print(&doc).unwrap_or_else(|e| panic!("{name}: cannot print: {e}"));
        let back = parse(&text).unwrap_or_else(|e| panic!("{name}: cannot parse its own output: {e}\n{text}"));
        assert_eq!(back.doc, doc, "{name} changed in the round trip:\n{text}");
    }
}

#[test]
fn formatting_is_idempotent() {
    for (name, doc) in all_docs() {
        let once = print(&doc).unwrap();
        let twice = print(&parse(&once).unwrap().doc).unwrap();
        assert_eq!(once, twice, "{name}");
    }
}

#[test]
fn the_design_language_example_compiles_and_checks() {
    let src = r#"
// A small notes app in the SwiftUI/QML-style text form.
app "Quick Notes" id io.github.techmore.QuickNotes icon accessories-text-editor
needs "filesystem:data"

type Note { id: Text, title: Text, when: Text, body: Text }

state {
  query: Text
  notes: [Note]
  selected: Text
  body: Text
  has_selection: Bool
}
persist notes, selected

message AddNote, DeleteNote, SelectNote(Text), EditBody(Text), Search(Text)

on AddNote {
  append notes { id: $id, title: "New note", when: $now, body: "" }
  has_selection = true
}
on DeleteNote {
  remove notes where "{selected}"
  has_selection = false
  toast "Note deleted"
}
on SelectNote {
  selected = $payload
  has_selection = true
}
on EditBody {
  body = $payload
  patch notes where "{selected}" { body: $payload, title: $payload }
}
on Search { query = $payload }

window "Quick Notes" {
  header {
    Button("New note", icon: "list-add", shortcut: "Ctrl+N") { AddNote }
    Button("Delete note", icon: "edit-delete", style: destructive, confirm: "Delete this note?") { DeleteNote }.visible(has_selection)
  }
  SplitView {
    sidebar {
      VStack(spacing: small) {
        SearchField("Search notes", bind: query, onChange: Search)
        List(notes, key: id, onSelect: SelectNote) {
          row { Text("{title}") }
          empty {
            EmptyState("No notes yet", description: "Press Ctrl+N to write one.", icon: "document-new") {
              action { Button("New note", style: suggested) { AddNote } }
            }
          }
        }
      }
    }
    content { TextEditor("Note text", bind: body, onChange: EditBody).visible(has_selection) }
  }
}
"#;
    let parsed = parse(src).unwrap_or_else(|e| panic!("{e}"));
    let errors: Vec<String> =
        check(&parsed.doc).iter().filter(|d| d.severity == Severity::Error).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    // Same behaviour as the JSON example: its gates pass against the parsed document.
    let gates = json(include_str!("../examples/quick-notes.gates.json"));
    assert!(msdk::gate::run(&parsed.doc, Some(&gates)).iter().all(|r| r.passed));
    assert_eq!(parsed.doc["surfaces"]["main"]["body"]["sidebar"]["children"][0]["entry_kind"], "search");
}

#[test]
fn diagnostics_map_back_to_text_lines() {
    let src = "app \"T\" id a.b.C icon x\nstate { n: Number }\nmessage Go\non Go { n = \"five\" }\nwindow \"T\" {\n  VStack {\n    Text(\"{n}\")\n    Text(\"{nope}\")\n  }\n}\n";
    let parsed = parse(src).unwrap();
    let diags = check(&parsed.doc);
    let d = diags.iter().find(|d| d.code == "state-type-mismatch").expect("type mismatch reported");
    assert_eq!(parsed.line_for(&d.path), Some(4));
}

#[test]
fn parse_errors_have_a_line_and_a_suggestion() {
    let e = parse("app \"T\" id a.b.C icon x\nwindow \"T\" {\n  Buton(\"x\") { Go }\n}\n").unwrap_err();
    assert_eq!(e.line, 3);
    assert!(e.message.contains("Button"), "{e}");
    let e = parse("app \"T\" id a.b.C icon x\nwindow \"T\" {\n  Text(\"x\", colour: red)\n}\n").unwrap_err();
    assert_eq!(e.line, 3);
    assert!(e.message.contains("no property `colour`"), "{e}");
    assert!(parse("state { a: Strng }").unwrap_err().message.contains("unknown type"));
    assert!(parse("on Go { n 5 }").is_err());
    assert!(parse("frobnicate").unwrap_err().message.contains("unknown section"));
}

#[test]
fn aliases_and_modifiers() {
    let p = parse("app \"T\" id a.b.C icon x\nwindow \"T\" {\n  HStack(spacing: large) { Toggle(\"A\", bind: a) }.id(\"row\")\n}\n").unwrap();
    let body = &p.doc["surfaces"]["main"]["body"];
    assert_eq!(
        (body["type"].as_str(), body["direction"].as_str(), body["id"].as_str()),
        (Some("Box"), Some("row"), Some("row"))
    );
    assert_eq!(body["children"][0]["type"], "Switch");
}

#[test]
fn text_files_load_like_json_files() {
    let dir = std::env::temp_dir().join(format!("msdk-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a.msdk");
    std::fs::write(&f, print(&json(templates::find("settings").unwrap().ui)).unwrap()).unwrap();
    let (doc, parsed) = text::load(&f).unwrap();
    assert!(parsed.is_some() && doc["app"]["name"] == "Template Settings");
    std::fs::remove_dir_all(&dir).ok();
}
