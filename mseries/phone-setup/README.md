# Connect iPhone (experimental)

A native GTK 4 / Granite 7 onboarding window over BlueFerry's existing backend.
Install with `bash mseries/install-phone-setup.sh` after installing BlueFerry's
backend and GTK client. Launch `mseries-phone-setup`; optionally pass
`--ios-version 27.0.1` for diagnostics. The opening page does not ask for an iOS version or require one to continue.

The intended flow is Next → choose a phone → Connect → confirm the matching code on the computer first, then tap Pair on the iPhone
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

The new window is an initial onboarding layer; message composition still opens
upstream BlueFerry. Tether and Bubbles are separate apps, not controlled by this
wizard. Localization infrastructure is not present yet.

## Current hardware result, 2026-10-20

M1 Pro J314S, elementary OS 9 / Aurora .20, BlueZ 5.85, iPhone 14 Pro / iOS
27.0.1. BlueFerry source `60fd7ef1abbd25947981f0cf27a521d82784a75f`,
package 0.8.1-1, built locally: 3734 tests passed, 16 skipped. Bluetooth powered
on, HFP device class and active LE solicitation confirmed. Bonding succeeds;
PBAP opens, but MAP returns OBEX Forbidden (0x43), and no message/contacts
switches appeared in the phone's Bluetooth detail page. One clean re-pair and
one HFP-role restoration/re-pair did not establish messaging. This is not a
qualified messaging installation. No test message was sent by this agent.

[BlueFerry issue #263](https://github.com/erikwb/blueferry/issues/263), opened
2026-10-18, reports the same iOS 27.0.1 missing-switch behavior. Restoring HFP
made its switches appear, but that reporter still had an SDP-service error on
BlueZ 5.87. That differs from our 0x43, so it is a related report, not a proven
root cause or guaranteed workaround. A BlueZ upgrade alone is not justified by
that evidence.

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
