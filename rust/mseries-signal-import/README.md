# mseries-signal-import

One-time (and safely repeatable) import of the owner's Signal Desktop history into
the Signal Pantheon client (`io.github.sdolbec.SignalPantheonClient`).

Signal does not sync old history to a newly linked device, so the Pantheon client
starts empty. The official Signal Desktop Beta on the same machine still holds the
history; this tool copies it into the Pantheon client's own encrypted store.

## Use

Close Signal Pantheon first (it rewrites its conversations from memory and would
discard the import), then:

```sh
mseries-signal-import import --dry-run   # read and compare; writes nothing (works while the client is open)
mseries-signal-import import             # do it; safe to run again
mseries-signal-import probe              # layout of the Pantheon store: counts and shapes, never content
```

Defaults: Signal data in `~/.config/Signal Beta`, keyring application `Signal Beta`,
history in `~/.local/share/signal-pantheon/message-history.db`. See `--help` style
usage by running the binary with no arguments. Needs `secret-tool`
(`libsecret-tools`) and an unlocked login keyring.

## Data flow and safety rules

1. The Signal database key is `encryptedKey` in Signal's `config.json`, protected the
   Chromium way (`v11`: AES-128-CBC, key = PBKDF2-HMAC-SHA1 of the keyring's
   `Chromium Safe Storage` password for application `Signal Beta`). It is read from
   the keyring once, in memory, wiped on drop, and never printed or logged.
2. Signal's files are **never opened in place or modified**. `db.sqlite` and its
   `-wal` are copied into a private `0700` directory under `$XDG_RUNTIME_DIR`, the
   copy is read, and the copy is removed afterwards.
3. Message text is never printed; the output is counts only.
4. Before the first write the Pantheon store is copied with SQLite's backup API to
   `message-history.db.pre-signal-import` (kept; never overwritten).
5. Each conversation is written in one transaction together with the catalog, read
   back, and compared. An interrupted run leaves whole conversations; running again
   continues. A message already in a conversation, matched on (timestamp,
   direction), is never added twice, so the tool is idempotent.
6. Records the client already holds are kept byte-for-byte; unknown snapshot members
   are preserved.

## The Pantheon store format (recovered from the installed client)

The client's source is not in this repository, so the format was taken from the
installed binary's `signal_history_store_*` and `signal_native_conversation_*`
functions and checked against the live store (the probe reads it back):

* The 32-byte key is a base64 secret in the login keyring (`purpose =
  encrypted-message-history`).
* `conversation_hash` = HMAC-SHA256(key, key-string).
* A row body is AES-256-GCM, random 12-byte nonce, no AAD, 16-byte tag stored in its
  own column.
* A conversation is one snapshot row (`timestamp = 0`, `outgoing = 0`) under
  `pantheon-v1:<account>:conversation:<id>`; the catalog (JSON array of ids) is under
  `pantheon-v1:<account>:catalog`. `<account>` is found automatically by testing the
  account number, uuid and folder name from `accounts.json` against the store.
* Snapshot: `version 1`, `id`, `name`, `group`, `draft`, `attachmentDraft`, `unread`,
  `expired`, `cleanup`, `viewOnceDraft`, `readSyncQueue`, `clockFloor`, `metadata`,
  `messages`, `pending`. Message record: `text`, `author`, `timestamp`, `modified`,
  `outgoing`, `deleted`, `viewOnce`, `viewOpened`, `localRead`, `readAt`,
  `expirationSeconds`, `expirationStart`, `status`, `attachments`, `reactions`,
  `reactionVersions`.

Conversation ids follow the client: a chat already in the catalog keeps its id (phone
number or uuid); new private chats use whichever form the catalog mostly uses
(default phone number); groups use the group id.

## What is imported, and what is deliberately not

Imported: incoming and outgoing text (mentions become `@name`), deleted-for-everyone
markers, and a one-line placeholder for each attachment, sticker or shared contact
(`[Attachment not imported: image/jpeg]`). Everything arrives marked read.

Left out and counted in the summary: disappearing messages (importing them would
defeat the sender's timer), view-once messages, story replies, and service events
(calls, safety-number and timer changes). Not imported: attachment files (Signal
stores them encrypted separately), reactions, quotes, edit history, and read state.

## Tests

`cargo test` builds synthetic Signal and Pantheon profiles (invented names, 555
numbers, random keys) and exercises the Chromium key unwrap, the store crypto against
independent vectors from Python's `cryptography`, fresh import, merge, idempotence,
resume, dry run, read-only source handling, missing/wrong keys, and the content-free
probe. No real chat, key or keyring is used.

This crate is deliberately outside the `rust/` workspace: it needs SQLCipher, which
claims the same `sqlite3` native library as the plain bundled SQLite the other crates
use, and Cargo allows only one such crate per dependency graph.

## Context and open gaps (2026-10-23)

* The owner asked for their old Signal Desktop history in the Signal Pantheon client,
  which they now use as their primary Signal, and authorized reading the Signal Beta
  key from their login keyring for this import only (read-only, once).
* The official Signal Desktop Beta stays installed as the fallback and keeps the
  `sgnl://` and `signalcaptcha://` handlers: the Pantheon client has no handler for
  those links (its only URI parsing is for links inside messages).
* The Pantheon client has no tray icon and no background mode, so it is not
  autostarted; that is a gap against the official client's `--use-tray-icon`.
* The Pantheon client's source is not in this repository and the working copy is gone:
  it was a Vala/GTK project under `~/Documents/Codex/2026-10-20/com/outputs/signal-pantheon`,
  and the whole `2026-10-20` Codex folder no longer exists (apparently removed by a
  disk-space cleanup); it was never pushed. Only the installed binary and its data
  remain. Changing the client (URL handlers, tray, background mode) needs its source
  recovered or rewritten first, which is why this tool follows the store format of the
  installed binary.
* Not verified against the live account: the real-data dry run reads 80 chats and
  about 2,200 messages, but the written result has not yet been opened in the client.
