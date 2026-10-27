# Recovery notes (2026-10-27)

This tree was rebuilt by replaying every file edit recorded in the two Codex
threads that created Signal Pantheon (`01a15900…` prototype, `01a159e3…` client),
in time order, inside a read-only/no-network bubblewrap sandbox. The original
working copy (`~/Documents/Codex/2026-10-20/com/outputs/signal-pantheon`) was
deleted and never pushed.

## What this is
- State of the client as of **2026-10-20 17:46 UTC** (last recorded edit).
- Builds with meson/valac; 8 of 9 test suites pass (the UI suite needs a display).
- `src/qrcodegen.c/.h` is the MIT-licensed Nayuki QR generator, fetched from
  github.com/nayuki/QR-Code-generator `c/` (the original did the same).

## What this is NOT
The installed binary (`~/.local/bin/signal-pantheon-client`, built
2026-10-21 12:06) is **newer** than this source. About 62 symbols exist only in
the installed binary: avatars/initials (`appearance_avatar`, `main_window_avatar_*`),
inline images and thumbnails (`load_inline_image*`, `load_thumbnail*`,
`is_viewable_image`), a media window (`build_media_window`), `heading_time`,
`empty_state`, `is_active_conversation`, and more. Those edits (Oct 20 17:46 →
Oct 21 12:06) are not in any Codex or Claude session log, so they cannot be
replayed. They must be re-implemented, guided by `nm -C` / `strings` of the
installed binary (a copy is kept in the recovery folder).

## Not recoverable by replay
- `vendor-src/*.patch` (signal-cli backend, call tunnel, ringrtc): the patches
  were generated from upstream trees that no longer exist, so they replayed empty.
  The compiled results still live in `~/.local/share/signal-pantheon/runtime`
  (signal-cli-arm64, `signal-call-tunnel`, Temurin 25) and are not touched.
- `runtime/`, screenshots, evidence beyond two small JSON/TXT files.
