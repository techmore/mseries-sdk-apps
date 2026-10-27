# mseries-sdk-apps

The M-series app SDK (`rust/msdk`, `rust/msdk-gtk`) and the Rust apps built on it, kept together so SDK and app changes land in one PR.

Extracted with history from [techmore/ElementaryOS_MSeries](https://github.com/techmore/ElementaryOS_MSeries) (`dev`). The OS image, kernel, compositor and Vala plugins stay there.

- SDK and app-building docs: `mseries/SDK-*.md`, `mseries/APP-BUILDING-GUIDE.md`, `mseries/RUST-PANTHEON-SDK.md`
- Workspace: `rust/` (`cd rust && cargo test -p msdk`)
- SDK tests and bench: `mseries/sdk-tests`, `mseries/sdk-bench`

Layout is unchanged from the source repo on purpose; flattening paths is a later step. `msdk.yml` still references the Vala sources `mseries/codexbar-wingpanel` and `mseries/dock-taskbar`, which remain in the OS repo.
