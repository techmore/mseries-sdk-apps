# Connect iPhone (experimental)

Native GTK 4 / Granite 7 Messages and onboarding windows over BlueFerry's existing backend.
Install with `bash mseries/install-phone-setup.sh` after installing BlueFerry's
backend and GTK client. Launch `mseries-phone-setup`; optionally pass
`--ios-version 27.0.1` for diagnostics. The opening page does not ask for an iOS version or require one to continue.

The intended flow is Next → choose a phone → Connect → compare and approve the matching code on both devices
→ grant iPhone permissions → Open Messages. Existing configured phones skip
selection. Polling and Bluetooth requests run in worker threads; close/cancel
stops the owned pairing helper and stale callbacks are ignored. Status checks stop when the target is cleared instead of repeatedly activating an unconfigured backend. Only a live
`map=true` response enables the ready state. Contacts or a completed bond cannot
stand in for message access. A failed helper does not produce a success page.

The app uses upstream's typed JSON command adapters and Messages1.GetStatus.
The user's Connect click approves binding that phone. Numeric Bluetooth pairing
confirmation still requires the user's explicit Codes Match action; it is never
accepted automatically. No messages are sent by the setup app.

## Recovery boundary

- Power on the selected adapter through BlueZ when possible; surface actual
  permission/rfkill errors instead of silently invoking system-wide changes.
- Use BlueFerry's automatic compatibility/pairing strategy and device-class setup.
- Detect missing message authorization and give the iPhone-side instruction.
- For a user-entered iOS 27 version, offer one targeted recovery: restore the
  normal hands-free audio role by setting
  `BLUEFERRY_KEEP_PHONE_AUDIO_ON_PHONE=false` and restarting only BlueFerry.
  Upstream may reload WirePlumber when removing its own audio-role override.
  The page explains that the iPhone may offer this computer as an audio output.
  A previously applied setting is detected and is not applied in a loop.
- Do not repeatedly forget bonds, reset the radio, upgrade BlueZ, or modify
  kernels automatically. iPhone code approval and permission grants remain
  controlled by iOS and cannot be toggled from Linux.

The setup window hands message composition to the native Messages app. Tether and Bubbles are separate apps, not controlled by this
wizard. Localization infrastructure is not present yet.

## Working messages and native desktop — 2026-10-20

**Sending and receiving texts are hardware-verified** on M1 Pro J314S,
elementary OS 9, kernel `7.1.12-2-20261017.20-sep-ARCH`, iPhone 14 Pro / iOS
27.0.1. BlueFerry is pinned to `60fd7ef1abbd25947981f0cf27a521d82784a75f`.
A matched BlueZ 5.87 stack runs with the experimental bearer API. A fresh
Connect-first pairing with Hands-Free exposed produced Classic + LE keys.
The user enabled the newly visible **Show Message Notifications** switch;
MAP then opened and its MNS listener connected. A self-addressed text was
sent and received; an explicitly authorized contact test was sent, followed by
two incoming replies. Contacts remain available. ANCS app notifications remain
unavailable and are not required for these ordinary texts.

Launch **Messages** (`mseries-messages`) for the native GTK 4 / Granite 7
conversation sidebar, contact picker, message bubbles and composer. Sidebar
names are larger; avatars use BlueFerry’s opt-in contact-photo cache with an
initials fallback. The owner requested this opt-in; 34 real photos were synced.
Images are bounded/validated upstream and decoded to small thumbnails in memory. All sending,
recipient validation, encrypted history, group checks and phone sessions stay in
BlueFerry. The new window is a client of that existing daemon, not a competing MAP
client. Advanced BlueFerry remains accessible from its menu. Setup now opens this
native client once message access is ready.

Drafts are kept per conversation while the window is open. Sending is explicit,
shows pending/failure, prevents duplicate clicks, and keeps a failed draft for
retry. The captured thread identity/roster token controls the send even if the
selection changes while it is pending. Unknown or unconfirmed group routes stay
blocked; use Advanced BlueFerry to review their participants. Message text is
escaped, never interpreted as markup; only URLs that BlueFerry's own formatter
judges safe become links (the call its other clients use, which also refuses
text with direction-control characters). Fixture previews cannot send or query
private accounts.

### Native conversation experience

A pass over how the window looks and behaves, without changing what it sends
or where that data lives. Everything below is presentation; BlueFerry still
owns sending, recipient checks, history and group safety.

- **Conversation list.** Colour-coded initials (stable per person, every tone
  at least 4.5:1 with white text), a people glyph for groups, a large name
  (1.4em, bold while unread) with a short time (clock today, then Yesterday,
  weekday, date), a two-line preview that starts with "You:" for your own last
  message, and an accent-coloured unread dot immediately left of the avatar, in
  the list and in favourites, as the owner asked. Bare numbers show the
  person's name when the cached phonebook has exactly one match (see
  `resolve_contact_names`); groups and ambiguous contacts are never guessed.
  Favourites use larger avatars. An empty list says why: No Conversations or
  No Results.
- **Link previews.** Direct HTTPS image and GIF links and YouTube links get a
  card under the bubble (see *Media-link previews* below). Sent links load at
  once; incoming ones offer Load Preview. The card is built by the controller
  because it owns the network pool and cache; `messages_widgets` only places it.
- **Full text, attachments and links.** Messages received while BlueFerry runs
  arrive whole (BlueFerry fetches the full message; a 456-character text and its
  link render intact on this phone). The phone's inbox listing, which fills in
  what arrived while the app was closed, cuts every message at 128 characters
  without saying so. A message that came only from that listing and is at least
  120 characters long is captioned **Shortened by your iPhone · open it there for
  the rest** instead of looking complete; BlueFerry has no call that fetches the
  full body of a listed message, so the app cannot recover it. The iPhone shares
  only the text of a message over Bluetooth, so a photo or file arrives as an empty
  body or iOS's object-replacement mark (U+FFFC). That is drawn as an **Attachment**
  chip (also in the conversation list preview) rather than an empty bubble, with a
  tooltip pointing to the iPhone. No MMS has been received on this phone yet, so
  the chip is verified against fixtures and real-widget tests, not a real MMS.
- **Conversation.** Header with the contact's avatar and addresses. Bubbles from
  the same person within five minutes form a run with tight spacing and a tail
  on the last bubble. Centred time headings appear after a 30-minute silence or
  a new day, using BlueFerry's own wording. Group messages name the sender at
  the start of each run. Hovering a bubble shows its full time.
- **Sending.** A message appears at once, dimmed with "Sending…", and becomes
  solid when the phone accepts it. It is shown only until BlueFerry's history
  contains it, so it is never doubled. A failed send removes the bubble and
  keeps the draft. When new messages arrive while you are scrolled up, a
  New Messages button offers a jump to the latest.
- **Group replies.** BlueFerry refuses to message a group until a client has
  shown who is in it and the user confirmed. Sending to an unconfirmed group now
  opens a native Granite dialog listing every recipient (names when the phonebook
  has exactly one match, otherwise the number), with **Cancel as the default
  button** so Enter can never confirm it. Only the explicit Send button passes
  `confirm_group` and the roster token to BlueFerry; if the roster changes while
  the dialog is open nothing is sent. BlueFerry remembers the confirmation, so
  the dialog appears once per group. A group whose participant list is
  incomplete still goes to Advanced BlueFerry.
- **Why a send failed.** A refused or failed send used to say only "Could not
  send". It now gives BlueFerry's reason in plain words (no longer in history,
  cannot tell who to reply to, group needs confirming, phone denying message
  access, not connected, rate limited) and keeps the draft.
- **Quick actions.** Right-click or long-press a conversation for Mark as Read
  and Copy Number. **Keyboard Shortcuts** in the menu lists every shortcut with
  native keycaps.
- **Composer.** One rounded field with the send button inside it. The button
  slides in when there is something to send and shows a spinner while pending.
  Enter sends and Shift+Enter adds a line by default. **Press Enter to Send** in the
  menu (on by default since 2026-10-21; turning it off is remembered in
  `~/.config/mseries/messages.json`, owner-only, on/off switches only) makes Ctrl+Enter send
  and Enter add a line instead. It is input-method safe: a
  plain Return is noted, and the message is sent only if a literal newline is then
  inserted. When an input method uses the key to confirm a composition, the composed
  text arrives instead and nothing is sent. A Return held longer than 80 ms before its
  newline, a pasted newline and Ctrl+Enter behave as expected.
- **Colour.** Outgoing bubbles use the stylesheet's `accent_bg_color` with
  `accent_fg_color`, so the user's accent colour and dark mode are honoured. The
  older `theme_selected_*` pair was not used for text: it measured 1.8:1 in dark
  mode here.
- **Offline and reconnect.** The footer shows a coloured dot plus words. A banner
  appears when the phone is unavailable or access is pending, with **Retry
  Connection**. That reuses `connection.reconnect_existing`: it checks the saved
  bond, restarts only BlueFerry and never re-pairs or touches audio policy.
- **New Message.** A sheet with recent conversations before you type. Enter starts
  the typed address (or the first match), Down moves into the results and Escape
  closes it. Group conversations are not offered as recipients.
- **Keyboard.** Ctrl+N new message, Ctrl+F search, Ctrl+R or F5 refresh, Alt+Left
  back in the single-pane layout, Ctrl+Enter send.
- **Read state.** Opening a conversation, or receiving into one that is open and
  focused, marks it read once per newest message.
- **Single pane.** Below 760 px wide only one pane shows. The window now reacts
  to resizes instead of running a callback on every frame.

Code layout (all under `phone-setup/`): `messages.py` is the controller (state,
worker thread, BlueFerry calls, dialogs); `messages_model.py` holds pure,
GTK-free logic (message runs, time labels, previews, the send outbox, avatar
tones); `messages_widgets.py` builds the widgets; `messages_style.py` holds the
scoped stylesheet; `messages_settings.py` stores the on/off preferences. The installer copies them all.

Checked with the isolated fixture and Cairo rendering in light, dark and narrow
layouts, with 41 model tests, 5 media tests and 66 GTK tests (one screenshot
test needs its environment variable and skips), including end-to-end tests that
push realistic BlueFerry objects through the real refresh path. **Not verified:**
the redesign has not been run against the live phone, and the owner's open
Messages window keeps the previous code until it is relaunched.

### Crash on refresh and group texts — 2026-10-21

**Window abort.** Message bubbles are selectable labels, so clicking a message puts keyboard
focus inside the conversation. The next refresh rebuilt every bubble and GTK 4.22's AT-SPI
layer aborted the whole app (`Gtk-CRITICAL gtk_at_spi_root_to_ref`, then `GLib-ERROR
g_variant_new … '(so)'`, SIGABRT). A closed window cannot send. `release_focus` now moves
focus out of the list or bubbles before they are rebuilt and restores it afterwards;
`test_rebuilding_bubbles_never_destroys_the_focused_widget` fails without it.

**Group texts under one sender.** iOS gives MAP only the sender (`ConversationId` is always 0
and the recipient is just this phone); group membership comes only from ANCS notifications.
With ANCS off (`ancs:false`, `ancs_authorized:false`, LE bearer failing with
`le-connection-abort-by-local`) BlueFerry files a group text under whoever wrote it. When the
daemon reports ANCS off, a direct conversation's composer says "Sends to this person only"
and explains the cause in a tooltip. Turning on Share System Notifications for this computer
in the iPhone's Bluetooth settings is what lets ANCS authorise; the LE failure itself is a
BlueZ/controller issue tracked with the ANCS work above.

### Reply and Tapbacks — 2026-10-21

The phone's Bluetooth link carries iMessage as plain text only, so real iMessage threads, sent
Tapbacks, typing and read receipts cannot be produced from here. What the text channel can do:

- **Reply.** Right-click (or long-press) a bubble → Reply. A bar above the composer shows
  what is being answered; the message goes out as `↪ “quoted text”` on its own line, then
  your words. The quote also keeps the sent bubble and BlueFerry's echo identical. The
  bar's × cancels; a successful send clears it. Copy Text is in the same menu because the
  menu replaces the selectable label's own.
- **Tapbacks.** An incoming `Liked “…”`, `Derek laughed at “…”`, `Reacted 🎉 to “…”` or
  `Removed a like from “…”` folds onto the message it names as a small emoji chip (tooltip says
  who). It matches the phone's 128-character shortened copy too. One reaction per person, a
  newer one replaces it, and a Tapback whose target is not in the history stays an ordinary
  bubble. Folding works on copies, so history and read-state are untouched.

### Missed messages and sleep

The app queries up to **200 recent inbox entries exposed by the iPhone** when
opened, on a detected message reconnect, on logind resume, after five minutes between successful
queries (wall time includes suspend), and on manual Refresh. The normal status
poll is every five seconds while the app is open; requests run on one background
worker. Live events and inbox snapshots are deduplicated by MAP handle without
changing known group routes. This snapshot stays in memory; persistent live
history remains in BlueFerry's encrypted store. Offline views retain the current
window's last snapshot. The app does not download the full iCloud archive or
recover a complete sent history. A real inbox read returned 11 entries on this
phone; a controlled suspend/missed-message/reopen scenario remains unverified.

**One person, one conversation.** Snapshot messages join a direct conversation by
identity, not by string. BlueFerry's threads write a number as `+19086420729` while the phone's
snapshot reports `19086420729`; those are the same phone (as are a 10-digit number and its
`1`-prefixed form), so they now share one conversation. Before this, such a person appeared
twice and their newest texts sat in a duplicate phone-only conversation while the real one
looked stale. If two threads claim one number the text goes to a single phone-only
conversation rather than to either, and a sender that is not a valid recipient is still dropped.

### Verification-code offers

A separate native helper detects recent SMS verification codes and offers them
near the pointer. Its shortcut activates the card; Enter inserts into a supported
native field, or explicitly copies for Brave. Browser direct fill remains open.
Install/undo, qualification and privacy details: [Verification Codes](VERIFICATION-CODES.md).

### Apple Messages pins

Messages → menu → **Import iPhone Pins…** accepts the private JSON generated by
our macOS Apple pin-store helper. The preview preserves Apple's order and suggests only
unique exact individual-address matches. Choose the current conversation for
ambiguous people or groups; the recipients appear below each choice. Unmatched
pins remain available through **Review Imported Pins…**. Re-importing the same
Apple identifiers retains prior explicit matches even if Apple reorders them.
Pins appear as four contact tiles per row above ordinary conversations, with a
horizontal separator, photos/initials and the export time visible. This is
a snapshot, not live phone synchronization, and importing does not send texts
or mark conversations read.

Order, source and mappings are saved in an app-owned desktop keyring item bound
to the configured phone. BlueFerry's existing stars/preferences remain separate.
The importer adds a libsecret GI dependency (`gir1.2-secret-1`). A locked or
unavailable keyring reports failure and retains the preceding import.

The helper uses macOS's built-in JXA/Foundation to read IMCore's local pin
configuration or its canonical preferences cache. It installs no developer tools
and changes no SIP/signing/privacy settings. Missing and denied access remain
distinct from a verified empty pin list. Native binary-plist reading, private JSON
export and failure handling pass on ARM macOS 15/26 and Intel macOS 26. These are
fixture checks; owner-account export/import, SIP-enabled execution and NAS
transfer are **not hardware-qualified**. All CI images reported SIP disabled.
The kit verifies the exported file/checksum before writing its completion receipt;
that receipt certifies collection, not Linux transfer or live pin sync. See the
[source and qualification record](PIN-SYNC-RESEARCH.md).

### Persist and undo the Bluetooth stack

`bash mseries/install-messaging-bluez.sh --install BLUEZ_BUILD_DIR` installs the
reviewed three binaries under `/usr/local/libexec/mseries-bluez-5.87/`, writes
scoped system/user drop-ins, and preserves an already running successful trial
without restarting it. On a different installation use `--restart` explicitly.
`--rollback` removes only these drop-ins and restarts distro services, retaining
the current bond and contact data. Distro executables are never overwritten.
The current successful machine has the persistent configuration installed.

BlueFerry probes `bluetoothctl --version` to select pairing policy, so a 5.87
daemon with distro 5.85 CLI still incorrectly selects compatibility mode. The
scoped backend PATH and native launchers use the matching 5.87 client. See
[the upstream review](UPSTREAM-REVIEW.md) for build flags, sources and limitations.
The change is installed on this desktop; no factory-image Bluetooth upgrade or
reboot qualification is implied.

## Download the macOS visit kit

**One guided script now collects both Bubbles activation and Machine Lab files.**
On native macOS, open Terminal and run these two lines:

```sh
curl --fail --location 'https://raw.githubusercontent.com/techmore/ElementaryOS_MSeries/adde4b8c27ffd27fd7b0995ff0650d7b65f7e7f6/mseries/phone-setup/macos-visit/Collect-Everything.command' -o "$HOME/Downloads/Collect-Everything.command"
/bin/bash "$HOME/Downloads/Collect-Everything.command"
```

Before starting: AC power, internet, native Apple Silicon macOS 13+, **100 GiB
free on macOS and 100 GiB free on shared exFAT storage (internal or external)**. No Homebrew is needed.
The script selects an existing exFAT folder; it never formats a drive.
Without an external drive, use a separate internal exFAT partition. A proposed
120 GiB shared partition must be created separately by shrinking the macOS APFS
container after checking its capacity and backups. This is not the same as adding
an APFS volume. No partition changes have been performed or automated. The
existing macOS data volume cannot be unlocked by this Linux setup; Linux root
currently has only about 11 GiB free.

It downloads and checks the Mac helper, macosvm and Ventura restore image; guides
you through the full OABS hardware export; collects native firmware/provenance;
restores the guest; starts it for a graphical check and guest shutdown; and
re-reads checksums for every transferred file. Completed steps and interrupted
downloads resume. An incomplete restore is preserved rather than overwritten.

Your actions: choose the shared folder; in Mac Hardware Info turn **Prevent
sharing OFF** and **Copy Activation Code**, then return to Terminal; when the VM
opens, wait for Setup Assistant and use its Apple menu to **Shut Down**, then
confirm that result in Terminal. Do not sign into Apple in that VM.

Wait for **COLLECTION COMPLETE** and `COLLECTION-COMPLETE.txt` in the shared
`MSeries-Linux-Return` folder. Eject external storage if used, then return to Linux. We complete
Bubbles login/2FA there. Keep the private export and guest identity outside GitHub
and chat. Downloads, restore and full disk checks can take hours.

[Detailed collection behavior and manual fallback](macos-visit/README.md).
Shell checks and eight mocked failure/resume cases pass. Native macOS execution
remains untested here. The receipt certifies collection, owner-confirmed native
startup and transfer; Linux KVM boot, extracted-booter compatibility, Bubbles
activation and dock save/restore remain unverified. This gathers the known
macOS-only inputs; future experimental work may still require another visit.

## Bubbles personal installation

[Source](https://github.com/chrispouliot/bubbles/tree/9c677ec71ffc01eb87f8a7daf6f08289c03b61f3),
SSPL-1.0 with rustpush. Built with real default backend using
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=3 cargo build --locked` on ARM64.
Installed as `~/.local/bin/bubbles` with its upstream desktop entry and icons.
438 tests pass, 1 ignored after installing `libheif-plugin-libde265`; the two
initial HEIC failures were missing system decoder plugins, not activation tests.
GTK startup was verified; Apple activation and real messaging remain untested.

Build dependencies added: GStreamer/base development headers, libheif-dev,
protobuf-compiler; the local toolchain already supplied GTK/libadwaita and Clang.
The upstream NAC component was cached outside the repository and its expected
SHA-1 verified. No Apple binary, FairPlay material or hardware token is vendored
by this integration. This is a personal source build, not a factory-image payload.

Use official Mac Hardware Info v1.2.0 on the owner's native macOS partition:
turn Prevent Sharing off, copy the full base64 OABS blob, and save it privately
on cross-OS storage. Bubbles' local UI does not resolve short MB sharing codes;
its 517-byte raw-validation importer is stubbed in this source revision.
The Mac may then be shut down and Apple Account login completed on Linux.
No Apple verification challenge should be started before switching OSes.

## Validation and rollback

`python3 mseries/phone-setup/test_setup.py` verifies readiness, bounded recovery
and failure decisions. `--preview permissions|confirm|ready` renders isolated,
credential-free fixture states and never touches Bluetooth. Light/dark previews
and the live initial window were checked with Cairo rendering (the host default renderer reported a surface-lost error); a complete pairing through this new
window remains unqualified while the phone permission issue is unresolved.

Remove `~/.local/bin/mseries-phone-setup`, its desktop entry
`~/.local/share/applications/io.elementary.mseries.PhoneSetup.desktop`, and
`~/.local/share/mseries/phone-setup` to remove the wrapper. BlueFerry's pairing
and encrypted data remain untouched. Restore the previous BlueFerry audio policy
from `~/.config/blueferry/local.env.before-ios27-hfp` if needed, then restart its
user service; that backup contains private device configuration and must not be
committed. Restore only the audio setting if the phone has been re-paired since
the backup was made.

During live setup a cleared BlueFerry target made status polling repeatedly
activate an unconfigured backend. The guided window was stopped, the previously
approved phone's still-trusted bond was matched against the private backup, and
only the application target was restored. No additional Bluetooth pairing was
needed. `test_protocol.py` verifies that status does not activate a backend when
no target is saved. The corrected guided flow remains staged/offscreen while
message authorization is unresolved.

## Always-available status without repeating setup

`mseries-phone-status` (or `--json`) is installed alongside the wizard. It reads
live BlueFerry and BlueZ state without starting an absent daemon, pairing,
resetting a connection or sending a message. It distinguishes a live paired
bond, message access, the contact profile, cached-contact count and unobservable
iOS switch positions. The last verified message result survives transient
reconnect attempts so the interface does not alternate between a permission
failure and an unexplained new error.

Status changes, transport history and per-phone recovery steps are recorded
privately under `~/.local/state/mseries-phone-setup` (mode-0600 files, no message
contents, no phone addresses in status output). Recovery history is scoped to
a private hash of the configured Bluetooth target. The wizard checks live status
when opened with an existing target, refreshes while open, and shows the prior
recovery results instead of asking the same toggle/pairing questions again.
The iPhone switch positions remain explicitly `not-observable`; access results
must not be presented as a remote read of those switches.

Live latest result: trusted bond connected, **727 contacts cached**, PBAP open,
MAP still denied. This is partial progress, not working message delivery.

## Native retry and rollback, 2026-10-20

The failed HFP phone-audio workaround was rolled back to its saved original
preference on the live desktop, retaining the phone target, bond and 727 cached
contacts. That rollback is recorded alongside the failed recovery attempts.

The native app now offers **Retry Connection** when messaging is blocked or the
backend is stopped. This verifies the saved bond, restarts only BlueFerry and
waits for its message-access result. It does not forget/re-pair the phone or
rewrite audio preferences. Missing bonds fail without making changes. Closing
the window cancels subsequent checks; status polling remains read-only. A
manual retry is distinct from Refresh Status. Previously denied access never
becomes a success just because the service restarted or contacts connected.
Inactive, unavailable and disconnected states are presented separately, and
recorded failed recoveries suppress repeated toggle instructions.

Deep live diagnostics: the iPhone exposes MAP 1.4, RFCOMM channel 2 and L2CAP
PSM 4107. SDP lookup and the transport connection succeed; the iPhone returns
OBEX Forbidden (wire response 0xc3, BlueZ error 0x43) to CONNECT. Explicit
RFCOMM/L2CAP, fresh obexd after a quiet interval, basic OBEX 1.0/1.5, feature
parameters, SRM and a hands-free connection did not establish MAP. No message
was sent. Raw captures remain private outside Git. These observations narrow
the failure but do not prove which iOS permission or policy caused it.

18 native model/protocol/reconnect checks pass. The installed retry was exercised
against the live phone: bond retained, message access still denied. This is
not working text messaging; no end-to-end delivery is qualified.

### Media-link previews

Messages renders direct HTTPS JPEG/PNG/WebP images, animated GIFs, and YouTube
thumbnail/title cards. Original image dimensions are shown; the YouTube dimensions
refer to its thumbnail, and Watch on YouTube opens playback in the browser.
Sent media links load automatically. Incoming links offer Load Preview unless
already cached or loading. This does not add MAP attachment payloads.
Downloads use validated public HTTPS endpoints, bounded sizes/decoded frames and
an in-memory cache. No authentication cookies or private/LAN endpoints are used.
Private rendering of the actual self-test assets passed: photo 500×477,
GIF 400×400/44 frames with observed GTK frame advancement, YouTube thumbnail
480×360. All three full text links were received back through the phone.

### Native frontend integration prompt

```text
Continue our existing Messages app as an elementary GTK 4/Granite 7 frontend
for BlueFerry. Reuse its backend and typed client APIs; preserve working pairing,
message routing, contacts/photos, read state, history and send/retry behavior.
Keep four favorite contacts per row with a separator, large readable names and
blue unread dots left of avatars. Resolve names through unique cached phonebook
identities; never guess group recipients or ambiguous contacts. Support bounded
photo/GIF and YouTube link previews. Distinguish link previews from unsupported
MAP attachments, and Mac-exported pin snapshots from live iPhone pin sync.
Preserve drafts, focus, selection, light/dark themes and narrow layouts. Test the
actual behavior before claiming parity. Commit/push coherent changes and open
PRs against dev. Preserve the running Bluetooth, audio, kernel and panel setup.
```

### Running the GUI tests safely

`test_messages_gui.py` opens and closes dozens of windows and sends key events, so it must
never run against a live desktop. Run it only as `xvfb-run -a python3 test_messages_gui.py`.
`gui_safety.py` enforces this: the module skips itself unless `DISPLAY` is an isolated virtual
display (`:90` or higher, which is where `xvfb-run -a` starts), and it forces the X11 backend and
removes `WAYLAND_DISPLAY`. The second part matters even under `xvfb-run`: with a Wayland session
in the environment GTK otherwise opens the test windows on the real desktop.
