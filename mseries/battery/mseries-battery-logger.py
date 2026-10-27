#!/usr/bin/python3 -I
"""Battery history logger: samples power, CPU and top processes every few seconds into SQLite.

Runs as a user service (mseries-battery-logger.service). `--once` takes two samples a second apart and
prints the second, for checks. Never changes any setting; it only reads sysfs and procfs.
"""
import os
import shutil
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import battery_core as core  # noqa: E402


def notify(alert):
    """Tell the owner about sustained high draw. Uses notify-send; silently skipped if it is missing."""
    tool = shutil.which("notify-send")
    if not tool:
        return
    who = " %s is using the most CPU." % alert["culprit"] if alert["culprit"] else ""
    body = "Drawing %.0f W for the last few minutes.%s At this rate the battery lasts %s." % (
        alert["watts"], who, core.fmt_hours(alert["hours_left"]))
    subprocess.run([tool, "--app-name=Battery", "--icon=battery-caution-symbolic", "--category=device",
                    "High power use", body], check=False, timeout=10)


def main(argv):
    interval = core.SAMPLE_SECONDS
    sampler = core.Sampler()
    if not sampler.battery:
        print("No laptop battery found; nothing to log.", file=sys.stderr)
        return 0
    if "--once" in argv:
        sampler.sample()
        time.sleep(1)
        print(sampler.sample())
        return 0

    conn = core.connect()
    try:
        core.import_upower(conn)           # real history UPower kept from before this logger existed
    except Exception as error:
        print("UPower import skipped: %s" % error, file=sys.stderr)
    detector = core.EventDetector()
    alert = core.DrainAlert(core.load_config())
    last_day = None
    devices = {}
    last_devices = 0
    last_summary = 0
    stop = []
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda *_: stop.append(1))
    sampler.sample()                      # primes the CPU and per-process counters
    last_prune = 0
    while not stop:
        time.sleep(interval)
        if stop:
            break
        try:
            row = sampler.sample()
            core.store(conn, row)
            detector.feed(conn, row)
            day = time.strftime("%Y-%m-%d")
            if day != last_day:
                core.snapshot_daily(conn, day)
                last_day = day
            if row["ts"] - last_devices >= 300:
                core.log_devices(conn, row["ts"], devices)
                last_devices = row["ts"]
            conn.commit()
            if row["ts"] - last_summary >= 60:
                core.write_summary(conn)           # read by the panel menu item
                last_summary = row["ts"]
            alert.config = core.load_config()     # picks up Preferences changes without a restart
            fired = alert.feed(row)
            if fired:
                notify(fired)
            if row["ts"] - last_prune > 86400:
                core.prune(conn)
                last_prune = row["ts"]
        except Exception as error:  # keep logging through a transient sysfs hiccup
            print("sample failed: %s" % error, file=sys.stderr)
    conn.close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
