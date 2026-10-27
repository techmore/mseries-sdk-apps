# Plugins

One core (T3 connection, window, palette, Wingpanel status feed), many tools. A plugin implements
`plugin::Plugin` (`src/plugin.rs`): an id, title, icon, a page widget built on first show, an optional
`refresh()` on each show, and optional status lines. The registry adds each page to the window stack as
`plugin:<id>`, an "Open <title>" palette entry, and `plugins.<id>` in `status.json` for the indicator.

Disable a built-in: `~/.config/claude-pantheon/plugins.json` → `{"disabled": ["accounts"]}`.

| Plugin | State | Source of truth |
|---|---|---|
| `accounts` | Done: list and switch saved Codex accounts | `codexbar-accounts.py` (shared with the CodexBar indicator) |
| `repos` | Done as an adapter: `plugin_repos.rs` registers `ReposPage` (src/repos.rs stays; the runner, webhooks and thread-to-issue map reach into it) so it has a menu/palette entry, can be disabled, and reports open issues. Wingpanel `repobar-wingpanel` stays separate | `repobar.py`, GitHub via `gh` |
| `usage` | Next: move `UsagePage` behind the trait | server usage RPC |
| `shiplog` | Blocked: only a spec-phase concept upstream (omarchy-shiplog, no implementation); define our own log format first | n/a |

`Plugin::open(app)` lets a built-in page keep its own navigation; plugins that return false get a stack page built from `build()`.
Disabling a plugin removes its menu item and palette entry; the code behind the page stays compiled in.

Known: `tests/render.rs` times out waiting for the repos page in this environment (also on the commit before the plugins).

Order for porting existing pages: repobar, usage, diagnostics, changes. Each move must keep the
PERFORMANCE.md idle-RSS number flat.
