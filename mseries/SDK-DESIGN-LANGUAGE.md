# Design language for the SDK: SwiftUI/QML-style text (proposal, 2026-10-23)

Status: **parser, printer and formatter built (2026-10-23).** `msdk fmt`, `msdk text`, `msdk json`, `msdk new <template> <dir> --text`, and `.msdk` files work with `check`, `gate`, `complete` and `msdk-run`. Every template and example round-trips JSON to text and back unchanged, and formatting is idempotent (tests in `rust/msdk/tests/text.rs`).

The grammar as built differs from the sketch below in these ways: bindings are named arguments (`bind: query`), not `$query`; list rows use the same `"{title}"` templates as JSON (no `note in` closure yet); sections are `app`, `needs`, `logic`, `type`, `state`, `initial`, `persist`, `feed`, `message`, `on`, and the surfaces; state keys keep their snake_case names so the same gates work on both forms.

Original status note: The owner's direction: write apps in a concise, SwiftUI- or QML-like language that small models
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

## 5b. A Ruby-style dialect (owner likes it; proposal)

The same AST can be written in a Ruby-flavoured DSL: `do ... end` blocks, `:symbols`, `key: value` hashes, almost no punctuation.
Models know this shape well from Rails routes, RSpec, Rake and Sinatra, and the owner works in Rails daily.

```ruby
app "Quick Notes", id: "io.github.techmore.QuickNotes", icon: "accessories-text-editor" do
  needs "filesystem:data"

  record :Note, id: :text, title: :text, when: :text, body: :text

  state do
    query         :text
    notes         list_of(:Note), persist: true
    selected      :text,          persist: true
    body          :text
    has_selection :bool
  end

  message :add_note, :delete_note, :select_note => :text, :edit_body => :text, :search => :text

  on :add_note do
    append :notes, id: id!, title: "New note", when: now!, body: ""
    set has_selection: true
  end
  on :delete_note do
    remove :notes, where: "{selected}"
    set has_selection: false
    toast "Note deleted"
  end
  on(:search) { set query: payload }

  window "Quick Notes" do
    header do
      button "New note", icon: "list-add", key: "Ctrl+N", on: :add_note
      button "Delete note", style: :destructive, confirm: "Delete this note?", on: :delete_note, visible: :has_selection
    end
    split_view do
      sidebar do
        v_stack spacing: :small do
          search_field "Search notes", bind: :query, on_change: :search
          list :notes, key: :id, on_select: :select_note do
            row   { text "{title}" }
            empty { empty_state "No notes yet", icon: "document-new" }
          end
        end
      end
      content { text_editor "Note text", bind: :body, on_change: :edit_body, visible: :has_selection }
    end
  end
end
```

Rules that keep it safe and checkable:

- **It is parsed, not run.** No Ruby interpreter is involved (there is none on the laptop anyway); the dialect is a fixed grammar,
  so there are no loops, conditionals, method definitions or string interpolation. Anything outside the subset is an error with a
  line number and a pointer to the closest allowed form, which stops the model from "helpfully" writing real Ruby.
- Component names become `snake_case` calls (`empty_state`, `split_view`), enum values and state keys become symbols, and
  `payload`, `id!`, `now!` replace `$payload`, `$id`, `$now`.
- `msdk fmt --dialect ruby|swift|json` converts between the three spellings, and `msdk new --dialect ruby` starts from a template.

Cost: a lexer and parser of the same size as the SwiftUI-style front end (about 600 lines) plus a printer; nothing downstream.

Which spelling is primary is a question for the benchmark, not for taste: run the same six specs on Qwen ~4B in JSON, the SwiftUI-style
text and this dialect, and keep the one it gets right most often (any of them can stay as an input form). Until then the SwiftUI-style
text is built and the Ruby dialect is specified only.

## 6. Decision and next steps

1. Build the parser (about 600 lines): tokens, the grammar above, a source map to JSON paths, and a formatter (`msdk fmt`).
2. Add aliases to the vocabulary table; teach `complete`/`describe` to speak the text form.
3. At the benchmark decision point, run the six sdk-bench specs on Qwen ~4B twice, once with JSON and once with the text form,
   and keep whichever (or both) the model gets right. This is the one measurement the owner has agreed to make.
