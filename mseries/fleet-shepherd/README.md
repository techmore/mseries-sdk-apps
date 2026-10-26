# Native T3 Remote Machines

T3's Remote Machines page and unified menu's Fleet action manage an explicit
list of SSH aliases and inspect Herdr agent state and OMP usage. Machine Lab
remains the local VM/container product; it has replaced Orchard. This feature
neither reinstalls Orchard nor adds a Quattro shell.

## Primary flow

1. Configure SSH aliases and verified host keys in OpenSSH as usual. Participating
   machines need Herdr and/or OMP for the corresponding status source. Missing
   tools or failed access remain unavailable; no installation/authentication is
   attempted remotely.
2. Open T3 → Remote Machines, or choose Fleet in the unified dock menu. Opening
   reads connector settings only; it makes no SSH connection.
3. Enter a label and existing SSH alias, then Add Machine. Remove deletes only
   that local Fleet entry. Both actions preserve OpenSSH settings and keys.
4. Choose Refresh Status to inspect configured machines. Each card identifies
   the connector, observation time, agent state and OMP history. Partial/offline,
   stale and unavailable data are explicit; missing measurements are never zero.
   OMP history is machine telemetry, not T3 selected-account quota. Its date range
   is unavailable in the current upstream response.
5. Open SSH Terminal launches the user's native default terminal with an explicit
   alias and strict host-key checking. This interactive session can be used for
   manual machine management. Automatic remote start/stop, deployment or power
   controls are not supplied by the read-only Fleet API.

Configuration belongs to this computer's `~/.config/fleet-shepherd/connectors.json`
(or `XDG_CONFIG_HOME`), independently of the paired T3 server. The existing local
connector is retained. Writes are validated, atomic, mode 0600, serialized and
refuse symlink destinations. Opening/editing connectors never probes them.
Uninstall removes owned helpers; user connector configuration is retained.

## Source and adaptation

Public source: [joshuaswarren/omarchy-fleet-shepherd](https://github.com/joshuaswarren/omarchy-fleet-shepherd)
revision `b39cf962dcbbae397c1d21f4267c24ab49e5ddbc`, MIT, Joshua Warren 2026.
Full license retained here and installed with T3. Source hashes are in
`source.json`. The native GTK page is original project code.

`bin/fleet-snapshot` retains the upstream fixed-command, strict-host-key,
byte/item/concurrency/deadline and stream-isolation boundaries. The native path
uses no snapshot cache or background polling. Downstream additions register
owned probe processes and handle SIGTERM/SIGINT by killing their process groups;
this prevents a cancelled/hidden page leaving SSH/OMP children behind. Native
`bin/mseries-fleet` adds list/add/remove adapters with a private atomic config
writer and dispatches explicit snapshot requests to the installed collector.
It always uses the collector shipped with T3, not a similarly named PATH tool.

The Rust adapter bounds output to 4 MiB and 70 seconds, invalidates cancelled
requests, rejects incompatible schemas and keeps source identity visible.
Only read requests are cancellable; local config writes finish atomically even
if navigation changes. Optional terminal tooling is checked before enabling the
terminal action. No raw SSH stderr, tokens, transcript scraping or Hyprland
window-focus helper is imported.

## Evidence and limits

- 65 retained upstream collector/config/stream tests; unrelated Hyprland focus
  and QML/documentation test classes were excluded because those surfaces are
  deliberately absent. Four native adapter tests cover local settings, hostile
  aliases, symlink preservation, concurrent changes and owned-process teardown.
- 81 T3 library tests and strict clippy pass. Native light/dark compact/regular
  fixture screenshots retain offline, partial, stale and missing-source states.
- The real settings/add/remove/snapshot adapter is exercised from the GTK page
  with private HOME, non-activating D-Bus and inert SSH/Herdr/OMP/terminal commands.
  Exact SSH options and terminal argv are checked. No real machine is contacted.
- The unified menu's production-widget probe checks Activity, Fleet and Open T3
  desktop routes, hover/focus/dismissal and closed usage polling in both themes.
- Release and app-only Debian package builds include the helper, collector and
  license. None of this establishes live remote compatibility or owner acceptance.

Retained synthetic screenshots and test logs: `receipts/2026-10-26/`. The render
harness emits a non-fatal GtkText focus-out warning while switching between its
fixture windows; physical keyboard/focus and real-host acceptance remain open.
The broader chat qualification remains deferred by the owner.

## Installed state

Installed the final release and per-user helper/collector, then reloaded only
the T3 front end and dock. `fleet` is exposed in the loaded window action list
and opening it reads connector settings only. Installed adapter `list` succeeds
with the one existing local connector. `receipts/2026-10-26/loaded.json` records
matching source-built/installed/loaded hashes and process identities. Wingpanel
and the T3 server remained running. No real SSH probe was requested. Target
aliases and physical-host acceptance remain pending owner configuration.
