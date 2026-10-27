# Plugins

One core (T3 connection, window, palette, Wingpanel status feed), many tools. A plugin implements
`plugin::Plugin` (`src/plugin.rs`): an id, title, icon, a page widget built on first show, an optional
`refresh()` on each show, and optional status lines. The registry adds each page to the window stack as
`plugin:<id>`, an "Open <title>" palette entry, and `plugins.<id>` in `status.json` for the indicator.

Disable a built-in: `~/.config/claude-pantheon/plugins.json` → `{"disabled": ["accounts"]}`.

| Plugin | State | Source of truth |
|---|---|---|
| `accounts` | Done: list and switch saved Codex accounts | `codexbar-accounts.py` (shared with the CodexBar indicator) |
| `repobar` | Next: wrap `ReposPage` (src/repos.rs) as a plugin; the Wingpanel `repobar-wingpanel` stays | `repobar.py`, GitHub via `gh` |
| `usage` | Next: move `UsagePage` behind the trait | server usage RPC |
| `shiplog` | Blocked: only a spec-phase concept upstream (omarchy-shiplog, no implementation); define our own log format first | n/a |

Order for porting existing pages: repobar, usage, diagnostics, changes. Each move must keep the
PERFORMANCE.md idle-RSS number flat.
