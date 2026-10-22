# Wingpanel indicator

The at-a-glance view of the T3 server on the panel: a badge with the number of pending permission
prompts (or a dot while a thread runs), and a popover with the plan windows (session, weekly, monthly
per provider as gauges), the rotation decision for the next unattended task, and the running threads.
Modelled on the CodexBar indicator in `mseries/codexbar-wingpanel`.

It reads `~/.cache/claude-pantheon-rs/status.json`, which the client writes whenever the shell or the
provider snapshot changes (`src/status.rs`, schemaVersion 1), and watches it with a `FileMonitor`: no
polling, no second connection to the server.

Build (needs `libwingpanel-dev`, `libgranite-7-dev`, `libjson-glib-dev`, `valac`):

```sh
meson setup build --prefix=/usr
ninja -C build
sudo ninja -C build install        # indicators load from the system dir
systemctl --user restart io.elementary.wingpanel  # or log out and in
```

Status: written against the Wingpanel 8 API (`Wingpanel.Indicator`, `get_display_widget`,
`get_widget`, `opened`/`closed`); not compiled on the development machine because
`libwingpanel-dev` is not installed there (candidate `8.0.4+r767` in the elementary daily PPA).
