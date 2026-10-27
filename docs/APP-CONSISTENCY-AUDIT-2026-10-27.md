# Rust Pantheon app audit: duplication and consistency (2026-10-27)

Scope: every Rust crate in `techmore/mseries-sdk-apps` at `f59f5fa`, plus the
copies still held in `techmore/ElementaryOS_MSeries` at `f4530c13`.
Method: static survey (crate sizes, dependencies, app ids, desktop files, CSS,
window sizes, shared-helper use, duplicated type names). Nothing was built or
run for this audit, so each finding below is a source observation.

## The apps

| Crate | Lines | Role | Uses `mseries-pantheon-kit` |
| --- | ---: | --- | --- |
| claude-pantheon-rs (T3) | 22,088 | GUI | yes |
| mseries-messages | 10,323 | GUI | yes |
| msdk | 9,858 | CLI/library | n/a |
| mseries-reply | 6,677 | library | no |
| mseries-image-studio | 6,344 | GUI | yes |
| msdk-gtk | 5,748 | GUI host | no |
| mseries-apple-dev | 4,812 | GUI | yes |
| mseries-diskspace | 3,135 | GUI | yes |
| mseries-pantheon-kit | 3,346 | shared kit | n/a |
| torrential | 3,045 | GUI | **no** (different toolkit stack, outside the workspace) |
| mseries-signal-import | 2,683 | GUI/CLI | no |
| mseries-ai | 2,534 | library | no |
| mseries-updates | 1,480 | GUI | yes |
| mseries-broker, -dock, -greeter, -power, -system-monitor, -update-core, granite-smoke | under 900 each | small | partly |

Non-Rust first-party apps that remain: Battery (Python), Signal Pantheon
(Vala), PolyBagger print manager, phone-setup, Orchard/Machine Lab (Vala).

## Duplication

1. **Whole crates exist twice.** The OS repo still holds `claude-pantheon-rs`
   (84 files), `mseries-diskspace`, `mseries-power`, `mseries-updates`,
   `mseries-update-core` and `granite-smoke`. The SDK copy is the source of
   truth, but the OS copies have drifted: `claude-pantheon-rs` differs in 66
   files, diskspace and updates in a few. A fix landed in one place is not in
   the other. Highest-value dedup.
2. **Updates backend defined twice.** `mseries-pantheon-kit::updates` and
   `mseries-updates/src/backend.rs` both define `CliBackend` and `Failure`;
   `mseries-updates` then re-implements the trait for the kit's type. One of
   them should own it. `mseries-update-core` is a third overlapping place
   (`Channel`, `Check`, `Phase`, `Notice`).
3. **Signal history key defined twice.** `HistoryKey` in
   `mseries-messages/src/signal.rs` (plain `[u8; 32]`) and in
   `mseries-signal-import/src/history.rs` (wraps a `Secret`). Both derive the
   same key from the keyring; they must stay byte-compatible, and only one
   zeroises. Move to one shared crate, keep the `Secret` form.
4. **Two Messages front ends ship.** `io.elementary.mseries.Conversations`
   (Messages & Signal) and `io.elementary.mseries.MessagesRust` (Rust
   Preview) share one icon and one crate, and the Vala Signal Pantheon client
   is a third path to the same data.
5. **Two T3 front ends ship.** `ClaudePantheonRs` ("T3") and
   `ClaudePantheonSdk` ("T3 Monitor (SDK)") reuse one icon; both application ids
   are declared inside the `claude-pantheon-rs` crate.
6. **Settings persistence hand-rolled per app.** `HOME`/`XDG_*` is read
   directly about 75 times; `claude-pantheon-rs` alone has three separate
   `config_path()` functions (plugin, workflow, sidebar). No shared
   "load/save JSON settings" helper exists in the kit.
7. **Shared names, divergent definitions** (`Settings`, `Config`, `Report`,
   `Progress`, `Request`, `State`, `Account`, `Transcript`): mostly harmless
   local types, but `Account`/`Config` in claude-pantheon-rs and
   signal-import, and `Transcript`/`Composer` in claude-pantheon-rs and
   mseries-reply, look like real overlap and should be checked.
8. **Threading.** 16 crates call `thread::spawn` directly; only 5 sites use
   the kit's `Polling`.

## Inconsistency

| Area | Observation |
| --- | --- |
| App id namespace | Four prefixes: `io.github.techmore.*` (T3), `org.techmore.mseries.*` (Apple Dev, Disk Space, System Monitor, Updates), `org.elementary.mseries.*` (Image Studio), `io.elementary.mseries.*` (Messages). Battery uses `org.mseries.*`; Signal Pantheon and signal-import use `io.github.sdolbec.*`. Claiming `org.elementary.*` / `io.elementary.*` is a real naming-rights risk. |
| Desktop entries | Disk Space, System Monitor and Updates use generic theme icons rather than their own; only some set `StartupWMClass`, which the dock uses to group windows. Messages entries point at an icon (`io.elementary.mseries.Messages`) that does not match either app id. |
| Colours | `claude-pantheon-rs` has 37 hard-coded hex colours and 7 rgba; Apple Dev 20 hex; Disk Space 19 rgba. Messages, System Monitor, Updates and Torrential have none. `msdk/data/tokens.json` exists but only T3 and msdk-gtk read it. |
| Dark mode | Only Apple Dev and Torrential react to the Granite colour-scheme preference; the other GUI apps rely on GTK defaults. |
| CSS install | Kit `install_css` is used by most apps, but T3, Apple Dev, Disk Space and Image Studio also build their own `CssProvider` (and msdk-gtk, dock and greeter do, which is expected). |
| Toolkit | Torrential pins `granite-rs 3.3.0` with `v7_5` + `relm4` and `gtk4 0.11`, against `granite-rs 3.3` with `v7_8, gtk_v4_20` everywhere else. It is the only relm4 app and is outside the workspace. |
| Window chrome | About dialogs: T3, Apple Dev, Image Studio only. Toasts: absent in Messages and System Monitor. Keyboard shortcuts: T3, Apple Dev, Image Studio only. Translations (`gettext`): Messages and Updates only. |
| Default window size | 1280×780 (Disk Space), 1100×760 / 1380×900 (Image Studio), 720×520 min (Apple Dev), 1180×760 (T3). No agreed size or minimum. |
| Accessibility | The kit's `set_accessible_label` is used 73 times, but only by the apps that use the kit (not Torrential, signal-import or msdk-gtk). |

## Proposed order of work

1. **Remove the OS-repo copies** of the six duplicated crates, after confirming
   nothing in the OS repo's packaging or CI builds them from `rust/`. Needs the
   frozen-paths label and the kernel-split pin check. No behaviour change.
2. **One updates backend.** Keep `CliBackend`/`Failure` in the kit (or in
   `mseries-update-core`), delete the other copy.
3. **One Signal history key crate**, shared by messages and signal-import.
4. **App ids and desktop entries.** Pick one namespace
   (`org.techmore.mseries.*` is already the majority), give each app its own
   icon and `StartupWMClass`, retire the Rust-preview entry once Messages is the
   only front end. Existing installs need a migration note because ids key
   dock pins and settings.
5. **Shared settings store** in the kit (`load_json`/`save_json` under
   `glib::user_config_dir`), then replace the hand-rolled `config_path()`s.
6. **Theme pass**: route hard-coded colours through `tokens.json`/Granite
   named colours, make every GUI app follow the Granite colour-scheme
   preference, agree a default and minimum window size.
7. **Torrential**: decide whether to align on `granite-rs 3.3` + `gtk4`
   without relm4 or leave it a documented exception.

Items 2, 3 and 5 are code changes in shared crates; 4 and 6 touch every app
and so should run as one change at a time, not in parallel.
