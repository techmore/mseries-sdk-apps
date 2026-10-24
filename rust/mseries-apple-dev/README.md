# Apple Dev (`mseries-apple-dev`)

A native Pantheon window for [omarchy-apple-dev](https://github.com/joshuaswarren/omarchy-apple-dev),
the toolkit that builds, runs and ships iPhone apps (SwiftUI, UIKit, Flutter) and Mac apps from
Linux with no Mac and no Xcode install. Rust, GTK 4, Granite 7 and VTE, on `mseries-pantheon-kit`.

Apple Dev does not reimplement the toolkit. Every button runs the toolkit's own script in the
**Activity** terminal at the bottom of the window, so the installer's questions, sudo prompts and
`xtool auth` work exactly as in a terminal, and the full output stays visible.

## What it does

| Page | Shows | Actions |
|---|---|---|
| **Toolchain** | toolkit checkout and revision, Swift, the darwin iOS SDK, xtool, Apple ID sign-in, pymobiledevice3, usbmuxd, libimobiledevice (system or `~/.local/opt/libimobiledevice`), Flutter; on Ubuntu-family hosts the apt packages still missing and the swift.org Swift that matches the cached SDK's Xcode | Download / Update / Change toolkit, Install… (full with an Xcode `.xip`, or no-Xcode), Repair, Sign In (`xtool auth`), Flutter setup; Install packages (apt in Activity) / Copy the apt line; Install Swift |
| **Devices** | iPhones/iPads on USB (sysfs, works before any tool is installed), names and iOS versions from pymobiledevice3 or libimobiledevice; polled every 4 s while the window is open | Run target, Pair, Details, Live Log; tailnet address and Pair for Tailscale |
| **Project** | kind (xtool, Flutter, Mac SwiftPM, Xcode project), bundle id, last build output; after Upload to TestFlight, the newest build's App Store Connect processing and TestFlight state (re-checked every 30 s while Apple processes it) | Build, Run, Debug with LLDB, Attach, Device Build, App Store Build, Upload to TestFlight, Install over Tailscale; Flutter build/install/upload; Mac build/notarize; Convert Xcode project |

The header bar has **Build** (Ctrl+B), **Run** (Ctrl+R) and **Stop** (Ctrl+.) for the selected
project and a picker for the phone Run installs on (`device-run.sh --udid`). Add projects with
**+** (New iPhone App runs `xtool new`, Add Existing Folder, Import Xcode Project) or by dropping
folders on the window, or `mseries-apple-dev FOLDER…`. Ctrl+` toggles the Activity pane. A
finished command shows a toast, and a desktop notification plus a dock badge when the window is
in the background.

Settings live in `~/.config/mseries-apple-dev/settings.json`. Nothing secret is stored: for App
Store Connect only the `.p8` path, Key ID, Issuer ID and team id are kept, and passed to `ship.sh`
as `ASC_KEY_PATH`, `ASC_KEY_ID` and `ASC_ISSUER_ID` exactly as the toolkit README describes.

Every command (and the toolchain probe) gets the same environment first (`ENV_SETUP` in
`src/commands.rs`): `SWIFT_BACKTRACE=enable=no` (Swift's backtracer warns on every command on a
16K-page kernel), `TMPDIR=~/.cache/apple-dev-tmp` (/tmp is RAM on M-series laptops) and, when
`~/.local/opt/libimobiledevice` exists, its `bin`, `lib` and `lib/pkgconfig` on `PATH`,
`LD_LIBRARY_PATH` and `PKG_CONFIG_PATH`.

On elementary OS 9 / Ubuntu 26.04 the Toolchain page's **This Computer** section offers
`sudo apt install gnupg2 libcurl4-openssl-dev libheif-examples libimobiledevice-dev
libimobiledevice-utils libz3-dev llvm libplist-dev libusbmuxd-dev` (libimobiledevice 1.4.0 is new
enough; packages the user-prefix build covers are not asked for) and **Install Swift**: the
swift.org Ubuntu tarball for the Swift inside the SDK's Xcode (Xcode 27.1 ↔ Swift 6.4.1, 27.0 ↔
6.4.0), signature checked, unpacked into `~/.local/share/swift`, linked as `~/.local/bin/swift`,
followed by `install-toolchain.sh --repair` when an iOS SDK is already cached.

The TestFlight row asks App Store Connect read-only (`GET /v1/apps`, `GET /v1/builds?filter[app]=…`)
through the toolkit's own `tools/asc.py` `call()` with the venv's Python, so the key handling is the
toolkit's.

The toolkit is found at the configured path, else `~/.local/share/omarchy-apple-dev` (where
Omarchy's Install menu and **Download** put it), `~/omarchy-apple-dev` or
`~/Projects/omarchy-apple-dev`.

## Build, run, install

```bash
cd rust
cargo run -p mseries-apple-dev                      # debug build, run from the build output
cargo test -p mseries-apple-dev                     # model tests (no display needed)
bash mseries-apple-dev/install-local.sh            # release build into ~/.local, no sudo
bash mseries-apple-dev/build-deb.sh                # Debian package
bash mseries-apple-dev/snapshot.sh OUT_DIR         # light/dark PNGs of every page under Xvfb (fixtures)
```

`snapshot.sh` uses `fixtures/HelloOmarchy` (the shape `xtool new` writes), `fixtures/devices.json`
(one fake iPhone) and `fixtures/testflight.json` (an App Store Connect reply). Debug environment:
`MSERIES_APPLE_DEV_CONFIG`, `MSERIES_APPLE_DEV_FIXTURE_DEVICES`, `MSERIES_APPLE_DEV_FIXTURE_TESTFLIGHT`,
`MSERIES_APPLE_DEV_DARK`, `MSERIES_APPLE_DEV_SNAPSHOT[_DELAY|_ACTION]` (see `src/main.rs`).

## Layout

- `src/toolkit.rs`, `project.rs`, `devices.rs`, `commands.rs`, `settings.rs`, `distro.rs` (apt and
  swift.org), `testflight.rs` (App Store Connect build state): no GTK, unit tested.
  `commands.rs` is the only place that turns a button into a toolkit command line.
- `src/ui/`: `window.rs` (sidebar, pages, header), `console.rs` (VTE Activity pane), `dialogs.rs`,
  `widgets.rs`.

## Status

Used end to end on 2026-10-23: Rupert's iPhone app (Hotwire Native) was built on this aarch64
elementary OS 9 laptop in full mode (Xcode 27.1, Swift 6.4.1 from swift.org), signed and uploaded
to TestFlight, and runs on the owner's iPhone. That run exposed the environment, libimobiledevice,
apt/Swift and TestFlight-status gaps handled above. The live App Store Connect query was checked
against that build (1.0.0 (202610240241), VALID, IN_BETA_TESTING). Not run from the window yet:
Install packages and Install Swift (this machine already has Swift 6.4.1). xtool itself still had
to be installed by hand here (the pinned xtool 1.23.0 no longer builds libXADI.so; an upstream
installer bug).

## Screenshots

Fixture renders (`snapshot.sh`): [Toolchain](docs/toolchain-light.png),
[Devices](docs/devices-light.png), [Project](docs/project-light.png),
[Project, dark](docs/project-dark.png), [Activity after Build](docs/activity-light.png).
