# SDK checks on the elementary OS arm64 target (2026-10-27)

Source: mseries-sdk-apps main de5ed3d. Host: the owner's Apple Silicon laptop (elementary OS, aarch64, 10 cores, Linux 7.2.11-elementary-aurora72-wip4), private Xvfb + D-Bus sessions with cairo rendering. **Not** the physical Wayland compositor and not owner acceptance.

## Core + native tests

`cargo fmt --check`, `cargo clippy -p msdk -p msdk-gtk --all-targets -- -D warnings`: pass.

`cargo test -p msdk -p msdk-gtk` outside a private bus: all pass except `msdk-gtk --test cli`, which by design requires `MSDK_PRIVATE_TEST_BUS=1`.

Native host tests in the CI recipe (`MSDK_PRIVATE_TEST_BUS=1 ... xvfb-run -a dbus-run-session -- cargo test -p msdk-gtk --test lifecycle --test runtime --test cli --test usage --test codexbar -- --test-threads=1`): 9 passed, 0 failed.

## Idle footprint (20 s, release builds, empty HOME, offline)

| App | processes | RSS avg MB | RSS peak MB | idle CPU % | note |
|---|---:|---:|---:|---:|---|
| native T3 client (claude-pantheon-rs) | 1 | 44 | 44 | 0.0 | 20 s on 10 cores |
| SDK T3 client (claude-pantheon-sdk) | 1 | 35 | 35 | 0.0 | 20 s on 10 cores |
| SDK Home Storage (msdk-run) | 1 | 34 | 34 | 0.0 | 20 s on 10 cores |

Reproduce: `bash mseries/sdk-tests/perf-target.sh 20`. Processes counts only the app itself (Xvfb and the session bus are excluded). These are empty/unpaired states, not a loaded thread; the native client reaches ~75 MB with content (rust/claude-pantheon-rs/docs/PERFORMANCE.md). Start-up time was not measured here.
