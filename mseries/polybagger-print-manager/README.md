# PolyBagger T1000 Print Manager

Native elementary OS app (GTK 4 + Granite 7) and CLI for the label printer on the Advanced Poly T-1000
bagger: a Zebra ZTC 110Xi4-203dpi engine in a TI-1000Z inline frame, on USB. It finds the printer when it
is plugged in, loads a label for a set number of bags, confirms the printer received it, then walks the
operator through checking the bagger screen and one Manual Cycle.

The printer picker also supports other ZPL-compatible USB printers and any ZPL-compatible network or
shared printer already configured in CUPS. USB printers get separate private raw queues based on their
serial numbers. The app remembers the selected queue and loaded-label state per printer, and polls USB
connections so unplugging and reconnecting a device updates its status without restarting the app.
Identical USB printers without serial numbers cannot be distinguished safely.

Label sources: [techmore/thril](https://github.com/techmore/thril) `poly-bagger/UPC Labels`
(site: https://techmore.github.io/thril/poly-bagger.html). Printer manual: Advanced Poly TI-1000Z
inline thermal printer, "Printer Settings" and "Stored Labels", pp. 20–22.

```bash
mseries/polybagger-print-manager/install.sh                          # launcher, app icon, offline label sync
python3 mseries/polybagger-print-manager/polybagger_print_manager.py # GUI
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --status
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --status --printer QUEUE_OR_SERIAL
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --send "<file>.zpl" --qty 1000
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --send "<file>.zpl" --printer QUEUE_OR_SERIAL --qty 1000
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --setup --printer USB_SERIAL
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --setup --printer USB_IDENTITY # serial-less device
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --cancel        # your stuck jobs
python3 mseries/polybagger-print-manager/polybagger_print_manager.py --sync --pull   # refresh offline labels
```

## How this machine prints

The printer prints onto the bag film only when the bagger cycles. Sending ZPL **loads** a label; it does
not print by itself.

1. On the bagger's Printer Settings screen: printer **ON**, **Run** mode (settings only change in Setup mode).
2. Set **Labels to print** to the bags in the run (for example 1000). It is sent as `^PQ`, the label's Quantity.
3. Send. The bagger screen changes from **No Label** to **New Label**, or **StoredLabel** after a recall.
4. Each bag cycle (Manual Cycle, Test or auto) prints one label. When the quantity is used up the label
   clears and the screen returns to No Label.
5. **ReprintBuffer / Reprint ON** keeps printing the same label every cycle. **Clear Labels** unloads the
   current label.

The library's files carry no `^PQ`, so a plain send is a quantity of 1: one bag, then the label clears.
That matches what we saw on 2026-10-21 (three sends, three labels, then a clear label space). The app adds
`^PQ<n>` to a temporary copy at send time; the library files are never changed. Store files (`^DF`) are
sent unchanged because they load nothing.

## Icon
`io.techmore.PolyBaggerPrintManager.svg`: elementary-style tan industrial label printer (media window, green
ready light, a barcode label feeding out). `install.sh` installs it to `hicolor` (SVG plus
PNG 16–256) so it shows in the app grid and dock under any icon theme. Pin it from the dock's context menu.

## The window
- **Printer picker** (status bar): choose among configured CUPS queues and detected USB printers. A detected
  printer without a queue offers **Set Up Print Queue…**; network printers must first be added in System Settings.
- **Status bar**: a dot with **Ready** (green), **Confirm label / Ready, no label** (amber) or **Not ready**
  (red), then the selected printer's connection, queue state, loaded-label state and last job.
- **Sidebar**: Favorites (star a label), Recent (last six sends) and All labels. Search filters the full
  library; Ctrl+F or just start typing.
- **Print card**: label, preview, How many (with 1 / 100 / 500 / 1000 presets) and one button that says what
  it will do ("Print 1,000 Labels"; Ctrl+P). The count is remembered.
- **Result card**: what was sent, and the one check that matters: does the bagger show the label?
- **Today**: every job today with time, label and count, plus totals (sent, labels, failed).
- **Details** (collapsed): connection checks, Send Test Label, activity log.
- **Help** (?): the machine flow and the known issues below.

## What the app checks
- USB: printer-class devices (plus Zebra vendor-specific devices) from sysfs, polled every second; individual
  devices joining and leaving are detected, not just whether any printer is connected.
- CUPS: all configured destinations are selectable. USB queues are matched to a connected device by serial,
  or by a unique USB URI when the device has no serial;
  the app will not substitute a different queue when the selected printer is unplugged. Queue state and
  stalled jobs are checked every 5 s.
- Setup: **Set Up Print Queue…** creates a private raw ZPL queue for the selected USB device, using its serial
  and `printer-error-policy=retry-job`. Setup requires administrator authentication.
- Network: select any network or shared CUPS queue configured outside the app. It must accept raw ZPL; the
  app does not convert ZPL to another printer language.
- File: `^XA`…`^XZ` present; warns on `^PON`/`^FWN`, which this printer rejects.
- Delivery: `lp -n 1 -o raw`, then CUPS must report the job completed within 60 s. "Delivered" means the
  printer received the data, nothing more.
- Operator: step 1, does the bagger screen show the label loaded? Step 2, after one Manual Cycle, how many
  labels printed (none / exactly one / more than one)? Each answer is logged.
- Asks before re-sending the same file within 2 minutes.

Data lives in `~/.local/share/polybagger-print-manager/`: `labels/` (offline copy of every `.zpl` plus
`manifest.json` with sha256, refreshed from the local thril checkout on every start; the refresh button or
`--sync --pull` also `git pull`s and fails quietly offline), `log.jsonl` (every job, selected queue and answer) and
`settings.json` (last quantity, selected printer and per-printer label states).

The preview is drawn from the file's `^GFA` bitmap and inverted 180° for auditing only. Labels are sent
upside down on purpose. A recall file previews the stored copy it recalls.

## Known issues (2026-10-22)
- Only printers that accept ZPL can print these labels. Adding an arbitrary CUPS printer to the picker does
  not translate ZPL into another printer language.
- USB printers without a serial can be matched when only one identical model is connected. Multiple
  indistinguishable devices are left unavailable rather than risk sending to the wrong unit.
- **Delivered is not printed.** CUPS "completed" only means the printer received the data. A label prints
  on a bag cycle, so confirm on the bagger screen.
- **The printer stops taking data.** Jobs sat at "waiting for printer" for over 60 s while the printer still
  showed on USB, and once it dropped off USB entirely. A printer reboot cleared it, after which three sends
  worked. The app shows "Printer not taking jobs" and offers **Cancel Stuck Jobs** (yours only; `cancel -a`
  needs admin).
- **CUPS paused the queue after a printer power cycle** (2026-10-21, job in flight), leaving Send disabled.
  **Resume Queue…** (admin password) clears your leftover jobs, re-enables the queue and sets
  `printer-error-policy=retry-job` so a power cycle no longer pauses it. Applied to `ZTC-110Xi4-203dpi-ZPL`.
- **Existing queues may use a vendor driver.** The app sends with CUPS raw mode. New USB queues created by
  the app are private raw queues; setup still needs administrator authentication.
- **Repeat sends.** Sending the same label twice re-loads it. With Reprint on it can keep printing every
  cycle. Use the quantity instead of re-sending.
- **Flash overwrite during testing.** A test sent `CCSBLACK_store_flash.zpl` to printer `JJK245392`, which
  rewrote `E:CCSBLACK.ZPL` with the repo's copy. The thril notes name printer `JJL100932`; this is the
  other unit.
- **Printer hardware.** The printer has separate intermittent faults. Wiring notes:
  [../../docs/polybagger-t1000-printer-link-troubleshooting.md](../../docs/polybagger-t1000-printer-link-troubleshooting.md).

## Not verified
- Multiple physical printers, USB unplug/replug behavior, and `--setup` on each supported printer model.
- A `--qty` above 1 on the machine (printer behavior is from the manual).
- A full GUI send on the machine. Earlier UI renders covered the single-printer version; they do not qualify
  the new printer picker or hotplug flow.

## Development
`PBPM_SHOT=/path.png` renders the window to a PNG and quits (`PBPM_SHOT_H` sets the height,
`PBPM_SHOT_ASK=1` shows the confirmation card). Render off-screen so the window does not take keyboard
focus on the desktop:

```bash
env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSK_RENDERER=cairo PBPM_SHOT=/tmp/pbpm.png \
  xvfb-run -a -s "-screen 0 1400x1400x24" python3 polybagger_print_manager.py
```
