# iPhone Messages pinned conversations — 2026-10-20

Requested behavior: show the same pinned people/groups, in the same order, in
native MSeries Messages. These are Messages conversation pins, distinct from
Phone favorites, contact groups and pinned links inside a conversation.

**Result:** Apple implements pin storage and cross-device synchronization, but
no supported public API for reading another app's Messages pins from Linux was
found. Public projects expose private Apple interfaces on macOS or jailbroken
iOS. Our current BlueFerry connection does not import or export iPhone pins.
No phone pin data was read or changed during this investigation.

## Implemented snapshot path and remaining owner qualification

The owner selected the private macOS framework path. `macos-visit/Export-Message-Pins.js`
now implements an original JXA/Foundation reader with runtime return-signature
checks and a permission-aware read of Apple's local pin cache. It does not call
the void fetch selector or extra synchronization/migration methods. Native
Messages has an ordered import/review interface, explicit group matching,
re-import identity retention, snapshot provenance and app-owned phone-scoped
keyring persistence. This is an initial snapshot implementation, not live sync.

Linux qualification: 12 matching/import model tests, 8 existing message model
tests, 14 private GTK interaction/render checks and a real isolated Secret
Service round trip passed. Light/dark/narrow fixture screenshots were inspected.
The installer staged the update while leaving the running Messages PID and
working MAP session unchanged; the current window loads it at next launch.

The earlier interpretation of `process_supported=false` as a decisive read gate
was too strong. The private implementation gates controller initialization, but
has a distinct local configuration getter; Beeper's current Mac client reads the
same pin data from preferences. The adapter now tries the getter, then the
canonical `~/Library/Preferences/com.apple.messages.pinning.plist`, extracting
only `pD` and its ordered `pP` identifiers. The `pT` Apple modification time and
supported configuration version are checked; a missing/inaccessible cache does
not become an empty successful export. Normal macOS file permissions are
respected, without Beeper's alternate-case preference-domain workaround.

macOS qualification at `4d878a07d4f70aa5ee0641af4a1179763aecb0e4`:
[Actions run](https://github.com/techmore/ElementaryOS_MSeries/actions/runs/39462452180)
passes on macOS 15 ARM, macOS 26 ARM and macOS 26 Intel. Tests exercise actual
Foundation binary-plist reads, ordered/empty data, denied/missing reads,
JSON export/readback and mode 0600. Apple-documented bridged NSError handling
fixed a real failure-path crash; an explicit nil check prevents parsing nil data.
No-account probes report configuration version 2 and a missing cache (Cocoa 260),
with the unsupported standalone role retained as diagnostic information.
**All three runners report SIP disabled by their image configuration.** These
checks do not prove a signed-in owner export, owner Full Disk Access, SIP-enabled
execution or cross-OS NAS transfer. The helper itself makes no SIP, signing or
privacy-setting changes. Real owner export/import and live pin sync remain
unqualified; no real pins, account secrets or Apple implementation code enter Git.

## Apple documentation and public implementations

| Source | Evidence and limit |
| --- | --- |
| [Apple Messages guide](https://support.apple.com/guide/iphone/keep-track-of-messages-iphe9b48b89e/ios) | Conversation pins and pinned shared content are separate features. The guide documents the user interface, not a pin-export API. |
| [Apple Bluetooth profiles](https://support.apple.com/en-us/102842) | MAP provides message browsing, composition and replies. This document does not advertise conversation-pin synchronization. The [MAP specification page](https://www.bluetooth.com/specifications/specs/message-access-profile/) was checked, but the full specification download was unavailable to this audit; no exhaustive protocol-level absence claim is made. |
| [Apple NSUbiquitousKeyValueStore](https://developer.apple.com/documentation/foundation/nsubiquitouskeyvaluestore) and [entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.ubiquity-kvstore-identifier) | The public API shares app-specific settings between instances of an app on Apple devices. Enabling it for our app does not grant access to Messages' store. |
| [Apple MSConversation](https://developer.apple.com/documentation/messages/msconversation) | Supports a Messages extension's current conversation, participants and message insertion; it does not document enumeration of the user's pinned conversations. |
| [Apple TelephonyMessagingKit](https://developer.apple.com/documentation/telephonymessagingkit) | Provides carrier SMS/MMS/RCS for an entitled default iPhone messaging app, with regional restrictions. It is not a Linux interface or a documented iMessage-pin reader. |

The important private component is `IMPinnedConversationsController` in IMCore.
The public [iOS 18 runtime-header record](https://github.com/qingralf/iOS18-Runtime-Headers/blob/cfdd406d3a0bbac8473571dda5482e155b1cc402/PrivateFrameworks/IMCore.framework/IMPinnedConversationsController.h)
contains an ordered pin set, pin indices, chat metadata, a
`NSUbiquitousKeyValueStore` data store, external-change handling and local/cloud
synchronization methods. This supports investigating the dedicated settings
store rather than assuming ordinary message-history import contains pins.
It is reverse-engineered interface evidence, not current iOS 27/macOS 27
runtime qualification or an Apple-supported API contract.

Public repository checks:

- **Beeper platform-imessage**, MIT licensed, revision
  [`79383860ff40b5c146b209bd438b33e05b0a6d22`](https://github.com/beeper/platform-imessage/blob/79383860ff40b5c146b209bd438b33e05b0a6d22/src/IMessage/Sources/IMessage/Defaults.swift):
  its pin reader identifies the `com.apple.messages.pinning` domain and `pD` →
  `pP` dictionary path. The source notes privacy protection on Sonoma and uses
  an alternate-case domain workaround. We use the factual schema reference,
  retain normal file-access checks and do not copy that workaround or code.
  [SetShot's FDA-gated domain list](https://github.com/adamengst/setshot-app/blob/0cea2d2b54a4d4d63d3acfd9e2ea3a21afcf6c24/SetShot/Services/DiffEngine.swift)
  independently includes the canonical pin domain. Neither source qualifies the
  owner's current Mac; a real denied read is reported with its observed status.

- **webMessages**, revision
  [`67ab65de818ac6bf6b3216b55fc451106976fdbc`](https://github.com/Pandoks/webMessages/tree/67ab65de818ac6bf6b3216b55fc451106976fdbc):
  its [Swift reader](https://github.com/Pandoks/webMessages/blob/67ab65de818ac6bf6b3216b55fc451106976fdbc/src/lib/server/pinned-chats.swift)
  loads IMCore and expects a local configuration dictionary containing an ordered
  identifier array. Its README advertises pin integration and requires SIP
  disabled for the overall application. That does **not** establish that a
  separate read-only exporter needs SIP disabled. There is a concrete compatibility
  concern: the reader expects an object returned by
  `fetchPinnedConversationIdentifiersFromLocalStore`, whereas both the iOS header
  above and BlueBubbles' older macOS header declare that method as returning
  `void`. This discrepancy requires runtime checking; the reader's empty-array
  fallback must not be treated as successful discovery of zero pins. No license
  was declared by GitHub's repository metadata or license endpoint, so its code
  was inspected as a reference and not copied into this project.
- **BlueBubbles helper**, revision
  [`1ee57f3fde63f8cc5769d60cfabfe9a06fcc1069`](https://github.com/BlueBubblesApp/bluebubbles-helper/tree/1ee57f3fde63f8cc5769d60cfabfe9a06fcc1069):
  [helper implementation](https://github.com/BlueBubblesApp/bluebubbles-helper/blob/1ee57f3fde63f8cc5769d60cfabfe9a06fcc1069/Messages/MacOS-11%2B/BlueBubblesHelper/BlueBubblesHelper.m)
  reads the pin set and changes it using IMCore. This runs inside the macOS
  Messages helper; it is not a Bluetooth capability. Their
  [private API documentation](https://github.com/BlueBubblesApp/bluebubbles-docs/blob/4bcb6e318412d150dca0fd35558a05b841c63503/private-api/imcore-documentation.md)
  warns of version-specific crashes in the old pinning example. Do not ship it
  unchanged on a newer Mac.
- **SMServer**, revision
  [`5332968a86993c1a0e524a3671da6533e3db0c61`](https://github.com/itsjunetime/smserver/tree/5332968a86993c1a0e524a3671da6533e3db0c61):
  the iOS sender's pin-reader attempt is commented out, notes entitlement/signing
  problems, and redirects investigation to its privileged helper. This is not
  evidence of a normal App Store iPhone app exposing pins.
- **RustPush**, current master
  [`b3c96c23e1aae78d1229ebaccb12027624f1eaff`](https://github.com/OpenBubbles/rustpush/tree/b3c96c23e1aae78d1229ebaccb12027624f1eaff):
  inspected `src/imessage/cloud_messages.rs`, `cloud_messages.proto`,
  `messages.rs` and `rawmessages.rs`; the CloudChat/CloudProp models cover chat
  and message metadata but contain no implemented pin model or dedicated
  ubiquitous-settings reader. Repository code searches also found no pin
  implementation. This is a scoped source finding, not proof that no fork has
  implemented it or that every unknown protocol field has been decoded.
- **BlueFerry**, installed revision
  [`60fd7ef1abbd25947981f0cf27a521d82784a75f`](https://github.com/erikwb/blueferry/tree/60fd7ef1abbd25947981f0cf27a521d82784a75f):
  `backend_operations.set_thread_starred()` writes the local encrypted settings
  store; it does not issue a pin command to the iPhone. Existing local stars
  therefore cannot be labelled as imported phone pins.

## Feature implementation path

1. Add native Pin/Unpin using the existing backend's encrypted local preference
   store, explicitly identified as pins on this computer until a real import
   exists. Preserve drafts, selections and new-message routing through reorder.
2. Qualify a small **read-only macOS exporter** against the owner's actual OS:
   first try reading the local pin configuration with SIP retained, distinguish
   unavailable/permission-denied from a verified empty pin set, and preserve
   order plus group identity metadata. The Mac must have the owner's Messages
   pins synchronized; Bubbles hardware activation alone is insufficient.
   Integrate the export into the existing macOS visit kit only after this probe
   works. A successful export is a snapshot, not continuing synchronization.
3. Import only exact individual-handle matches and verified group mappings.
   Ambiguous groups remain unresolved for explicit selection; never infer a
   group solely from its name or shared participant. Preserve local additions
   and source/time provenance. Store private identifiers through the encrypted
   backend rather than checking a personal export into Git.
4. For automatic iPhone changes while running Linux, investigate authenticated
   access to the Messages ubiquitous pin-settings store in the Bubbles/RustPush
   path. Scope the first experiment to read-only account data. No working Linux
   adapter, credentials flow, endpoint/schema or delivery timetable was
   established here. A continuously running Mac relay conflicts with the
   owner's stated requirement and is not the target architecture.

Acceptance: exact phone order, single-person and group pins, pin/unpin/reorder
changes, offline and stale-source states, account isolation and malformed import
handling. A snapshot must remain labelled with its export time. Bidirectional
sync requires separate conflict handling and real-device qualification before
the UI can claim phone synchronization.

This investigation did not change Bluetooth pairing, message transport, iCloud
sign-in, SIP, kernel, compositor, dock or Wingpanel. No public repository code
was vendored and no external message was sent.
