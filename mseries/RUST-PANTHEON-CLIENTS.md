# Rust Pantheon clients: T3, Signal and Messages

## Direction

The owner wants the first-party desktop app layer to use a shared Rust SDK,
GTK 4 and Granite on Pantheon. `rust/claude-pantheon-rs` is the reference
implementation: it is
the T3 Code client for Claude Code, Codex, OpenCode and Grok. Signal and Messages
are separate apps in the same native app family; this does not imply replacing
Gala, Wingpanel or Dock.

The broader SDK design and app-by-app port inventory are in
[`RUST-PANTHEON-SDK.md`](RUST-PANTHEON-SDK.md).

## Messages

`rust/mseries-messages` is the native client prototype. It talks to BlueFerry's
versioned session D-Bus `Messages1` API (currently API v2) for status, history,
favorites, read state and checked sends. BlueFerry remains the only owner of the
iPhone MAP session, recipient/group validation and encrypted message history.
The Rust client never opens BlueFerry's private database or starts a competing
MAP connection.

The preview currently supports conversation search, message history, in-memory
per-thread drafts, favorites, read state, checked replies, history invalidation,
and a participant-review dialog before a group send. It installs beside the
existing Python GTK/Granite Messages app. The Python app remains the qualified
client until the Rust app reaches feature and accessibility parity and passes
the owner's live phone acceptance.

Parity work still includes the new-message/contact picker, favorite layout and
imported iPhone pins, contact photos, safe link previews, detailed delivery and
retry feedback, message actions, accessibility/keyboard and narrow-window
behavior, localization, package/launcher handoff, and fixture plus private GUI
and owner-phone qualification. Those are migration gates, not claims about the
current prototype.

## Signal

The installed signed Signal Desktop remains the functional client while a
first-party app boundary is investigated. Signal Desktop is officially
distributed as an Electron/TypeScript application. Signal's `libsignal`
repository says its client APIs are for Signal's own use and external use is
unsupported. A Rust GTK shell alone would not provide supported access to
Signal conversations, encrypted storage, attachments or sends.

Do not inspect or modify Signal's private profile/database, reimplement its
protocol, or present a shell around the existing client as a native Signal
replacement. A Rust Signal app needs a supported upstream API or an explicit
upstream-supported integration boundary, plus separate security review and
owner acceptance. Until then, keep the official signed app as the transition
path and label any theme, tray or launcher integration as transitional.

## Acceptance boundaries

- T3: the existing Rust GTK/Granite app and its T3 Code server are the pattern;
  this client work does not requalify all T3 features.
- Messages: source/build success is not phone delivery, feature parity or an
  owner desktop installation.
- Signal: no Rust replacement is accepted until the supported transport and
  encrypted-data boundary exist.
- All three apps: Pantheon-native app work does not authorize replacing the
  operating-system shell or restarting the owner's session.
