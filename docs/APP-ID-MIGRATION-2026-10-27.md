# App id migration (2026-10-27)

Item 4 of [APP-CONSISTENCY-AUDIT-2026-10-27.md](APP-CONSISTENCY-AUDIT-2026-10-27.md). Every Rust app now
uses the `org.techmore.mseries.*` namespace, its own icon of the same name, and a `StartupWMClass` equal to
its application id (what the dock uses to group a window with its launcher). The `io.elementary.*` and
`org.elementary.*` prefixes are no longer claimed.

## What changed

| Old id | New id | Notes |
| --- | --- | --- |
| `io.github.techmore.ClaudePantheonRs` | `org.techmore.mseries.T3` | T3. D-Bus name, desktop actions, `gapplication` and the Wingpanel indicator follow. |
| `io.github.techmore.ClaudePantheonSdk` | `org.techmore.mseries.T3Sdk` | T3 Monitor (SDK). Now has its own icon (the T3 icon with an SDK badge). |
| `org.elementary.mseries.ImageStudio` | `org.techmore.mseries.ImageStudio` | |
| `io.elementary.mseries.Conversations` | `org.techmore.mseries.Messages` | "Messages & Signal". |
| `io.elementary.mseries.MessagesRust` | `org.techmore.mseries.MessagesPreview` | "Messages (Rust Preview)". |
| `io.github.sdolbec.SignalHistoryImport` | `org.techmore.mseries.SignalImport` | The importer used the Signal Pantheon client's icon. |
| `com.github.davidmhewitt.torrential` | `org.techmore.mseries.Torrential` | Torrential. The GSettings schema (`….torrential.settings` → `….Torrential.settings`), its path, the gresource prefix and the Flatpak manifest id follow, and a desktop entry and icon are new. **Saved Torrential preferences (download folder, window size) live under the old schema and are not carried over**: copy them with `dconf dump /com/github/davidmhewitt/torrential/settings/ | dconf load /org/techmore/mseries/Torrential/settings/` (the dump has a `[/]` header; edit it to match). |

Already in the namespace, unchanged: `AppleDev`, `DiskSpace`, `SystemMonitor`, `Updates`.

Icons are new for Disk Space, Software Updates, System Monitor, Messages (it pointed at
`io.elementary.mseries.Messages`, an icon that belongs to the Python Messages app and matched neither Rust
entry), Signal Import and T3 Monitor. Disk Space and Updates install theirs from `mseries/diskspace-app` and
`mseries/updates-app`.

Left alone on purpose, because they are data keys or other apps rather than the Rust apps' identity:

- `io.elementary.mseries.Messages` (the Python Messages app, its desktop entry, icon, keyring application)
  and `io.elementary.mseries.apple-message-pins` (the GSettings/keyring schema).
- The gettext domain `io.elementary.mseries.MessagesRust` (`po/`, the `.pot` and the installed `.mo`).
- `AI_APP_ID` in `mseries-messages`: the key reply-draft preferences are stored under.
- `io.github.sdolbec.SignalPantheonClient` (the Vala client) and `io.github.techmore.Dock` / `.Greeter`
  (session components).
- Both Messages entries and both T3 entries still ship. Retiring either is the owner's decision.

## What an existing install must do

Nothing is migrated automatically except the reply rules (below). Settings files live under
`~/.config/<crate name>/` (`claude-pantheon-rs`, `mseries-apple-dev`, `mseries/messages*.json` ...), not under the
app id, so **settings are unaffected**. What is keyed by the id:

1. **Dock pins.** Pins are desktop-file ids in `io.elementary.dock launchers`. After installing the new
   packages, swap the old ids for the new ones (this prints the new list; review it, then apply):

   ```bash
   python3 - <<'PY'
   import ast, subprocess
   renames = {
       "io.github.techmore.ClaudePantheonRs.desktop": "org.techmore.mseries.T3.desktop",
       "io.github.techmore.ClaudePantheonSdk.desktop": "org.techmore.mseries.T3Sdk.desktop",
       "org.elementary.mseries.ImageStudio.desktop": "org.techmore.mseries.ImageStudio.desktop",
       "io.elementary.mseries.Conversations.desktop": "org.techmore.mseries.Messages.desktop",
       "io.elementary.mseries.MessagesRust.desktop": "org.techmore.mseries.MessagesPreview.desktop",
       "io.github.sdolbec.SignalHistoryImport.desktop": "org.techmore.mseries.SignalImport.desktop",
   }
   current = ast.literal_eval(subprocess.check_output(["gsettings", "get", "io.elementary.dock", "launchers"], text=True))
   print([renames.get(item, item) for item in current])
   PY
   ```

   then `gsettings set io.elementary.dock launchers "<that list>"`.
2. **Old launchers.** The new packages do not remove the old desktop files and icons. Delete the stale
   `io.github.techmore.ClaudePantheon{Rs,Sdk}.desktop`, `org.elementary.mseries.ImageStudio.*`,
   `io.elementary.mseries.{Conversations,MessagesRust}.desktop`,
   `io.github.sdolbec.SignalHistoryImport.desktop` (and the old T3 `.service` D-Bus file and ImageStudio's) from
   `/usr/share/applications`, `~/.local/share/applications`, `.../dbus-1/services` and `.../icons`, then
   `update-desktop-database` and `gtk-update-icon-cache`.
3. **Running instances.** Quit the old app first: a new T3 will not reach an old instance over D-Bus
   (`gapplication action io.github.techmore.ClaudePantheonRs ...` becomes `org.techmore.mseries.T3`).
4. **Reply rules.** `mseries-reply` treats the old and new ids as the same app, so an existing
   `rules.json` that names an old id keeps working; the default rule list now names the new ids.
5. **MCP tool names.** The SDK-hosted T3 monitor's MCP tools are named from the last segment of its app id, so
   `claudepantheonsdk_send` / `_list_thread` / `_select_thread` are now `t3sdk_send` / `_list_thread` /
   `_select_thread`. Anything that calls them by name (agent prompts, saved MCP configs) needs the new names.
6. **Notification and badge identity.** The Messages dock badge follows the Preview desktop id
   (`application://org.techmore.mseries.MessagesPreview.desktop`), so it only shows on the new launcher.

## Not done here (other repositories)

The OS repo (`techmore/ElementaryOS_MSeries`) was out of scope. If it hard-codes any of the old ids (default dock
pins, packaging, polkit or notification allow lists, tests), those need the same rename. This repo's CI does not
cover that.
