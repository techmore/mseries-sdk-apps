#!/usr/bin/python3 -I
"""mseries-battery: read the battery history the logger keeps.

  mseries-battery diagnose          check the battery stack (sensors, logger, data, condition)
  mseries-battery daily [days]      energy and time on battery per day
  mseries-battery import            backfill from UPower's own battery history
  mseries-battery open              open the Battery app
  mseries-battery status            health, current draw and runtime estimate
  mseries-battery sessions [days]   each unplugged run: length, charge used, average watts, projected runtime
  mseries-battery profile [days]    average draw and runtime by CPU load band, plus the biggest CPU users
  mseries-battery events [days]     power steps and suspend gaps, with the likely cause
  mseries-battery apps [days]       estimated energy by app while on battery
  mseries-battery charging [days]   charging runs and standby drain
  mseries-battery devices           accessory batteries (mouse, keyboard)
  mseries-battery health            capacity and cycle trend by day
  mseries-battery tips [days]       suggestions backed by your own samples, and power-profile comparison
  mseries-battery report [days]     Markdown report on stdout (e.g. > battery.md)
  mseries-battery export [days]     samples as CSV on stdout
"""
import csv
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import battery_core as core  # noqa: E402


def stamp(ts):
    return time.strftime("%a %d %b %H:%M", time.localtime(ts))


def days_arg(argv, default):
    try:
        return float(argv[0])
    except (IndexError, ValueError):
        return default


def main(argv):
    command = argv[0] if argv else "status"
    rest = argv[1:]
    if command == "import":
        wconn = core.connect()
        print("Imported %d samples from UPower history." % core.import_upower(wconn))
        return 0
    if command == "open":
        os.execv("/usr/bin/python3", ["python3", os.path.join(os.path.dirname(os.path.realpath(__file__)), "mseries-battery-app.py")])
    try:
        conn = core.connect(readonly=True)
    except Exception:
        print("No history yet. Is mseries-battery-logger.service running?", file=sys.stderr)
        return 1
    if command == "status":
        health = core.battery_health()
        rows = core.fetch(conn, int(time.time()) - 6 * 3600)
        if rows:
            last = rows[-1]
            print("Charge      %.0f%%  (%s)" % (last["pct"] or 0, last["status"]))
            if last["sys_w"] is not None:
                print("System draw %.1f W   CPU %.0f%% @ %.0f MHz" % (last["sys_w"], last["cpu_pct"] or 0, last["cpu_mhz"] or 0))
            est = core.estimate_runtime(rows)
            if est:
                print("Runtime     %s left, %s from full  (at %.1f W, last hour)" % (
                    core.fmt_hours(est["hours_left"]), core.fmt_hours(est["hours_full"]), est["avg_w"]))
        if health:
            print("Health      %s%% of design (%.1f / %.1f Wh), %d cycles, limit %s%%" % (
                health["health_pct"], health["full_wh"], health["design_wh"], health["cycles"] or 0,
                int(health["limit_pct"]) if health["limit_pct"] is not None else "—"))
    elif command == "sessions":
        runs = core.sessions(core.fetch(conn, int(time.time() - days_arg(rest, 14) * 86400)))
        print("%-18s %9s %11s %8s %7s %9s" % ("Unplugged", "Length", "Charge", "Used", "Avg W", "Full run"))
        for run in reversed(runs):
            print("%-18s %9s %5.0f→%3.0f%% %6.1fWh %7s %9s" % (
                stamp(run["start"]), core.fmt_hours(run["seconds"] / 3600), run["pct_start"] or 0, run["pct_end"] or 0,
                run["wh_used"], run["avg_w"] or "—", core.fmt_hours(run["projected_hours"])))
    elif command == "profile":
        rows = core.fetch(conn, int(time.time() - days_arg(rest, 14) * 86400))
        print("%-9s %-8s %7s %8s %12s" % ("Usage", "Draw", "Hours", "Avg W", "Runtime"))
        for band in core.usage_profile(rows):
            print("%-9s %-8s %7.1f %8s %12s" % (band["band"], band["cpu_range"], band["hours"],
                                                band["avg_w"] if band["avg_w"] is not None else "—",
                                                core.fmt_hours(band["runtime_h"])))
        print("\nTop CPU users on battery:")
        for item in core.top_culprits(rows):
            print("  %-24s %5.1f%% of CPU" % (item["name"], item["avg_cpu"]))
    elif command == "events":
        for ev in reversed(core.events(conn, int(time.time() - days_arg(rest, 2) * 86400))):
            if ev["kind"] == "gap":
                info = json.loads(ev["detail"])
                print("%s  asleep/off %s, charge %.0f%% → %.0f%%" % (
                    stamp(ev["ts"]), core.fmt_hours(info["seconds"] / 3600), info["pct_from"] or 0, info["pct_to"] or 0))
            else:
                print("%s  %-4s %+5.1f W (%.1f → %.1f W)  CPU %s%%  top: %s  — %s" % (
                    stamp(ev["ts"]), ev["kind"], ev["delta_w"], ev["before_w"], ev["after_w"],
                    "%.0f" % ev["cpu_pct"] if ev["cpu_pct"] is not None else "—", ev["culprit"] or "—", ev["detail"]))
    elif command == "apps":
        apps = core.energy_by_app(core.fetch(conn, int(time.time() - days_arg(rest, 14) * 86400)))
        if not apps["apps"]:
            print("Not enough time on battery yet.")
            return 0
        print("Total %.1f Wh on battery; idle floor %.1f W = %.1f Wh (display, radios, memory)\n" % (
            apps["total_wh"], apps["floor_w"], apps["floor_wh"]))
        print("%-26s %8s %7s" % ("App", "Wh", "Share"))
        for app in apps["apps"][:15]:
            print("%-26s %8.2f %6.1f%%" % (app["name"], app["wh"], app["share"]))
        print("%-26s %8.2f" % ("(other CPU, kernel)", apps["other_wh"]))
    elif command == "charging":
        since = int(time.time() - days_arg(rest, 14) * 86400)
        print("%-18s %9s %11s %8s %7s" % ("Charging", "Length", "Charge", "Added", "Avg W"))
        for run in reversed(core.charge_sessions(core.fetch(conn, since))):
            print("%-18s %9s %5.0f→%3.0f%% %6.1fWh %7s" % (stamp(run["start"]), core.fmt_hours(run["seconds"] / 3600),
                                                         run["pct_start"] or 0, run["pct_end"] or 0, run["wh_added"], run["avg_w"]))
        drain = core.standby_drain(conn, since)
        print("\nStandby: %.1f h asleep on battery, %.1f%% lost%s" % (
            drain["hours"], drain["pct_lost"], " (%.2f%%/h)" % drain["pct_per_hour"] if drain["pct_per_hour"] is not None else ""))
    elif command == "health":
        for day in core.health_trend(conn):
            print("%s  %.1f Wh  %4.1f%% of design  %d cycles" % (day["day"], day["full_uwh"] / 1e6,
                  100.0 * day["full_uwh"] / day["design_uwh"] if day["design_uwh"] else 0, day["cycles"]))
    elif command == "devices":
        for dev in core.device_summary(conn):
            print("%-40s %s%s  since %s%s" % (dev["name"], dev["level"] or dev["status"],
                  " %.0f%%" % dev["pct"] if dev["pct"] is not None else "", stamp(dev["since"]),
                  "  (~%.0f days left)" % dev["days_left"] if dev["days_left"] else ""))
    elif command == "diagnose":
        worst = 0
        for c in core.diagnose(conn):
            print("%-4s %-26s %s" % (c["status"].upper(), c["name"], c["detail"]))
            worst = max(worst, {"ok": 0, "warn": 1, "fail": 2}[c["status"]])
        return worst
    elif command == "daily":
        rows = core.fetch(conn, int(time.time() - 40 * 86400))
        print("%-11s %9s %9s %9s %8s" % ("Day", "On battery", "Charger", "Energy", "Low"))
        for d in core.daily_usage(rows, int(days_arg(rest, 10))):
            print("%-11s %9s %9s %7.1fWh %7s" % (d["day"], core.fmt_hours(d["battery_h"]), core.fmt_hours(d["charger_h"]), d["wh"],
                  "%.0f%%" % d["min_pct"] if d["min_pct"] is not None else "—"))
    elif command == "tips":
        rows = core.fetch(conn, int(time.time() - days_arg(rest, 14) * 86400))
        for item in core.profile_breakdown(rows):
            print("%-12s %7s on battery  avg %.1f W  idle %s  light %s" % (
                item["label"], core.fmt_hours(item["hours"]), item["avg_w"],
                "%.1f W" % item["idle_w"] if item["idle_w"] else "—", "%.1f W" % item["light_w"] if item["light_w"] else "—"))
        tips = core.suggestions(conn, rows)
        print("" if tips else "\nNo suggestions yet (needs an hour or more on battery).")
        for tip in tips:
            print("* %s\n  %s" % (tip["title"], tip["body"]))
    elif command == "report":
        print(core.report_markdown(conn, days_arg(rest, 14)))
    elif command == "export":
        writer = csv.writer(sys.stdout)
        writer.writerow(core.COLUMNS)
        for row in core.fetch(conn, int(time.time() - days_arg(rest, 30) * 86400)):
            writer.writerow([row[c] for c in core.COLUMNS])
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
