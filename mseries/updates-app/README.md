# Software Updates (Rust + Granite 7)

A native front end for [`mseries-update`](../UPDATER-DESIGN.md). It shows `mseries-update status --json`
(schema 1) and offers **Check for Updates**, **Install at Next Restart**, **Roll Back…** and, under
**Advanced**, the update channel. It contains no update logic: the updater decides what is available,
staged, applied or rolled back, and the app turns a click into one fixed operation.

```
rust/mseries-update-core   status model (serde), the closed command set, and the root helper binary (no GUI deps)
rust/mseries-updates       the GTK 4 + Granite 7 app (granite-rs); backend, window, tests
mseries/updates-app/       install.sh (dev install), test.sh (everything CI runs), this file
```

## How a click becomes a root action

1. The window calls `Backend::run(Command)`. `Command` is `Check`, `Stage`, `Rollback` or `SwitchChannel(stable|dev)`;
   there is nothing else to call (no `init`, `config`, `apply --now`).
2. The backend runs `pkexec /usr/libexec/mseries-updates/mseries-update-helper <op>`. polkit action
   `org.techmore.mseries.update` authorises exactly that program (its `exec.path`), with
   `auth_admin_keep` for the active session: an administrator password in the graphical prompt, no terminal and no `sudo`.
3. The helper (a binary, because pkexec matches scripts by interpreter) re-parses the arguments against the same
   allowlist, refuses anything else with exit 2, then replaces itself with `/usr/sbin/mseries-update <op>` under a cleaned environment.
   Exit codes reach the app unchanged: 75 busy, 3 rollback failed, pkexec's 126 (prompt dismissed, shown as nothing) and 127.
4. Reading status needs no privilege and runs on a worker thread; the window re-reads it after every operation.

## What the window shows

| Updater state | Headline | Actions |
| --- | --- | --- |
| never checked | Updates haven't been checked yet | Check for Updates |
| checked, nothing new | The M-series software is up to date, last checked N minutes ago | Check for Updates, Roll Back… if an update can be undone |
| updates offered (entries already installed or rolled back are not listed) | N updates available, old → new versions | Install at Next Restart (= `stage`), Check |
| `staged` | Ready to install at your next restart | Check Again |
| `awaiting-confirmation` | Updates installed (not yet marked working) | Roll Back…; channel switch off |
| `recovering` | Restoring the previous versions | Check off; wait for the next restart |
| rolled back (by the updater or by the user), rollback failed | an icon + sentence under the headline | as above |
| `mseries-update` missing / unreadable status / newer schema | placeholder with the reason and Try Again | Try Again |

Roll Back and the channel switch ask first, with what will happen in plain words. A dismissed password prompt is silent;
a refusal or failure is shown as a toast and as text under the headline, and the user can try again. While a task runs the
controls are off and the headline area says what is happening.

Follows the [UI specialist skill](../../.agents/skills/elementaryos-mseries-ui-specialist/SKILL.md): Granite header labels,
placeholder and toast; boxed list rows; standard suggested-action button; status carried by icons and words, not colour alone;
accessible names on the list and the channel selector; labels wrap and the page scrolls.

## Install, test

```bash
bash mseries/updates-app/install.sh      # builds, installs under /usr/local, asks for sudo; --uninstall removes it
bash mseries/updates-app/test.sh         # unit, helper, contract, Xvfb render + interaction, staged install
MSERIES_UPDATES_SHOTS=/tmp/shots bash mseries/updates-app/test.sh   # also keep light/dark screenshots of every state
```

`.github/workflows/mseries-updates-app.yml` runs `test.sh` on every change and then installs the policy for real and
checks it with `pkaction`.

## Not done / not verified

- **Not run on the M1 desktop session or against a real update repository.** The tests use a fake updater (every state, every
  button, every failure), the real `mseries-update.py` reading scratch state (parser contract), and Xvfb. A real password prompt
  through the Pantheon polkit agent, `pkexec` end to end into `mseries-update`, and the staged update actually installing at a
  restart are untested here. Nothing was installed on this machine and no Wingpanel, dock or compositor restart was done.
- **No Switchboard pane and no Wingpanel indicator yet.** It is a launcher app (`org.techmore.mseries.Updates`). A Switchboard plug
  would be Vala (like Work Mode) and a separate change; the app is written so a plug can reuse `mseries-update-core`.
- **Not translated.** Strings go through `strings::tr`/`trn`, which return English: there is no gettext domain yet. Plural rules are English.
- **No notification** when an update is available (the optional `mseries-update-auto.timer` is separate); `auto_stage` stays off by default.
- **No package.** `install.sh` is the development route; shipping it in the `mseries-*` apt suites is the next step.
- Keyboard focus and accessible names are asserted in the test, not exercised with a screen reader. Screenshots were reviewed with the
  elementary stylesheet on the development machine; CI renders with whatever stylesheet the runner has.
- A new launcher can show a generic icon until the dock next restarts (restarting the dock for that is the owner's call; this change does not do it).
