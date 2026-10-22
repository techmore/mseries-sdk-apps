## Reproducible Messages GTK test harness — 2026-10-22

Run `bash rust/mseries-messages/test-gui.sh` to exercise the offline widget test
under Xvfb/Cairo, temporary XDG directories and a private D-Bus with no service
activation directories. The script cleans its temporary files and passes locally;
the Messages CI workflow now installs Xvfb/xauth/D-Bus and runs it explicitly.
Expected missing-portal/software-display warnings do not affect this widget-state
check. CI execution, visual/accessibility and connected-phone acceptance remain
unverified. The locally built Debian package remains uninstalled under build/.

## Messages isolated GTK recovery regression — 2026-10-22

An opt-in ignored GTK test now constructs the real App, verifies offline compose
controls are disabled and confirms Back clears selection while retaining a draft.
It passes under Xvfb/Cairo with a private D-Bus configuration containing no service
activation directories and temporary config/cache/data paths. This checks widget
state, not visual rendering, accessibility or a connected phone. An initial run
using the standard private-session configuration activated host-installed portal
helpers; the retained validation was rerun with activation disabled. Next: make
this isolated harness reproducible in CI and add synthetic connected-service cases.

## SDK phone identity validation — 2026-10-22

The public status parser now rejects malformed supplied phone UUIDs before any
app can use them for session/cache keys. Missing IDs remain compatible with
released BlueFerry. A focused SDK regression checks valid/missing IDs, malformed
values, API version rejection and omission of invalid identity values from the
error. The focused test passes; live service acceptance remains open.

## Messages avatar cache isolation — 2026-10-22

A reported phone change invalidates the stored photo revision before the normal
revision update clears cached avatars/requested addresses and increments the
photo generation. Matching revision numbers on different phones can no longer
retain old photos or accept old-generation results. Strict Messages Clippy,
formatting and diff checks pass; delayed-photo GUI acceptance remains open.

## Messages contact-picker phone guard — 2026-10-22

New-message dialogs capture the phone generation. Activating a contact, starting
a direct conversation or searching after an observed switch is rejected with
reopen guidance; in-flight contact search results from the previous generation
are discarded. Strict locked Clippy across all Messages targets, formatting
and diff checks pass. GTK interaction/delayed-service tests remain open.

## Messages refresh identity consistency — 2026-10-22

Refresh now rereads public status after inbox/history calls and rejects results
when phone identity or MAP readiness changed during the request. Failed refresh
also clears send readiness, preventing keyboard sends from bypassing the disabled
button. All 17 Messages library tests pass, including identity/disconnect rejection.
Formatting and diff checks pass. Endpoint checks cannot detect an A-to-B-to-A
switch or distinguish phones on old releases without UUIDs; a backend snapshot
identity/generation contract is still needed for that guarantee. Delayed D-Bus
integration and live phone acceptance remain open.

## Messages phone-state regression coverage — 2026-10-22

The production phone-state transition is now exercised by tests covering A/B
phones with identical thread keys, separate draft restoration, cleared selection
and pending flags, and unresolved sends restored as OutcomeUnknown. A second
case verifies unidentified drafts are never assigned to a newly identified phone.
All 16 Messages library tests pass. GTK callback/delayed-service interaction and
live acceptance remain open; these tests qualify the state transition only.

## Messages per-phone session state — 2026-10-22

On an observed phone change, Messages parks identified phones' in-memory drafts
and outbox separately and restores them when that UUID returns. Selection is
cleared, old inbox-only rows are removed, and unresolved Sending entries become
OutcomeUnknown; no retry is dispatched. Unknown-identity state is not assigned
to a newly identified phone. Existing 14 library tests pass; dedicated synthetic
switch/delayed-response tests remain required. Released BlueFerry without phone
UUIDs cannot provide this isolation. Live phone qualification remains open.

## Messages confirmation and read callback guards — 2026-10-22

Group confirmation refuses to dispatch after an observed phone-profile change
and asks the user to review the conversation again. Retained Mark as Read row
actions reject a changed profile; both read callback paths ignore old-phone
results. The existing 14 Messages library tests pass, with formatting and diff
checks clean; they do not exercise these GTK callbacks. Synthetic delayed-response
and dialog interaction coverage, per-phone draft/outbox retention and live
phone acceptance remain open. No messages were sent during validation.

## Messages stale send/favorite responses — 2026-10-22

Send and favorite completion callbacks now capture the phone-profile generation
and ignore results after an observed profile change. An old send response can
no longer clear a new profile's matching-key draft or edit its outbox/UI.
This does not cancel an already dispatched backend operation. Locked Messages
check, formatting and diff checks pass. Remaining phone-switch work includes
per-profile draft/outbox retention, confirmation dialogs, contact searches,
history/read callbacks and synthetic delayed-response regressions.

## Messages inbox cache on phone changes — 2026-10-22

A reported phone-profile change now forces a recent-inbox fetch and discards
the previous inbox snapshot, timestamp, warning and synthesized inbox-only
rows before merging the new phone data. A failed new inbox read cannot reuse
the old snapshot. Locked `cargo check -p mseries-messages`, formatting and
diff checks pass. This is a partial phone-switch correction: draft/outbox,
in-flight operations and backend identity changes during requests still need
a dedicated review and synthetic-service regression coverage before live
phone-switch acceptance. No owner conversations were accessed.

## Rust app package runtime requirements — 2026-10-22

Messages, System Monitor and T3 Debian packages now require GTK 4.20 and
Granite 7.8, matching the enabled Cargo API features and shared SDK. Previously
the package metadata allowed GTK 4.12 and unversioned Granite, which could
permit installation on unsupported runtimes. Both Granite package-name
alternatives retain the same minimum version. Shell syntax and dependency
version comparisons pass; this metadata correction does not qualify a live
phone or desktop. Next: finish current build/package CI and the outstanding
visual, keyboard/accessibility and owner-phone acceptance.

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

`rust/mseries-messages` is the native client prototype. It uses the shared SDK's
typed BlueFerry client for the versioned session D-Bus `Messages1` API
(currently API v2) for status, history, favorites, read state and checked
sends. BlueFerry remains the only owner of the
iPhone MAP session, recipient/group validation and encrypted message history.
The Rust client never opens BlueFerry's private database or starts a competing
MAP connection.

If BlueFerry's Messages API cannot be reached, the main pane explains the
connection state and offers inline Connect iPhone, Advanced BlueFerry and Retry
actions. Retry checks the service again without leaving the recovery screen. The
running app menu also exposes both setup tools, preserving the existing client's
recovery and group-participant flows.

The preview searches conversation names, recipients, latest previews and loaded
message bodies with Unicode case folding. It shows message history, keeps
per-thread drafts editable offline, supports favorites and read state, and
offers checked replies and a starred-conversation strip. Group sends show a
participant-review dialog. Message bubbles offer Reply and Copy actions, render
safe clickable web
links, format message times in the local calendar, show Bluetooth attachment
placeholders, and disclose text shortened by the phone. The Rust app also has
a new-message picker that searches contacts through BlueFerry's supported
`FindContacts` API, validates direct phone/email destinations, and sends
through BlueFerry's supported `Send` method. New
conversations stay local to the app until BlueFerry reports the resulting
thread. When BlueFerry's existing contact-photo opt-in is enabled, it requests
photos through the supported `GetContactPhoto` method, validates bounded JPEG/PNG
headers in the shared SDK, decodes small in-memory avatars, and drops its cache
when the service photo revision changes. It never enables the preference or
writes photos to disk. It installs beside the existing
Python GTK/Granite Messages app. The Python app remains the qualified client
until the Rust app reaches feature and accessibility parity and passes the
owner's live phone acceptance.

Link previews are now implemented for public HTTPS images (including GIF and
WebP) and YouTube links. Incoming previews require an explicit Load Preview
action; sent links can load automatically. Requests pin DNS validation to
public addresses, limit response bytes, dimensions, animation frames and
in-memory cache, and never save previews to disk or retrieve MAP attachments.

The main window now supports Ctrl+F to focus conversation search, Ctrl+R to
refresh, Ctrl+N to start a message, and a Send with Enter preference matching
the existing Python client. Its automatic-refresh switch also survives app
restarts in a separate owner-only `~/.config/mseries/messages-rust.json` file;
it does not change the qualified Python client's settings schema. It reads and
writes the shared
`~/.config/mseries/messages.json` `enter_sends` setting. When enabled, Enter
sends, Shift+Enter inserts a newline, and Ctrl+Enter remains an alternative;
when disabled, Ctrl+Enter sends and Enter inserts a newline. The GTK text
insertion hook distinguishes a literal newline from an input-method composition
commit, so confirming composed text does not send it. Escape clears a focused
search or returns from a selected conversation to the compact conversation list. Search,
conversation list, favorite control and composer expose accessible names
through the SDK. The conversation header can explicitly copy its recipient
addresses to the clipboard. Conversation rows also provide right-click and
long-press actions to mark an unread thread as read or copy its recipient
address(es), matching the qualified client's quick actions. The Send button
remains available, and Ctrl+Enter is its keyboard equivalent when sending is
enabled. Full keyboard and assistive-technology review is still required.

The SDK also folds text-form iPhone Tapbacks onto the earlier message they
quote, including reaction changes and removals. Unmatched Tapbacks stay visible
as messages. Rust Messages presents reaction symbols with accessible names and
hover descriptions. It preserves BlueFerry's uncertain-transfer error so the
app tells the user to check the iPhone before trying again; a completed MAP
transfer is not represented as a recipient delivery receipt. Live phone
coverage remains part of acceptance.

Outgoing messages appear immediately with a Sending state. After a confirmed
MAP transfer they remain in the timeline until BlueFerry history echoes them;
matching uses the outgoing body count captured at send time to avoid duplicate
bubbles. A known failure removes the optimistic bubble but keeps the draft. An
unknown outcome keeps the bubble and draft, tells the user to check the iPhone,
and does not claim delivery. The in-memory outbox is capped at 128 unresolved
entries per app session.

The application menu exposes a keyboard-shortcuts sheet that reflects the
selected send preference. Ctrl+N starts a message, Ctrl+F searches, Ctrl+R or
F5 refreshes, Alt+Left returns to the conversation list, and Escape clears
search or returns from a compact conversation. Returning to the list clears the
selected conversation and retains its draft. The list starts without an
automatic selection, so a refresh cannot open or mark the first conversation
read without user action.
The SDK also sends the aggregate unread-message count to the Pantheon dock badge
through its shared nonblocking LauncherEntry client.

Rust Messages now imports and reviews bounded Apple pin snapshots, and scopes
the saved order and conversation matches in Secret Service by BlueFerry's
optional stable phone UUID. The Python GTK client reads the same status field
and keyring item, migrating its prior custom-schema/MAC-hash item locally when
it first sees the UUID. BlueFerry PR [#277](https://github.com/erikwb/blueferry/pull/277)
merged at `ff418298e3e9b71b650e0d72b4b1ba61576cf29f`. The latest public
release remains v0.8.1 from 2026-09-15, before the merge, so released packages
still omit the ID. Both clients keep pin import unavailable until a release
with the field is available. A direct switch to Rust before running the updated
Python client leaves the old private keyring item in place; the user can
re-import the snapshot if needed.

The Rust client now fetches the phone's bounded recent inbox through the
released `Messages1.ListRecent` contract using the shared SDK. It requests a
snapshot at startup, manual refresh, reconnect and resume, then every five
minutes while connected if auto-refresh is enabled. Failed inbox reads retain
the last snapshot and show a warning. The in-memory merge matches only one
direct recipient by typed address identity, leaves ambiguous senders in
phone-only conversations, never
guesses group routes, and deduplicates by message handle. No private database or
second MAP session is used.

Full accessibility and keyboard review, narrow-window qualification,
localization, package/launcher handoff, private GUI review and owner-phone
acceptance also remain. Imported pins need the merged BlueFerry API change to
ship in a release and pass live acceptance; MAP does not expose recipient
delivery receipts. The contact picker, Reply/Copy actions, shared
Enter-to-send preference and recent-inbox behavior still need owner-session
interaction review.

## Signal

The selected path is the official signed Signal client with Pantheon
integration. Ready PR [#525](https://github.com/techmore/ElementaryOS_MSeries/pull/525)
keeps Signal's official Electron client and adds system-accent refresh for the
mutable install, desktop launcher identity, and a Wingpanel tray host. Atomic
releases retain upstream colors. All applicable checks for current head
`b6ca6d24` pass. A throwaway, network-blocked Xvfb launch of Signal Desktop
8.32.0-beta.1 showed its onboarding screen in light and dark modes after the
Pantheon palette patch; the installed client was checked read-only and no
account was opened. This verifies only onboarding, not composer or message
states. The PR does not replace Signal's UI with Rust or GTK.

The screenshot shelf's actual compose preview, multiple images, paste fallback,
and XWayland-to-Wayland acceptance remain open under
[issue #227](https://github.com/techmore/ElementaryOS_MSeries/issues/227).
The shelf's source-level retention and portal checks do not prove Signal
attached the image.

Signal Desktop is officially distributed as an Electron/TypeScript application.
Signal's `libsignal` repository says its client APIs are for Signal's own use
and external use is unsupported. A Rust GTK shell alone would not provide
supported access to Signal conversations, encrypted storage, attachments or
sends.

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
