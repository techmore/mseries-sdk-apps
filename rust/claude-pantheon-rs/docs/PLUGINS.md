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
| `activity`, `fleet`, `tstack`, `repository-overview` (the RepoBar GitHub monitor) | Done as adapters (`plugin_pages.rs`) | existing pages |
| `usage`, `changes`, `diagnostics` | Done as adapters (`plugin_pages.rs`): menu and palette entries, can be disabled | server RPCs |
| `shiplog` | Done (v1): merged PRs of the pinned repos grouped by day, via `gh pr list`; nothing stored. Later: release tags and thread outcomes as extra sources | `gh` |

`Plugin::open(app)` lets a built-in page keep its own navigation; plugins that return false get a stack page built from `build()`.
Disabling a plugin removes its menu item and palette entry; the code behind the page stays compiled in.

Each further move must keep the
PERFORMANCE.md idle-RSS number flat.
