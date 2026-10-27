# SDK usage in the shared desktop status menu

The optional **Plan usage** page embeds the SDK's aggregate CodexBar snapshot
control in `TaskbarT3RPopover`, the shared source used by the Windows-style Dock
and the unified T3 Wingpanel indicator. Existing Usage, T3 server and GitHub
pages remain intact. The SDK page reads only while selected and the menu is
open; closing or changing pages cancels owned work. Account-specific actions
remain on the existing Usage page. Snapshot coverage is partial.

This is an opt-in development integration, not an installed or qualified
module. It needs a matching Linux architecture and GTK 4.20 or newer; do not
load it into a desktop with an older GTK. The C ABI and the Vala binding are
provisional and GTK-main-thread only. Never free a handle while its widget is
parented. The page wrapper unparents the control before destruction.

To prepare an isolated SDK prefix on a compatible Linux test system:

```sh
python3 mseries/sdk-tests/prepare.py /tmp/msdk-status-sdk
cargo build --manifest-path /tmp/msdk-status-sdk/Cargo.toml -p msdk-gtk --lib
mkdir -p /tmp/msdk-status-prefix/{include,lib}
cp /tmp/msdk-status-sdk/msdk-gtk/include/msdk-usage.h /tmp/msdk-status-prefix/include/
cp /tmp/msdk-status-sdk/target/debug/libmsdk_gtk.so /tmp/msdk-status-prefix/lib/
```

For an isolated Wingpanel build, configure `mseries/t3-wingpanel` with
`-Dmsdk_prefix=/tmp/msdk-status-prefix`. For an isolated pinned Dock source,
set `MSDK_USAGE_PREFIX=/tmp/msdk-status-prefix` when running
`mseries/dock-taskbar/prepare-source.sh`. These options embed an absolute
runtime library path; use a durable, architecture-matched prefix for any later
packaging. Default builds do not opt in. Neither command deploys, activates,
restarts, or changes the currently running panel.

The local integration fixture compiles the actual shared menu with synthetic
neighboring indicators and a private CodexBar fixture, testing both hosted and
dock modes. It is not a full Wingpanel/Dock build, a real-account test, a target
desktop interaction, a visual review, or owner acceptance.
