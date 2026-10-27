# mseries-sdk-apps

The M-series app SDK (`rust/msdk`, `rust/msdk-gtk`) and the Rust apps built on it, kept together so SDK and app changes land in one PR.

Extracted with history from [techmore/ElementaryOS_MSeries](https://github.com/techmore/ElementaryOS_MSeries) (`dev`). The OS image, kernel, compositor and Vala plugins stay there.

- Torrential (Relm4/Granite torrent client, own Cargo workspace so its dependencies stay out of the others): `rust/torrential` (imported with history from techmore/torrential, branch `mseries-native`)
- SDK and app-building docs: `mseries/SDK-*.md`, `mseries/APP-BUILDING-GUIDE.md`, `mseries/RUST-PANTHEON-SDK.md`
- Apps outside the Rust workspace: `mseries/polybagger-print-manager` (PolyBagger T1000 label printing; printer-link notes in `docs/polybagger-t1000-printer-link-troubleshooting.md`)
- Battery app and history logger: `mseries/battery` (docs `mseries/BATTERY-*.md`, `mseries/install-battery.sh`). The Wingpanel patches (`panel-power-battery-history.patch`, `panel-power-battery-eta.diff`) and `install-battery-eta.sh`'s helpers (`lib-backup.sh`, `lib-build.sh`, `patches/`) belong to the OS image repo, so that script only works from there.
- Image Studio (native Rust/Granite image generation UI): `rust/mseries-image-studio`, backend contract in `mseries/image-studio/README.md`
- Messages: Rust app `rust/mseries-messages`; Python frontend and phone setup `mseries/phone-setup`; MCP server `mseries/messages-mcp`; status/parity notes `mseries/MESSAGING-STATUS.md`, `mseries/MESSAGES-PARITY.md`; installers `mseries/install-phone-setup.sh`, `mseries/install-messaging-bluez.sh`
- Workspace: `rust/` (`cd rust && cargo test -p msdk`)
- SDK tests and bench: `mseries/sdk-tests`, `mseries/sdk-bench`

Layout is unchanged from the source repo on purpose; flattening paths is a later step. `msdk.yml` still references the Vala sources `mseries/codexbar-wingpanel` and `mseries/dock-taskbar`, which remain in the OS repo.

`rust/pantheon-compositor/protocol/pantheon-desktop-shell-v1.xml` is a vendored copy of the compositor's shell protocol, which `msdk-gtk` builds against. The OS repo is the source of truth; update this copy when the protocol changes.
