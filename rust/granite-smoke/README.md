# granite-smoke

Proves Rust can drive the system Granite 7 through `granite-rs` (bindings generated from `Granite-7.0.gir`).
Builds Granite widgets in an unmapped window and prints `PASS`. Needs a display, so run it under Xvfb:

```sh
CARGO_TARGET_DIR=/tmp/granite-smoke-target cargo build -p granite-smoke
dbus-run-session -- env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSETTINGS_BACKEND=memory \
  xvfb-run -a /tmp/granite-smoke-target/debug/granite-smoke
```

See `../../mseries/RUST-UPSTREAM-MAINTENANCE.md` for the gtk4 feature workaround and update steps.

Performance rules for GTK apps on this Mac: [`mseries/GTK-PERFORMANCE-ASAHI.md`](../../mseries/GTK-PERFORMANCE-ASAHI.md).
