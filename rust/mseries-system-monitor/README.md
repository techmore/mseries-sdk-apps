# System Monitor

Native GTK 4 and Granite app using the shared Rust Pantheon SDK. Overview
samples Linux host CPU and memory on demand; **Live updates** can pause or
resume polling, manual refresh works while polling is paused, and polling stops
when the Overview page is unmapped. The btop page embeds the installed `btop`
program through VTE, with a two-second btop update interval. btop is asked to
quit when its page is hidden, so it does not continue sampling in the
background. Install `btop` to use that page; CPU and memory cards work without
it.

The SDK sampler reads `/proc/stat` and `/proc/meminfo`; it does not run a
background service or retain process history. The custom Pantheon compositor's
frame/client metrics are not yet exposed through an app-facing API. The app
will use a typed, versioned compositor adapter once that OS contract is
available rather than parsing compositor journal text.

Build from the workspace:

```sh
cargo build --release -p mseries-system-monitor
```

The binary requires GTK 4, Granite 7, and VTE GTK 4 at runtime. `btop` is an
optional runtime dependency for process-level details.
