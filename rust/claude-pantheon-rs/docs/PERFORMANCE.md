# Performance

Measured 2026-10-21 on elementary OS (Pantheon, Wayland) on an Apple Silicon Mac: 10 cores, 30 GB RAM,
aarch64. The client is a release build of this repository (`cargo build --release`, cairo renderer);
the T3 server is the user service `t3code.service` (node + resource-monitor sidecar + the Claude Code
server-child it spawns) with four providers authenticated; the Electron app is
`/usr/lib/claude-desktop/claude-desktop` idle with its Code tab open. All rows come from
`./bench.sh <pattern> 20`, which sums RSS over the matched process tree and averages CPU from
`/proc/<pid>/stat` deltas over 20 s. The client rows were taken under Xvfb (no screenshot or display
tool on this host) after a 30 s settle, with `CLAUDE_PANTHEON_OPEN` selecting the chat.

## Idle, with a chat open

| Process tree | Processes | RSS avg (MB) | RSS peak (MB) | CPU % over 20 s |
|---|---|---|---|---|
| claude-pantheon-rs, connected, Claude thread (87 items) open | 1 | 60 | 60 | 0.0 |
| claude-pantheon-rs, offline, 13.5 MB JSONL transcript open | 1 | 58 | 58 | 0.0 |
| T3 server (`t3code.service`), idle, four providers | 4 | 237 | 237 | 1.0 |
| client + server | 5 | ~300 | – | ~1 |
| claude-desktop (Electron), idle, this run | 81 | 1661 | 1723 | 112.2 |
| claude-desktop (Electron), `BASELINE.md` (19 processes sampled then) | 19 | 2330 – 2390 | – | 108 |

The client alone is 1/28 of the Electron app's memory and uses no CPU at idle; with the server it is
about 1/6 of the memory and 1 % of a core (the server's provider health probes), for every provider
instead of one. The first bench run, taken 12 s after launch while the thread was still loading,
showed 4.4 % CPU and 56 MB; the numbers above are after the window settled.

Start-up has no persistent timers: the only recurring work while idle is one WebSocket ping every
30 s and one `server.getConfig` call every five minutes for the plan windows.

## Start-up

`claude-pantheon-rs --quit-after-frame` prints the time from `main()` to the first frame
(`add_tick_callback`), five launches in a row on the real Wayland display, warm page cache:
**186, 184, 213, 188, 188 ms**; wall time from `exec` to exit (`/usr/bin/time`) 0.22 – 0.25 s. The
offline index (141 sessions, cache warm) takes 1 ms; cold with the cache missing it scans 650 MB of
transcripts with rayon in 53 ms. The server connection and the thread list arrive after the first frame.

## Transcript parsing (gio thread pool)

The 13.5 MB "Messages app UI/UX enhancement" transcript opens in well under a second: lines are read
one at a time (`BufReader`, 256 KiB), only `user`/`assistant` records go through serde, tool results
are joined onto their calls, and only the last 200 turns become widgets (a button reveals the rest).
Tool-row details are built on first expand.

## Targets

| Target | Result |
|---|---|
| Under 120 MB RSS | 58 – 60 MB |
| ~0 % CPU at idle with a transcript open | 0.0 % |
| Start-up under 300 ms | ~190 ms to the first frame |

## What it took

The three findings from the Vala twin (`claude-pantheon/docs/PERFORMANCE.md`) apply here too and are
built in:

1. **Renderer.** Mesa on Asahi loads LLVM and ~180 MB of shader heap for the GL renderers. `main()`
   sets `GSK_RENDERER=cairo` unless the environment already names a renderer;
   `CLAUDE_PANTHEON_RENDERER=gl|ngl|vulkan` switches back.
2. **Window shadow.** The default client-side decoration is a blurred `box-shadow` that cairo
   re-blurs over the whole window on every frame; `data/style.css` replaces it with a 1 px outline.
3. **Cursor blink.** GTK fades the text cursor with a frame-clock tick for ten seconds after each
   keystroke; `gtk-cursor-blink` is off, so a focused composer costs nothing.

Everything else: parsing off the main thread, widgets only for the visible tail, the whole T3
protocol on the GLib loop (no tokio, no worker threads: the WebSocket and HTTP clients sit on
`gio::SocketClient`), and the sidebar rebuilt once per burst of shell events.

## Per-PR idle numbers

Every PR re-runs `bench.sh` with a Claude thread open (connected, 30 s settle, 20 s sample) and the
start-up probe; the table tracks drift per feature.

| PR | Change | RSS avg MB | CPU % | Start-up ms (first frame) |
|---|---|---|---|---|
| #462 | initial import | 60 | 0.0 | 184 – 213 |
| #463 | install target, .desktop actions, D-Bus activation, metainfo, icons | 59 | 0.0 | 187 – 205 |

## Reproduce

```sh
./bench.sh '^/home/me/.cache/cargo-target/release/claude-pantheon-rs' 20 "Rust client"
./bench.sh '^/home/me/.local/share/t3code/home/runtime' 20 "T3 server"
./bench.sh '^/usr/lib/claude-desktop/claude-desktop' 20 "Electron"
for i in 1 2 3; do claude-pantheon-rs --quit-after-frame; done
```
