# BlueFerry message-access upstream review — 2026-10-20

Scope: sending and receiving texts through BlueFerry on this desktop. This
review does not qualify message delivery or add another setup interface.

Conversation-pin import is a separate requested feature. The
[2026-10-20 pin-sync investigation](PIN-SYNC-RESEARCH.md) records Apple/private
API evidence, current public implementations and remaining Linux access gates.
Working text delivery and contact sync do not establish access to iPhone pins.

## Versions and primary sources

- BlueFerry upstream main and our installed backend/GTK source:
  [`60fd7ef1abbd25947981f0cf27a521d82784a75f`](https://github.com/erikwb/blueferry/tree/60fd7ef1abbd25947981f0cf27a521d82784a75f).
  GitHub main was checked directly. 152 installed Python modules match that
  checkout byte for byte; eight Qt-only modules are intentionally absent in
  this GTK installation. Package verification also reports four missing gzip
  documentation files; that does not indicate modified executable modules.
- BlueZ installed: bluetoothd and obexd **5.85-4ubuntu0.3**, also the newest
  candidate in this machine's configured Ubuntu repositories. Latest upstream
  release: [5.87](https://github.com/bluez/bluez/releases/tag/5.87), published
  2026-07-07. No package upgrade or alternate bluetoothd was installed by this audit.
- Tether latest release: [v0.2.40](https://github.com/zackb/tether/releases/tag/v0.2.40).
  Reviewed main [`9088422a31c99a8f43e7e4ec5ec4cb4a2883e3f9`](https://github.com/zackb/tether/tree/9088422a31c99a8f43e7e4ec5ec4cb4a2883e3f9).
  Its README requires BlueZ 5.86+ with experimental bearer API **before pairing**.
  Installed Tether 0.2.38 remains inactive; starting another MAP client would not
  remove the iPhone's authorization requirement and could compete for its one session.

## Practices to retain

| Area | Current upstream practice | Desktop result |
| --- | --- | --- |
| Pairing initiation | Client initiates Device1.Connect on the unpaired phone; iOS initiates authentication. Explicit Pair is a controller workaround. | Latest saved pairing report uses iphone-initiated-connect. No new re-pair during this review. |
| Profile order | First establish Classic/bond, then solicit ANCS; attempt MAP/PBAP before outbound LE. | Installed upstream implementation retained. |
| Local identity | A/V Hands-Free CoD; scoped post-bond advertisement. | Low device-class bits 0x0408 verified; LE advertising present. |
| Advertisement format | Full payload normally; compact fallback only after rejection. No globally discoverable pre-bond LE advertisement. | Forced compact and global-discovery experiments were unproven and removed. |
| OBEX services | Keep upstream obexd configuration and one long-lived MAP/PBAP session, with serialized operations. | Removed experimental mns/bluetooth-only server filter; normal obexd restored. |
| Permissions | A bond/contact sync is not MAP permission; Forbidden is not itself a reason to repeatedly re-pair. | Bond and 727 cached contacts retained; MAP still denied. |
| Optional notifications | BlueZ >=5.86 with functioning experimental bearer methods for modern ANCS flow. | BlueZ 5.85 has partial bearer interfaces without those methods. BlueFerry correctly selects compatibility/ANCS-disabled mode. Do not force full mode on this version. |
| Reset scope | Avoid unbounded radio resets and concurrent MAP clients. | No kernel, partition or global BlueZ replacement performed. |

Primary references:
[BlueFerry protocol](https://github.com/erikwb/blueferry/blob/60fd7ef1abbd25947981f0cf27a521d82784a75f/PROTOCOL.md),
[pairing guide](https://github.com/erikwb/blueferry/blob/60fd7ef1abbd25947981f0cf27a521d82784a75f/docs/en/user/pairing.md),
[troubleshooting](https://github.com/erikwb/blueferry/blob/60fd7ef1abbd25947981f0cf27a521d82784a75f/docs/en/user/troubleshooting.md),
[Tether requirements](https://github.com/zackb/tether/blob/9088422a31c99a8f43e7e4ec5ec4cb4a2883e3f9/README.md#bluetooth-for-messages-and-notifications).

## Corrections to the exploratory trials

BlueZ 5.85's
[`set_flags()`](https://github.com/bluez/bluez/blob/5.85/src/advertising.c#L785)
intentionally adds BR/EDR Not Supported when only the advertisement is discoverable
and the adapter itself is not. Seeing that flag is **not proof of a BlueFerry
bug or the cause of MAP denial**. Our temporary globally discoverable/legacy
broadcast made that flag disappear, but did not establish message access or
surface the phone switch. Changing discoverability while advertising also caused
HCI 0x2036 / EBUSY errors in this trial. Stopping the advertisement allowed the
original discovery settings to be restored. Do not promote this as a fix.

A custom HFP diagnostic completed BRSF, CIND test/read and CMER successfully;
MAP still returned Forbidden. WirePlumber is running again. The minimal OBEX
server-role experiment and Car Stereo selection likewise did not establish MAP.
These are failed/unfinished compatibility trials, not upstream recommendations.
All raw captures/identities remain private outside Git.

The transient BlueFerry ExecStart wrapper and OBEX plugin filter were removed.
Both services again run their packaged commands; original discovery settings and
phone-audio preference are restored. No contact data or bond was discarded.

## Open upstream work and limits

[BlueFerry #263](https://github.com/erikwb/blueferry/issues/263) is still open with
no comments or published fix at review time. It reports iOS 27.0.1 missing
permission switches; HFP restoration exposed switches for that reporter, but
SDP discovery still failed on BlueZ 5.87. Our phone **does** supply the MAP SDP
record and accepts the transport, then rejects OBEX CONNECT with 0xc3 (reported
by BlueZ as 0x43). Related symptoms do not establish the same cause.

[PR #256](https://github.com/erikwb/blueferry/pull/256) is open. It adds connection
backoff, manual reconnect and HFP recovery; its description explicitly separates
that scope from messaging. It is not an authorization fix and was not cherry-picked.

## Next useful protocol trial

Use a reversible, matched BlueZ 5.87 bluetoothd/obexd test build, enable and verify
the experimental bearer API before a new full-mode attempt, and capture the
actual pairing/LE-key/permission sequence. This removes the known 5.85 full-mode
feature gap. It is **not a proven MAP fix**: BlueFerry supports ordinary MAP in
compatibility mode, and issue #263 also reproduces a different failure on 5.87.
The current Ubuntu repositories do not provide that update, so it requires an
isolated build/package and rollback plan rather than claiming apt upgrade will
supply it. Preserve the current bond until an explicitly coordinated new pairing
is needed. Verification must include an authorized outgoing test and a real
incoming text; status checks, builds and contact sync do not count as delivery.

## Prepared candidate, not installed or running

Built the matched BlueZ 5.87 `src/bluetoothd` and `obexd/src/obexd` ARM64
executables from annotated release tag 5.87, commit
`65d11edee469c24dcc8d075b8be9c040cebaaf98`, in this chat workspace's
`work/bluez-5.87`. Bootstrap/configure and both links succeeded. Configure flags:
`--prefix=/usr/local --sysconfdir=/etc --localstatedir=/var
--libexecdir=/usr/local/libexec/mseries-bluez-5.87 --disable-client
--disable-manpages --disable-cups --disable-mesh --disable-midi`. Core phone profiles/OBEX are
built; normal /etc configuration and /var/lib/bluetooth storage paths are retained
in the compiled configuration. This service-only candidate is not feature-identical to Ubuntu's complete
package set. No make install was run and no new build dependencies were installed.

The daemon reports 5.87. Both ELF files are AArch64 with resolved shared-library
dependencies. Upstream unit tests: 34 GObex + 134 SDP passed (168 total). Build
and test logs remain in the chat workspace. These are software checks, not
phone authorization or delivery verification. The live host still runs Ubuntu's
5.85 bluetoothd/obexd. Runtime experimental bearer API and the live upgrade/rollback
trial remain unperformed. Current host kernel is
`7.1.12-2-20261017.20-sep-ARCH`; the audit changed no kernel.


## Live 5.87 trial and successful delivery — 2026-10-20

The owner authorized the live trial. Installed root-owned candidate binaries
under `/usr/local/libexec/mseries-bluez-5.87`, with distro files untouched and
root-private pre-trial bonds retained outside Git. Verified the **running**
bluetoothd path and both bearer Connect/Disconnect methods; merely reading the
candidate's version or a loaded ExecStart is insufficient. In the initial
activation a desktop D-Bus caller raced daemon-reload and reactivated 5.85;
an explicit restart corrected that before the actual 5.87 phone trial.

Existing bond: MAP remained Forbidden with matched 5.87 bluetoothd/obexd.
The owner then forgot eos on the iPhone. Removed only that phone's Linux bond,
set `BLUEFERRY_KEEP_PHONE_AUDIO_ON_PHONE=false` to expose Hands-Free during
pairing, and ran upstream's interactive Connect-first transaction. The owner
compared the passkey and accepted it on the phone; Linux confirmation followed.
Class 0x7c0408 and local Hands-Free/MNS/PBAP-client UUIDs were verified. This time
the phone acquired LinkKey type 8, IRK and LE long-term keys; previously only a
Classic key was present. Contact permission appeared first. The user subsequently
found and enabled **Show Message Notifications**; MAP and MNS then connected.
There is no requirement for a separate "sync messages" popup.

The fresh setup also exposed a **version-probe mismatch**: BlueFerry's
`bluetooth_capabilities.bluez_stack()` tries `bluetoothctl --version` before
daemon paths, so the distro 5.85 CLI selected compatibility mode despite the
verified 5.87 runtime. Built the matching 5.87 client (installed the small distro
`libreadline-dev` dependency), set a scoped PATH for backend/native launchers,
and reran completion on the existing new bond. It now reports 5.87,
notifications_supported/active and bearer_api_supported/active true. This fixes
policy detection; the evidence does not prove it alone made MAP permission
appear. Multiple conditions changed in the successful trial. No further
unpair was needed for this correction. Optional ANCS is still not authorized;
ordinary text delivery works independently.

Delivery evidence: one explicitly authorized self-addressed text produced
`sms_sent` then `sms_received` approximately 2.4 seconds later; a contact selected
by the owner's exact contact name received an authorized explanatory test, and
two incoming replies from that contact were recorded. No phone numbers, bond
keys, message contents or raw captures are published. A read-only recent-inbox
query returned 11 entries. Full missed-message/suspend recovery is not yet a
hardware pass. Apple documents MAP inbox browsing, composing and replying at
[Bluetooth profiles supported by iOS](https://support.apple.com/en-us/102842).
BlueFerry's full-history limitations still apply.

Additional current upstream checks:
- [Tether #243](https://github.com/zackb/tether/issues/243) was resolved by enabling
  L2CAP ERTM; this machine already has `disable_ertm=N`, and its failure was an
  actual OBEX Forbidden, so that fix was not applied here.
- [Tether #213](https://github.com/zackb/tether/issues/213) includes a corrected
  BlueZ build producing notification access; it is another reason to verify live
  APIs and build configuration, not proof of this phone's MAP cause.
- [BlueFerry #181](https://github.com/erikwb/blueferry/issues/181) records daily
  working iOS 27 use on Intel AX200/5.87. Broadcom reports #164/#146 do not provide
  a published universal authorization fix. Blue and bluez-message use the same
  MAP/OBEX authorization mechanism, so no alternate client was started in parallel.

`install-messaging-bluez.sh --install BUILD_DIR` now persists the successful
scoped binary/service setup without interrupting it; `--rollback` restores
packaged services and retains current bonds/contacts. It is applied to this
machine. Source/configure flags for the final client build match the earlier
candidate except `--enable-client` replaces `--disable-client`. `--experimental`
is a runtime daemon argument. Factory-image integration and post-reboot recovery
are not qualified.

## Upstream refresh — 2026-10-21

BlueFerry main is now `69122b3bd8579093e2a9c7b3d05ad66ff38ab50f`, 51 commits
ahead of our recorded `60fd7ef1` backend. This is a reviewed comparison, not a
backend pin change or deployment. The existing public upstream watcher records
this same new head. Changes include LE/media recovery, short-code conversations,
alphanumeric sender IDs, confirmed phone-link dialing and WirePlumber role-key
correction. BlueZ 5.85 still cannot qualify the newer bearer behavior.

The [alphanumeric-sender fix](https://github.com/erikwb/blueferry/commit/96d7d04223b9e8f81193b117fa5edbe7adaca15d)
marks keypad-spelled sender IDs read-only: replying to their digit representation
could address a different phone owner. Native Messages already respects backend
`reply_ready=false`. Its phone-snapshot adapter now also respects optional
`sender_alphanumeric` metadata when synthesizing or updating a direct thread,
including a duplicate message handle. Contact-name presentation preserves this
explicit sender identity. Direct read-only conversations explain that replies
are unavailable, rather than incorrectly asking to confirm group participants.

This compatibility patch does not make the old backend detect sender IDs, add
short-code support to old recipient validation, or qualify current upstream on
the phone. Upgrade the actual BlueFerry backend separately after preserving our
protocol and native adapter tests, reviewing all source changes, and authorizing
any live service/device activation.
