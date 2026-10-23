# s03 Software Updates (medium)

Golden reference: `rust/mseries-updates` (UI) over `rust/mseries-update-core` (logic). Design: [../../UPDATER-DESIGN.md](../../UPDATER-DESIGN.md).

Front end for the `mseries-update` command. A channel switch (stable, dev), a "Check for updates"
button, a list of available packages (name, installed version, new version, size), an "Update all"
button with a progress bar and a live log drawer, and a footer showing the last check time.
All work is done by running `mseries-update` as a subprocess; the app never touches apt directly.

Capabilities: subprocess (`mseries-update`), polkit via the command, notifications.

Acceptance (a fake `mseries-update` script is supplied for tests):
1. Check shows a spinner, then the list, or "Up to date" with the last-check time.
2. Update all disables the buttons, streams the command output into the drawer, and advances the bar.
3. A non-zero exit shows an error row with the last log lines and a Retry button; nothing is lost.
4. Switching channel asks for confirmation and re-checks.
5. Closing the window during an update asks before quitting.
6. A notification fires when an update finishes while the window is unfocused.
