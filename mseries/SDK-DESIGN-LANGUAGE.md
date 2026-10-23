# Design language for the SDK: SwiftUI/QML-style text (proposal, 2026-10-23)

Status: **proposal, not built.** The owner's direction: write apps in a concise, SwiftUI- or QML-like language that small models
already know, and still render through our Rust Pantheon stack. This note defines the shape so the choice can be made once and
tested at the benchmark decision point.

## 1. Principle: one AST, two spellings

The canonical form stays the typed JSON (`*.ui.json`) that the checker, JSON Schema, renderer, gates and tools already use. The
SwiftUI-style text is a **surface syntax that compiles to exactly that JSON**. Nothing downstream changes: `msdk check`,
`describe`, `complete`, `gate`, `msdk-run` and the Rust generator all keep working, and diagnostics still point at JSON paths
(the parser keeps a source map so they also show the text line).

Why keep JSON canonical: a grammar generated from `msdk schema` forces a small model to emit structurally valid JSON; the text
syntax is for readability and prior knowledge, so we offer both and let the benchmark decide which a 4B model gets right more often.

## 2. What it looks like

Quick Notes (today 80 lines of JSON) as text:

```
app QuickNotes "Quick Notes" id io.github.techmore.QuickNotes icon accessories-text-editor
needs filesystem:data

type Note { id: Text, title: Text, when: Text, body: Text }

state {
  query: Text
  notes: [Note] persist
  selected: Text persist
  body: Text
  hasSelection: Bool
}

message AddNote, DeleteNote, SelectNote(Text), EditBody(Text), Search(Text)

on AddNote {
  append notes { id: $id, title: "New note", when: $now, body: "" }
  hasSelection = true
}
on DeleteNote { remove notes where selected;  hasSelection = false;  toast "Note deleted" }
on SelectNote { selected = $payload;  hasSelection = true }
on EditBody   { body = $payload;  patch notes where selected { body: $payload, title: $payload } }
on Search     { query = $payload }

window "Quick Notes" {
  toolbar {
    Button("New note", icon: "list-add", key: "Ctrl+N") { AddNote }
    Button("Delete note", icon: "edit-delete", style: destructive, confirm: "Delete this note?") { DeleteNote }
      .visible(hasSelection)
  }
  SplitView {
    sidebar {
      SearchField("Search notes", text: $query) { Search }
      List(notes, key: id, select: SelectNote) { note in
        Text(note.title)
      } empty: EmptyState("No notes yet", detail: "Press Ctrl+N to write one.", icon: "document-new") {
        Button("New note", style: suggested) { AddNote }
      }
    }
    TextEditor("Note text", text: $body) { EditBody }.visible(hasSelection)
  }
}
```

## 3. Mapping rules (so it compiles to the same JSON)

| Text | JSON |
|---|---|
| `Name(args) { ... }` | component `{"type": "Name", ...}`; first string is the `label`/`text`/`title` |
| `{ Message }` trailing closure on a control | its action: `on` / `on_change` / `on_submit` / `on_select` by component |
| `$name` | a two-way binding to state (`bind`) |
| `.visible(flag)` | `visible_if` |
| `VStack` / `HStack` | `Box` with `direction` column / row (aliases) |
| `state { ... }`, `type`, `message`, `on Msg { ... }` | `state`, `types`, `messages`, `update` |
| `key = value` inside `on` | `set`; `append`, `patch`, `remove`, `call`, `toast`, `add` keep their names |
| `panel(bottom, size: 64, hide: overlappingFocusWindow)` | panel surface |
| `needs` | `capabilities`; `persist` after a state key | `persist` |

Names borrow from SwiftUI where the meaning is identical (`Text`, `Button`, `List`, `Toggle`, `TextField`, `NavigationSplitView`
as `SplitView`) and from QML where it is closer (`ListView` semantics for `List`), because the model has seen both many times.
The vocabulary table in `vocab.rs` gets an `aliases` column so `Toggle` and `Switch` are one component; `msdk complete` offers
the preferred spelling.

## 4. Size and accuracy

- The text form of Quick Notes is about 45 lines against 80 of JSON, with no punctuation noise for the model to get wrong.
- Accuracy comes from the same place as today: `describe`, `complete` and `explain` work on the text (completions are produced from
  the AST at the cursor), the checker is the same, and gates are unchanged.
- A model-friendly rule: one construct per line, no operator precedence, no expressions beyond literals, `$payload`, `$id`, `$now`
  and `{template}` strings. Anything harder goes to a Rust `Logic`.

## 5. Rust objects

`msdk gen` emits, from the same AST: `struct State`, `enum Msg` with payload variants, a `trait Logic` stub for the hard parts, and
the view as a Rust builder. That is the "make Rust objects" requirement and the release-mode compile target (the interpreter is the
development path).

## 6. Decision and next steps

1. Build the parser (about 600 lines): tokens, the grammar above, a source map to JSON paths, and a formatter (`msdk fmt`).
2. Add aliases to the vocabulary table; teach `complete`/`describe` to speak the text form.
3. At the benchmark decision point, run the six sdk-bench specs on Qwen ~4B twice, once with JSON and once with the text form,
   and keep whichever (or both) the model gets right. This is the one measurement the owner has agreed to make.
