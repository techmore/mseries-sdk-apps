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
| **Toolchain** | toolkit checkout and revision, Swift, the darwin iOS SDK, xtool, Apple ID sign-in, pymobiledevice3, usbmuxd, Flutter; a note on non-Arch hosts | Download / Update / Change toolkit, Install… (full with an Xcode `.xip`, or no-Xcode), Repair, Sign In (`xtool auth`), Flutter setup |
| **Devices** | iPhones/iPads on USB (sysfs, works before any tool is installed), names and iOS versions from pymobiledevice3 or libimobiledevice; polled every 4 s while the window is open | Run target, Pair, Details, Live Log; tailnet address and Pair for Tailscale |
| **Project** | kind (xtool, Flutter, Mac SwiftPM, Xcode project), bundle id, last build output | Build, Run, Debug with LLDB, Attach, Device Build, App Store Build, Upload to TestFlight, Install over Tailscale; Flutter build/install/upload; Mac build/notarize; Convert Xcode project |

The header bar has **Build** (Ctrl+B), **Run** (Ctrl+R) and **Stop** (Ctrl+.) for the selected
project and a picker for the phone Run installs on (`device-run.sh --udid`). Add projects with
**+** (New iPhone App runs `xtool new`, Add Existing Folder, Import Xcode Project) or by dropping
folders on the window, or `mseries-apple-dev FOLDER…`. Ctrl+` toggles the Activity pane. A
finished command shows a toast, and a desktop notification plus a dock badge when the window is
in the background.

Settings live in `~/.config/mseries-apple-dev/settings.json`. Nothing secret is stored: for App
Store Connect only the `.p8` path, Key ID, Issuer ID and team id are kept, and passed to `ship.sh`
as `ASC_KEY_PATH`, `ASC_KEY_ID` and `ASC_ISSUER_ID` exactly as the toolkit README describes.

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

`snapshot.sh` uses `fixtures/HelloOmarchy` (the shape `xtool new` writes) and `fixtures/devices.json`
(one fake iPhone). Debug environment: `MSERIES_APPLE_DEV_CONFIG`, `MSERIES_APPLE_DEV_FIXTURE_DEVICES`,
`MSERIES_APPLE_DEV_DARK`, `MSERIES_APPLE_DEV_SNAPSHOT[_DELAY|_ACTION]` (see `src/main.rs`).

## Layout

- `src/toolkit.rs`, `project.rs`, `devices.rs`, `commands.rs`, `settings.rs`: no GTK, unit tested.
  `commands.rs` is the only place that turns a button into a toolkit command line.
- `src/ui/`: `window.rs` (sidebar, pages, header), `console.rs` (VTE Activity pane), `dialogs.rs`,
  `widgets.rs`.

## Status

Implemented, built, unit tested, rendered under Xvfb in light and dark, installed for the owner
and launched on the desktop. Not yet verified on this machine: the real install, build and device
flows, because Swift, xtool and an iPhone are not set up here. The installer is written for Arch;
on elementary OS it skips the pacman steps (the Toolchain page says so and links
`docs/DISTROS.md`). The Arch rebase (`mseries/ARCH-REBASE-PLAN.md`) makes it a first-class host.

## Screenshots

Fixture renders (`snapshot.sh`): [Toolchain](docs/toolchain-light.png),
[Devices](docs/devices-light.png), [Project](docs/project-light.png),
[Project, dark](docs/project-dark.png), [Activity after Build](docs/activity-light.png).
