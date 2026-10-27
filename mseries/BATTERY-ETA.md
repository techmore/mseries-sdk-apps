# Native battery time to full

The elementary Power indicator already receives UPower's `TimeToFull` estimate,
but the panel previously showed it only in battery details and descriptive
text. This change adds a compact, rounded estimate beside the existing native
battery icon while charging, such as `1h 24m`; it hides the estimate when the
computer is not charging or UPower has no positive estimate. The label tooltip
spells out the approximate time to full.

`patches/panel-power-charge-eta.patch` applies to the official
`elementary/panel-power` source at commit
`e05723d8cba82b596a5efc259b4e0d15ad9a2bc4`. `install-battery-eta.sh` fetches
that exact commit, applies the patch, builds the native Wingpanel module, checks
its dependencies, backs up the installed module to
`/usr/local/lib/mseries/libpower.so.before-charge-eta`, installs the new module,
and restarts Wingpanel. Run the script with `--uninstall` to restore the backup.
The installer targets the elementary Power indicator 8.0.2 package family.

On the M1 desktop, UPower reported a positive time-to-full while charging. The
patched module built successfully, installed, and Wingpanel restarted and
remained running. The original module was retained as a backup. A visual check
of the panel label was not available in the isolated desktop command session.

## Watt meter in the menu bar (added 2026-10-14)

`patches/panel-power-watts.patch` adds a small label beside the battery icon with the live power draw from
UPower's `EnergyRate` (for example `12.1 W`), whether discharging or charging; it hides below 0.1 W (fully
charged on AC). The tooltip says whether the battery is charging at or the computer is using that power.
It is applied after the ETA patch by the same installer (`install-battery-eta.sh`), so both ship together,
and `--uninstall` restores the original module as before.

Verified: both patches apply in order to `elementary/panel-power` at `e05723d` with plain `git apply`, and the
patched indicator compiles with valac 0.56.19 into `libpower.so` (checked in a scratch tree with the
build-only `libgtop-2.0` dependency removed, since no Vala code uses it). Not verified: installation on the
running desktop and how the label looks in the panel (needs sudo and a login/panel restart).

Fix in the same change: `panel-power-charge-eta.patch` used zero-context hunks. With current `git apply`
they were placed after the end of the class in `DisplayWidget.vala`, which cannot compile; the patch was
regenerated with three lines of context and now lands in the right place.

Install: `sudo apt install libgtop2-dev` (build dependency), then `bash mseries/install-battery-eta.sh`
(or `bash mseries/apply-pending.sh --only wattmeter`).

Spacing (2026-10-15): at the owner's request the watt label has an 8 px left margin so it sits clear of the
battery icon and percentage. Installed on the M1 Pro; the owner confirms the look.

Update (2026-10-15): the label now shows the whole machine's power from the SMC ("Total System Power" in
`/sys/class/hwmon/*/power*_input`, chip `macsmc_hwmon`), updated every 2 s, on battery and on the charger. The first
version showed UPower's battery flow, which is exactly 0 W when the charger is connected and the battery sits at the 80%
limit, so the label hid and the owner reported the meter as lost. Falls back to UPower's rate if the sensor is missing.
Compiled and installed on the M1 Pro; how it looks in the panel is for the owner to confirm.


## Menu-bar watts toggle — 2026-10-18

Open the battery menu and turn **Show Watts in Menu Bar** off to hide the watts label immediately. Turn it on to show the latest reading again. The choice persists in `~/.config/mseries/battery-meter.ini` (`[panel] show-watts`), shared by the menu and bar; missing configuration keeps the existing enabled default. Percentage and charging-time displays keep their existing controls.

The installer applies `panel-power-watts-toggle.patch` after the ETA/watts patches. Verified with a full release build and an isolated GTK harness: menu binding hides/shows the label immediately, later sensor updates cannot unhide it, and a second process restores the saved off choice. Installed live with `bash mseries/install-battery-eta.sh`; Wingpanel restarted successfully. The live menu still needs an owner visual check.

## Battery History button (added 2026-10-18)

`patches/panel-power-battery-history.patch` adds a "Battery History…" item above "Power Settings…" in the Power
popover. It opens the Battery app (`org.mseries.Battery.desktop`, see BATTERY-PROFILER.md) and is only shown when that
desktop file is installed. Applied third by `install-battery-eta.sh`. `--build-only` builds and checks the module
without installing or restarting Wingpanel. Verified: all three patches apply in order to `e05723d` and the module
builds (release, valac) with no unresolved libraries. **Not yet installed**: it needs a Wingpanel restart, which waits for
the owner's go-ahead; the button's look and click are unverified.

Update (2026-10-18): the Battery History build is **installed** on the M1 Pro (`strings` on the installed `libpower.so`
shows the new label); Wingpanel restarted and stayed up. The original module is still at
`/usr/local/lib/mseries/libpower.so.before-charge-eta` for `--uninstall`. The button's look and the click are for the
owner to confirm.

Update (2026-10-18, later): the Battery History patch now replaces the "Battery History…" menu item with a **summary
card** that takes the place of the stock battery rows in the Power popover while the Battery logger has data: headline
("5h 40m left", "Charging · 1h 24m to full" or "7h 12m typical runtime"), charge and system watts, a six-hour watts
sparkline, and a "Battery History…" hint; clicking it opens the Battery app. The card reads
`~/.cache/mseries-battery/summary.txt` (three lines, written by the logger once a minute) and refreshes every 15 s and when
the popover opens. Without that file the stock rows stay. Accessory batteries (mouse) leave the popover with the stock
rows and live on the app's Accessories page. Builds clean with `--build-only`; not installed yet, appearance unverified.


Update (2026-10-18, dropdown): the card is now the whole battery dropdown content: headline, charge and watts, a six-hour
sparkline, Typical runtime and Health, the three most recent runs on battery, the biggest CPU users, and the top
suggestion, all from real logger/UPower data in `summary.txt`. Clicking anywhere on it opens the Battery app. Built and
installed on the M1 Pro (Wingpanel restarted and stayed up); its look on the live bar is not yet confirmed.

Update (2026-10-18, dropdown 2): the dropdown also lists accessory batteries (the mouse, which the stock rows used to
show) and a Sleep drain stat once there is a night asleep on battery. Installed; Wingpanel restarted and stayed up.

Update (2026-10-18, dropdown 3): the dropdown adds "Recent power changes" (the last three rises/drops from the last 6 h
with the likely cause and busiest process), "Power profiles" (average and idle draw per profile once more than one has
been used on battery) and peak-watt / 6 h labels on the sparkline. Installed; Wingpanel restarted and stayed up.

Update (2026-10-18, verified on the live bar): a screenshot of the real desktop (portal screenshot) shows the watts label
("15.2 W") beside the battery icon and percentage. The dropdown card cannot be opened without a click on the owner's
screen, so it was rendered from the same HistoryCard code against the real summary file under the installed elementary
stylesheet; that showed wrapping rows and a collision between the sparkline label and the line, now fixed (ellipsized
rows, shortened causes, peak label at the top right, "6 h ago" at the bottom left).

Correction (2026-10-18): the watts label looked "missing" on the live bar because the owner's saved choice is off
(`~/.config/mseries/battery-meter.ini`, `show-watts=false`, the switch from `panel-power-watts-toggle.patch`). It is not a bug.
The earlier screenshot that showed "15.2 W" came from a hand-built module without that toggle patch; the installer build
(all four patches) was restored afterwards. Always install via `install-battery-eta.sh`, never a partial build.

Verified live (2026-10-18): the dropdown was opened on the owner's real desktop (one pointer click on the battery icon,
approved by the owner, then Escape) and captured with the screenshot portal. It shows the card (headline, charge and watts,
six-hour sparkline, Typical runtime, Health, recent runs, power changes, accessories) above the stock brightness slider and
power profiles, under the "Show Percentage" and "Show Watts in Menu Bar" switches. Stat values were indented against their
labels; fixed (heading style instead of h4). Rows are one line each at the popover's width.

## Retired: card-style menu patch — 2026-10-19

A second menu redesign (`panel-power-battery-menu.patch`: headline card, health/cycles/temperature tiles, charge-limit state,
segmented power modes) was written without knowing `dev` already had the Battery History menu above. It did not apply on top of it
and was dropped; the patch file is removed. The `ChargeCycles` and `ChargeEndThreshold` UPower properties it read are not used by dev.

## Level-aware battery gauge (added 2026-10-21)

UPower's panel icon only has a few steps (`battery-good-symbolic` anywhere around 30–60%) and no colour.
`patches/panel-power-level-gauge.patch` adds `Widgets/BatteryGauge.vala`, a drawn battery whose fill height tracks the
exact charge and whose colour eases lime → amber → muted strawberry as it drops (bolt overlay while charging). The
Battery app draws the same gauge. Non-battery states (loading, backlight) keep the symbolic icon. Applied last by
`install-battery-eta.sh`. Built and loaded in the live Wingpanel; the owner still needs to eyeball it in the bar.

The gauge also carries the percentage as a number on the battery (`patches/panel-power-gauge-percentage.patch`, applied
after the level gauge; the app's gauge does the same). Digits are dark over the fill, foreground colour over the empty
part, with a light halo where the fill edge crosses them. While charging, a small bolt sits above the number.
