# Signal Pantheon architecture

The GTK 4 / Granite frontend delegates account protocol and cryptography to
signal-cli 0.14.9. It does not reimplement Signal Protocol or reuse the Electron
renderer. The separate local backend speaks JSON-RPC over a private Unix socket.
Device-link code remains available, but the continuation intentionally does not
pair an account or send real traffic.

## Components

- `main-window.vala`: native sidebar/detail UI, composition, actions, asynchronous
  file choosers, notification/receipt preferences and user feedback.
- `message-composer.vala` and `appearance.vala`: multiline keyboard behavior and
  shared light/dark styling used by production and rendering tests.
- `signal-backend.vala`: daemon lifecycle, serialized RPC transport, timeout and
  disconnect handling, and bindings to encrypted storage.
- `message-event.vala`: typed, null-safe parsing for ordinary/synced messages,
  edits, deletions, reactions, direct receipts, attachments and timer updates.
- `message-commands.vala`: RPC parameter construction and explicit success/partial
  result checking against the staged signal-cli source.
- `conversation.vala`: messages keyed by timestamp and sender, mutation ordering,
  tombstones, reactions, direct receipt state, drafts and versioned serialization.
  Up to 2048 unmatched mutations/receipt entries are retained per conversation.
- `client-store.vala`: account-scoped encrypted catalog/conversations, contact
  aliases and migration from the earlier text-only history API.
- `history-store.c`: SQLite with HMAC lookup keys and AES-256-GCM payloads. Secret
  Service holds the key. Conversation/catalog snapshots commit in one transaction;
  replacement uses savepoints so failures preserve the previous state.
- `call-controller.vala`: account-scoped call subscription, exact 64-bit IDs,
  acceptance gating, busy rejection, state ordering and teardown.
- `call-video.vala`: private full-duplex bounded RGBA stream, explicit camera
  capture, frame backpressure and GTK memory textures; no video frames on disk.
- `call-media.vala`: GStreamer microphone/speaker bridges to the RingRTC tunnel
  virtual devices, with per-call mute and failure cleanup.
- `expiration-service.vala`: monotonic scheduling, durable-before-delete cleanup,
  countdown updates, clock-floor checkpoints and retry.
- `attachment-files.vala`: local attachment resolution matching signal-cli's ID
  sanitization, rejecting symlinks and ambiguous fallback candidates.

History snapshots encrypt content, draft text, attachment references, contact/group
metadata and unread state. Received attachment bytes remain in signal-cli's
attachment directory; outgoing bytes are staged in private per-send directories.
These files are ordinary local files, outside the encrypted history payloads.
No attachment is automatically opened or executed.

Edits and deletions replace the current snapshot instead of keeping a plaintext
change log. Deletion is a logical state update, not guaranteed disk/WAL erasure.
Unknown-author mutations cannot alter a different author's same-timestamp message.
Older prototype group messages did not record the original sender and cannot gain
that information during migration. Pending mutations handle arrival before the
original message; reaction version stamps prevent older events reversing a newer
reaction removal. Delivery receipts never regress an already-read direct message.

The frontend accepts one account per profile and rejects an ambiguous multi-account
profile rather than silently choosing a different account. Explicitly mismatched
account events are ignored. Contact phone/UUID aliases are reconciled locally.
Profiles with full multi-device/group state still need live qualification.

## Validation boundary

The native fixture is a separate executable compiled with `UI_TEST`; production
contains no fixture hooks or keyring bypass. It runs the same UI, transport,
storage, command and model code against fake Signal/notification services using
a private D-Bus server, display, temporary profile and test keyring. The C history
test accesses a private in-memory test key by including the implementation in its
own executable; the production API provides no key override.

Snapshots still serialize/rewrite a conversation on the GTK thread. Very large
histories need an indexed encrypted message store, worker-thread I/O and measured
performance before any scalability claim. Visible history is paged in batches of
100, while search uses the selected in-memory conversation.

Timed messages preserve their original TTL and earliest read/start timestamp.
Edits and conversation-default changes do not extend existing messages. Linked
read sync is durable and independent of optional remote receipts. Expired-message
tombstones and pending read markers survive restart. Cleanup only targets owned
paths and rejects symlinks; references shared by a live message defer deletion.

Voice signaling remains in signal-cli, with RingRTC performing media encryption
in the separate tunnel process. The app bridges the selected system audio devices
to the tunnel without changing system defaults. Incoming calls cannot start the
microphone before local acceptance; incoming cameras remain off. Video frames
travel through a private Unix socket in a per-call temporary directory; the
socket is mode 0600, and its directory is removed at teardown. The tunnel bounds,
scales and rotates frames, dropping frames when a consumer is slow. The app's
camera capture starts only for an explicit outgoing video call or camera toggle.

Group calls use RingRTC's native group client with the production Signal SFU URL,
current v2-group credentials and encrypted member IDs. Java handles HTTPS requests
with a pinned SFU host, no redirects and bounded waits; opaque group signaling
is sent only to current authorized members and requested recipient subsets.
Connected state requires both joined and connected RingRTC states. Group activity
announcements are sent after joining. The native RingRTC wrapper retains the
outgoing video track across FFI construction, fixing a sole-owner lifetime bug
that otherwise crashes group-client setup. That fix is exercised by the real
group-client mock-SFU probe. Live-account interoperability is deferred.

View-once records retain an encrypted reference only while available. Consumption
is committed before bytes are read/rendered; a pre-opened file handle survives
owned-file unlink during cleanup. Pictures use memory textures and videos use
Gtk.MediaFile over a memory input stream. No export action is offered. The durable
open-sync queue retries independently of optional remote viewed receipts. Replayed
originals and linked-open-before-original events cannot restore a consumed view.
Unopened records expire after 30 days, or earlier if their ordinary timer is due.

## Upstream provenance

The retained development source used for protocol field verification is:
`/home/sdolbec/Documents/Codex/2026-10-20/claclacladfsfs/work/signal-cli-source`.
Checked Java records/commands include JsonMessageEnvelope, JsonSyncDataMessage,
JsonEditMessage, JsonReaction, JsonReceiptMessage, JsonAttachment, JsonContact,
ListGroupsCommand, SendCommand, SendReactionCommand, SendReceiptCommand,
RemoteDeleteCommand, SendMessageResultUtils, and AttachmentStore.

- Signal Desktop: https://github.com/signalapp/Signal-Desktop
- signal-cli: https://github.com/AsamK/signal-cli
- signal-cli JSON-RPC: https://github.com/AsamK/signal-cli/blob/master/man/signal-cli-jsonrpc.5.adoc
- libsignal (matching staged library): https://github.com/signalapp/libsignal/tree/v0.103.0

- Call tunnel: https://github.com/visigoth/signal-call-tunnel
- RingRTC: https://github.com/signalapp/ringrtc

Runtime pins, hashes and source locations are in `runtime-manifest.json`. Runtime
binaries are staged in the working project but excluded from the source archive.
The backend patch adds capability discovery, original outgoing expiration start
and per-recipient TTL reporting, read/view sync fields, explicit timestamped
linked-read sync, and graceful tunnel teardown. Tunnel changes preserve ICE sender
device IDs and bound audio-device readiness. See `vendor-src/README.md`. No
sponsor-only content was imported, and upstream license notices are retained.

## Protocol references

The SFU URL and empty HKDF-extra-info convention were checked against Signal
Desktop's `config/production.json` and `ts/services/calling.preload.ts`.
The inspected Desktop revision is recorded in the runtime manifest. RingRTC's
pinned native sources define group membership, opaque messages and frame crypto.
View-once behavior reference: https://support.signal.org/hc/en-us/articles/360038443071-View-Once-Media .
