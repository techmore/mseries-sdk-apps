# s01 Quick Notes (easy)

Golden reference: none (new, small). Reviewed by a human against the rubric.

A notes app. Left: a list of notes (title = first line, date under it), newest first, with a search
entry above. Right: the selected note as editable text. A header-bar button adds a note; Delete
removes the selected note after an undo toast. Notes persist in SQLite in the user's data dir.

Capabilities: filesystem (own data dir).

Acceptance:
1. Add creates an empty note and focuses the editor.
2. Typing updates the list title live; the list re-sorts by last edit.
3. Search filters the list as you type; clearing restores it.
4. Delete shows a toast with Undo for 6 s; Undo restores the note in place.
5. Relaunching restores notes and the selected note.
6. Empty state ("No notes yet") with an Add button when the list is empty.
7. Shortcuts: Ctrl+N new, Ctrl+F search, Ctrl+Q quit.
