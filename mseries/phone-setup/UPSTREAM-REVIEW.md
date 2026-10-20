# BlueFerry message-access upstream review — 2026-10-20

Scope: sending and receiving texts through BlueFerry on this desktop. This
review does not qualify message delivery or add another setup interface.

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
