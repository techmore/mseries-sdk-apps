# App conventions

The standards every first-party Rust Pantheon GUI app follows, and that the SDK's catalog and generators are
meant to emit by default. They are machine-checked: [`rust/mseries-pantheon-kit/data/conventions.json`](../rust/mseries-pantheon-kit/data/conventions.json)
lists the rules and the apps, and `cargo test -p mseries-pantheon-kit --test conventions` fails when an app breaks one.
A generator can read the same JSON for the rule text and the app list.

| Rule | Standard | How apps meet it |
| --- | --- | --- |
| `id` | App id `org.techmore.mseries.<Name>`; desktop file `<id>.desktop`; `Icon=<id>`; `StartupWMClass=<id>`; `<id>.svg` ships; the same id is the registered application id. | See [APP-ID-MIGRATION-2026-10-27.md](APP-ID-MIGRATION-2026-10-27.md). A transitional entry of the same app may borrow a sibling's icon (`icon_alias`). |
| `colors` | No hex colour literals in an app's Rust or CSS. Use theme and Granite named colours (`@accent_color`, `@error_color`, `@warning_color`, `@theme_fg_color`...) or tokens from `rust/msdk/data/tokens.json` (`colors.pantheon`, `colors.accents`). | The kit CSS defines the palette once; `appearance.rs` tests it against the tokens. |
| `color_scheme` | Follow the Granite colour scheme. | `mseries_pantheon_kit::appearance::follow_color_scheme(None)` after `granite::init()`. |
| `window_size` | Workspace windows: default 1100×760, minimum 720×520. Utility windows: default 560×680, minimum 360×480. | `appearance::size_window` with `WORKSPACE_*` / `UTILITY_*`. |
| `accessible_labels` | Every icon-only control has an accessible label. | `mseries_pantheon_kit::set_accessible_label`. |
| `about` | An About dialog, reachable from the header's menu. | `chrome::install_app_actions` (registers `app.about`, `app.quit`) and `chrome::primary_menu`. |
| `shortcuts` | Keyboard shortcuts for the main actions, including Ctrl+Q and Ctrl+? where it applies. | `chrome::install_app_actions` (Ctrl+Q), plus app-specific `set_accels_for_action` / key controllers. |
| `toasts` | Results and errors are reported with toasts. | `chrome::toast_overlay` and `chrome::show_toast`. |
| `gettext` | User-visible strings go through an app-owned gettext domain. | A `strings.rs` per app (`DOMAIN` = app id, `tr()` over `mseries_pantheon_kit::i18n`). **Coverage is partial:** the domain is bound and the window chrome (About, menus, toasts) is translated; converting every remaining literal in T3, Image Studio, Apple Dev, Disk Space and System Monitor is still owed. Messages and Updates route their strings already. |
| (config) | Per-app settings are JSON under the user config dir. | `mseries_pantheon_kit::settings`. |

## Exceptions

An app that cannot meet a rule has an entry under `exempt` in `conventions.json` that names the rule and says why.
Two kinds exist: a **gap** (the reason starts "gap:"; the work is owed) and a **deliberate exception** (a terminal palette, a
data colour, an app that already follows the scheme another way). The test also fails when an exempt rule starts
passing, so gaps are removed from the file as they are fixed and the file only ever lists what is still open.

## Adding an app

1. Add it to `apps` in `conventions.json` (name, crate dir, desktop files) with no exemptions.
2. Make the test pass. Start from an existing app of the same shape (workspace or utility window).
3. Describe any exception in `exempt`, with the reason; a reviewer decides whether it is acceptable.

## Not covered by the check yet

Layout adaptivity (compact breakpoints) and the per-app `Settings` file formats. Torrential is on the list; it keeps
Fluent catalogs instead of gettext (a documented exception) and its relm4 stack otherwise.
