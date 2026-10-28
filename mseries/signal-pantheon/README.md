# Signal Pantheon 0.5.1

A native GTK 4 / Granite messaging client for elementary OS, using the local
signal-cli service. This is an unofficial client, not a Signal Desktop release.
Pairing and live-account verification are deferred at the owner's request.

## Local features

- Contact and group discovery, refresh, conversation details, and new direct
  conversations by full phone number or UUID.
- Conversation filtering, message/attachment-name search within a conversation,
  unread counts, and loading older messages.
- Multiline composition: Enter sends, Shift+Enter adds a newline. Draft text and
  a selected attachment survive conversation changes and application restart.
- Text and one file per send (up to 100 MiB); outgoing files are copied into a
  private staging folder before sending. Downloaded incoming files can be saved
  through a native file chooser. No file opens automatically.
- Editing sent text, deletion requests with confirmation, six reaction choices
  and removal, incoming edits/deletions/reactions, and direct delivery/read status.
- Opt-in desktop notifications with generic content, and opt-in read receipts
  sent while viewing the conversation in the active window. Both default off.
- Encrypted conversation snapshots, drafts, metadata and catalog. The previous
  text-only history format migrates locally when its conversation is discovered.
- Serialized RPC requests, bounded waits, reconnect feedback, duplicate-send
  protection, partial-send reporting, and stale socket-reader isolation.

- One-to-one voice/video calls: outgoing/incoming controls, accept/decline/hangup,
  microphone/speaker mute, camera toggle, remote video, and reconnect/error feedback.
  Incoming calls leave the camera off until you enable it. An outgoing Video call
  starts the camera after connection. Closing the video window hides that view;
  End call in the main window ends the call and stops audio/camera capture.
- Group voice/video calls: join/start from a group conversation, SFU authentication
  and membership mapping, participant/mute state, media-key signaling, per-device
  video tiles, activity announcements and leave/hangup. Microphone/camera state
  is synchronized to RingRTC. Live group interoperability remains unverified.
- View-once photos/videos: select one media attachment and View once with no text.
  Sent copies are consumed immediately; received media opens in a native viewer
  after consumption is durably saved. Opened-device sync, optional viewed receipts,
  replay protection and a 30-day unopened timeout are included. The viewer has no
  Save action. App-owned copies are removed; the original selected file remains.
- Disappearing-message timers: per-conversation settings, per-message countdowns,
  outgoing start times, incoming read-triggered expiration, linked-device read
  sync even with remote read receipts off, restart recovery, and owned-file cleanup.

## Build and run

Build dependencies: GTK 4.12+, Granite 7.7+, JSON-GLib, Cairo, SQLite, libsecret,
OpenSSL, GStreamer core/app (including PulseAudio, video conversion/capture and
codec plugins, including H.264/MP4 decoding), GTK media backend, Meson, Ninja, a C compiler
and Vala. Voice calls require a working PulseAudio-compatible server. The local
ARM64 runtime includes Java 25, patched signal-cli 0.14.9, libsignal and the
RingRTC call tunnel. See `vendor-src/README.md` for source/build provenance.

The default build uses Granite 7. To build against the elementary OS 9
development SDK, install `libgranite-9-dev` and configure a separate build
directory with `meson setup build-granite9 -Dgranite_major=9`. Granite 9
requires GTK 4.22 or newer; the build checks the library's pkg-config
requirements. Use only one Granite major version in a process. The Granite 9
build follows GTK's system color-scheme setting; verify desktop appearance on
the target system before shipping it.

```sh
meson setup build
meson compile -C build
meson test -C build --print-errorlogs
./run-local.sh --diagnose
./run-local.sh
```

The launcher accepts `SIGNAL_CLI_PATH` and `JAVA_HOME`, an installed signal-cli,
or `SIGNAL_PANTHEON_RUNTIME` containing `temurin25/` and `signal-cli-arm64/`.
On this development host it also recognizes the earlier prototype's runtime.
The source archive excludes the runtime and compiled app; the working project
contains the staged runtime. Nothing is silently downloaded or updated. The working runtime also includes
an app-local GStreamer libav plugin for H.264/AAC; it uses the host FFmpeg shared
libraries. The plugin does not change the system GStreamer installation. `runtime-manifest.json` identifies the checked
local runtime; service compatibility must be rechecked when upgrading it.

Tests additionally require Python 3 with GI, Xvfb, xdotool, dbus-run-session and
gnome-keyring-daemon. The UI fixture uses its own bus, keyring, profile, display,
notification service and fake signal-cli. It does not pair or send real messages.

## Install

Inspect an installation without changing the running desktop:

```sh
DESTDIR="$PWD/stage" meson install -C build --no-rebuild
```

For an explicitly chosen local installation, run `./install-local.sh` (defaults
to `~/.local`) or provide an absolute prefix. It builds, tests and installs the
binary and launcher, and leaves an existing app process alone. Installed launches
use the staged runtime installed under the chosen prefix when present; otherwise
they need a compatible runtime in the session environment. This work has
only staged installation; it has not installed or launched a paired client.

## Data and behavior

Account and backend files live under `$XDG_DATA_HOME/signal-pantheon` (normally
`~/.local/share/signal-pantheon`), separate from Signal Desktop/Beta. The app
communicates over a private Unix socket under `$XDG_RUNTIME_DIR` and manages its
own daemon. Message history uses AES-256-GCM with a key from Secret Service;
conversation lookup keys use HMAC. Drafts, paths and names are in encrypted
snapshots. Notification/receipt preferences are ordinary config booleans.

Attachment bytes and signal-cli's own profile/cache files are managed separately
and are **not encrypted by this app's history encryption**. Saving an attachment
creates a normal file at your chosen destination. Deletion replaces the local
message with a tombstone after confirmation; it is not a forensic erasure of
SQLite/WAL files, downloaded attachments, exports or other devices.

A timed-out send has an unknown outcome. Check before retrying; the client never
automatically retries a message. Group partial results remain visibly partial.
Direct receipts do not imply that every group member read a message.

## Deliberate limits

Screen sharing, call links, complete group administration, safety-number UI,
registration, multi-account selection and complete multi-device reconciliation
remain unsupported. Group calls require a current Signal v2 group with valid
membership credentials. Calls, timed messages and view-once sync use the bundled
backend extensions; the corresponding controls are gated with older backends.
This local build does not claim complete official-client parity.

Camera capture uses the selected GStreamer camera source at 320×240,
15 fps. Received video is bounded/scaled to 1280×720; up to 16 remote video tiles
are displayed. The call video window includes a local self-preview and camera selector.
Device discovery runs off the UI thread; choosing a camera while it is off does
not start capture. Switching an active camera stops the previous source first.
Unavailable cameras leave capture off with an error; no fallback camera is
opened. The preview uses the transmitted capture stream and clears on camera-off
and teardown. Selection lasts for the call and resets to the default afterward.
Screen capture and advanced call moderation are not implemented. Group calling has a real RingRTC/SFU path,
but successful live group joining and multi-party media need pairing to verify.
Incoming group rings show a Join/Decline prompt and an opt-in desktop notification.
The signaling-only listener opens no media resources. Current v2 group membership
and sender identity are checked; stale rings, duplicates and replay after local
closure are suppressed. Cancellation, timeout, busy state and linked-device
accept/decline/busy responses dismiss pending prompts. Accepting starts the group
media path with the camera off. No separate audible ringtone is implemented.
Join can create a new call if the earlier call has ended.

View-once availability is consumed before rendering, including on a viewer error
or process interruption after the commit. Opened sync is queued durably and can
only reach other devices after connectivity returns; simultaneous offline opens
across devices cannot be prevented locally. Playback depends on installed codecs.

Expiration removes the message from the active encrypted snapshot and deletes
app-owned attachment copies after the snapshot commits. Shared files stay until
no unexpired record references them. Files saved outside the app remain intact.
Tombstones prevent late events from restoring expired content; cleanup retries
after failures/restarts. A monotonic clock prevents backward wall-clock changes
from extending running timers; restored already-started timers are conservatively
removed if a clock rollback is detected. This is logical deletion, not forensic
erasure of storage, exports, backend caches or other devices.

There is no historical backfill or Signal Desktop database import. Old prototype
group messages lack sender identity and retain that limitation after migration.
History search decrypts the selected conversation in memory; very large histories
and multi-day operation are not performance-qualified. The current snapshot store
rewrites a conversation on save. Localization is not yet wired.

## Verification

Nine Meson suites pass: RPC lifecycle/concurrency/timeouts; typed event parsing;
conversation mutations/commands; encrypted history; attachment path/cleanup;
native UI; disappearing-message lifecycle; voice-call controller; and view-once
consumption, persistence, replay, linked-open ordering and unopened expiry.

The native fixture also checks camera switching, missing-camera recovery,
preview clearing and queued-frame suppression, and choosing a camera before
explicit activation on incoming calls. Camera sources remain synthetic.
It exercises incoming group prompts, cancellation/replay,
acceptance, decline and acceptance on another device. A real signaling-only child
probe checks official ring protobufs, signed IDs, stale/malformed messages and
linked responses, with no socket/audio/video descriptors. Java tests cover ring
policy and real child restart. These checks use synthetic envelopes, not live
Signal delivery.

The native fixture exercises direct voice/video and group-call controls, incoming
camera-off behavior, camera toggling, video rendering, and photo/video view-once
playback after the underlying owned file is deleted. Light/dark/narrow/long-text
renders are retained. A separate real-media probe connects two local RingRTC
peers on linked-device IDs 2 and 3, passes synthetic audio/video through the
production bridges, checks mute/unmute and removes owned virtual devices.
A real group client sends authenticated SFU GET/PUT requests to a mock endpoint
and rejects 401 responses without joining; this is a negative transport check,
not a successful live group-call test. The staged daemon starts with an empty
profile and reports all six feature capabilities. Java and Rust tests also pass.

Tests use private profiles, displays, buses, keyrings and audio servers. No hardware
camera/microphone or paired account was used. Pairing, live network delivery/calls,
successful live SFU group media, native Wayland, real file-picker interaction,
accessibility and sustained daily use remain unqualified. Builds retain GTK
and generated-C deprecation warnings.

See `test-results.json`, `screenshots/`, and `STATUS.md` for retained evidence.

## Licensing

The app project has not been assigned a distribution license. The QR generator's
MIT copyright/license is retained in `src/qrcodegen.h`. signal-cli is GPL-3.0;
its libsignal component and RingRTC are AGPL-3.0-only. The local working tree
includes runtime binaries and retained third-party notices; runtime binaries are
excluded from the app source archive. Corresponding pinned sources and patches
are retained under `vendor-src/`. Upstream source locations and component revisions are recorded
in `ARCHITECTURE.md` and `runtime-manifest.json`. Signal's name and marks are not
granted by those code licenses. This is a local development build, not a public
release or an official Signal client.
