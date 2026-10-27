"""Shared battery history code: sampling, SQLite storage, sessions, power events and runtime estimates.

Used by the logger service, the `mseries-battery` command and the Battery app. Everything here is plain
sysfs/procfs plus sqlite3, so it works on any Linux laptop; the Apple SMC hwmon "Total System Power" sensor
is used when it exists and the battery's own flow is the fallback.
"""
import glob
import json
import re
import os
import shutil
import sqlite3
import subprocess
import time

DB_PATH = os.path.join(os.environ.get("XDG_DATA_HOME", os.path.expanduser("~/.local/share")),
                       "mseries-battery", "history.db")
SAMPLE_SECONDS = 15
GAP_SECONDS = SAMPLE_SECONDS * 3      # a longer silence means suspend, shutdown or a crash
SESSION_GAP_SECONDS = 300             # discharge runs split when samples are further apart than this
RETENTION_DAYS = 90
TOP_N = 5                             # processes stored per sample
CONFIG_PATH = os.path.join(os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")), "mseries-battery", "config.json")
DEFAULT_CONFIG = {"alerts": True, "alert_watts": 15.0, "alert_minutes": 5, "alert_cooldown_minutes": 30}
EVENT_STEP_WATTS = 4.0                # system power change that is worth an event
EVENT_MIN_GAP_SECONDS = 60            # at most one event per minute

SCHEMA = """
CREATE TABLE IF NOT EXISTS samples (
    ts INTEGER PRIMARY KEY, status TEXT, ac INTEGER, pct REAL, energy_uwh INTEGER, full_uwh INTEGER,
    batt_w REAL, sys_w REAL, cpu_pct REAL, cpu_mhz REAL, load1 REAL, temp_c REAL, bright_pct REAL, top TEXT,
    profile TEXT);
CREATE TABLE IF NOT EXISTS events (
    ts INTEGER, kind TEXT, delta_w REAL, before_w REAL, after_w REAL, cpu_pct REAL, culprit TEXT, detail TEXT);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
CREATE TABLE IF NOT EXISTS devices (
    ts INTEGER, name TEXT, level TEXT, pct REAL, status TEXT);
CREATE INDEX IF NOT EXISTS devices_ts ON devices(ts);
CREATE TABLE IF NOT EXISTS daily (
    day TEXT PRIMARY KEY, full_uwh INTEGER, design_uwh INTEGER, cycles INTEGER, limit_pct INTEGER);
"""


def rd(path, default=None):
    try:
        with open(path) as handle:
            return handle.read().strip()
    except OSError:
        return default


def rd_num(path, scale=1.0):
    value = rd(path)
    try:
        return float(value) * scale
    except (TypeError, ValueError):
        return None


SYS = os.environ.get("MSERIES_BATTERY_SYSROOT", "")   # tests point this at a fake /sys


def find_battery():
    """The laptop's own battery, not a mouse or keyboard (those report scope=Device)."""
    for node in sorted(glob.glob(SYS + "/sys/class/power_supply/*")):
        if rd(node + "/type") == "Battery" and rd(node + "/scope", "System") != "Device":
            return node
    return None


def read_devices(root=None):
    root = root or SYS + "/sys/class/power_supply"
    """Accessory batteries (mouse, keyboard, ...): scope=Device. Many report only a level, not a percentage."""
    found = []
    for node in sorted(glob.glob(root + "/*")):
        if rd(node + "/type") == "Battery" and rd(node + "/scope") == "Device":
            found.append({"name": rd(node + "/model_name") or os.path.basename(node),
                          "level": rd(node + "/capacity_level"), "pct": rd_num(node + "/capacity"),
                          "status": rd(node + "/status")})
    return found


def log_devices(conn, now, last):
    """Store an accessory reading only when it changed (or hourly), so the table stays tiny. Returns the new state."""
    for dev in read_devices():
        key = (dev["level"], dev["pct"], dev["status"])
        before = last.get(dev["name"])
        if before is None or before[0] != key or now - before[1] >= 3600:
            conn.execute("INSERT INTO devices VALUES (?,?,?,?,?)", (now, dev["name"], dev["level"], dev["pct"], dev["status"]))
            last[dev["name"]] = (key, now)
    return last


def device_summary(conn):
    """Latest reading and the time of the last level change per accessory."""
    out = []
    try:
        names = [r[0] for r in conn.execute("SELECT DISTINCT name FROM devices")]
        for name in names:
            rows = [dict(r) for r in conn.execute("SELECT * FROM devices WHERE name=? ORDER BY ts", (name,))]
            latest = rows[-1]
            since = latest["ts"]
            for older in reversed(rows[:-1]):
                if older["level"] != latest["level"] or older["status"] != latest["status"]:
                    break
                since = older["ts"]
            # percentage devices: days left from the slope of the last week, when it is falling
            days_left = None
            pts = [(r["ts"], r["pct"]) for r in rows if r["pct"] is not None and r["ts"] >= latest["ts"] - 7 * 86400]
            if len(pts) >= 3 and pts[0][1] > pts[-1][1] and pts[-1][0] > pts[0][0]:
                rate = (pts[0][1] - pts[-1][1]) / ((pts[-1][0] - pts[0][0]) / 86400)
                days_left = round(pts[-1][1] / rate, 1) if rate > 0 else None
            out.append({"name": name, "level": latest["level"], "pct": latest["pct"], "status": latest["status"],
                        "since": since, "seen": latest["ts"], "days_left": days_left,
                        "changes": [r for r in rows if r["ts"] >= rows[0]["ts"]][-8:]})
    except sqlite3.OperationalError:
        pass
    return out


def find_system_power():
    for label in glob.glob(SYS + "/sys/class/hwmon/hwmon*/power*_label"):
        if (rd(label) or "").lower() == "total system power":
            return label.replace("_label", "_input")
    return None


def find_ac():
    for node in sorted(glob.glob(SYS + "/sys/class/power_supply/*")):
        if rd(node + "/type") == "Mains":
            return node + "/online"
    return None


def connect(path=DB_PATH, readonly=False):
    if readonly:
        conn = sqlite3.connect("file:%s?mode=ro" % path, uri=True, timeout=10)
    else:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        conn = sqlite3.connect(path, timeout=10)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.executescript(SCHEMA)
        if "profile" not in [r[1] for r in conn.execute("PRAGMA table_info(samples)")]:
            conn.execute("ALTER TABLE samples ADD COLUMN profile TEXT")  # database from before profiles were logged
    conn.row_factory = sqlite3.Row
    return conn


_APP_NAMES = {}


def pretty_app(key):
    """Display name for a cgroup/app id: the .desktop Name= if one exists, else a tidied version of the id."""
    if key in _APP_NAMES:
        return _APP_NAMES[key]
    name = None
    for base in ("/usr/share/applications", os.path.expanduser("~/.local/share/applications")):
        for candidate in (key, key.lower()):
            text = rd("%s/%s.desktop" % (base, candidate))
            if text:
                match = re.search(r"^Name=(.+)$", text, re.M)
                if match:
                    name = match.group(1).strip()
                    break
        if name:
            break
    if not name:
        tail = key.split(".")[-1] if key.count(".") >= 2 else key
        name = tail[:1].upper() + tail[1:] if tail.islower() else tail
    _APP_NAMES[key] = name
    return name


def app_key(cgroup_text, comm):
    """Stable per-application key from a process's cgroup: app-<launcher>-<name>-<pid>.scope, *.service, else comm."""
    last = (cgroup_text or "").strip().splitlines()[-1].rsplit("/", 1)[-1] if cgroup_text and cgroup_text.strip() else ""
    last = last.replace("\\x2d", "-")
    match = re.match(r"app(?:-[a-z]+)?-(.+?)(?:-\d+)?\.scope$", last)
    if match:
        key = match.group(1)
        return comm if "launcher" in key else key      # apps started from a launcher keep their own name
    # Only system services name an app. User-session services (gala@wayland.service holds Wingpanel, the dock and
    # other shell parts) would lump unrelated processes under one name, so those keep their own process name.
    if last.endswith(".service") and "/system.slice/" in (cgroup_text or "") and not last.startswith(("user@", "session-")):
        return last[:-8].split("@")[0].lstrip("_")
    return comm


class Sampler:
    """Reads one snapshot of power, CPU and process state; keeps the previous counters for deltas."""

    def __init__(self, proc="/proc"):
        self.proc = proc
        self.battery = find_battery()
        self.sys_power = find_system_power()
        self.ac = find_ac()
        self.backlight = (glob.glob(SYS + "/sys/class/backlight/*") or [None])[0]
        self.prev_cpu = None
        self.prev_procs = {}
        self.prev_time = None
        self.profile, self.profile_read = None, 0

    def cpu_times(self):
        fields = (rd(self.proc + "/stat") or "cpu 0 0 0 0").splitlines()[0].split()[1:]
        values = [int(v) for v in fields[:8]]
        idle = values[3] + values[4]
        return sum(values), idle

    def proc_ticks(self):
        ticks = {}
        for entry in os.listdir(self.proc):
            if not entry.isdigit():
                continue
            stat = rd("%s/%s/stat" % (self.proc, entry))
            if not stat:
                continue
            # comm may contain spaces and parentheses, so split on the last ')'
            head, _, tail = stat.rpartition(")")
            parts = tail.split()
            if len(parts) < 13:
                continue
            name = head.partition("(")[2]
            ticks[entry] = (name, int(parts[11]) + int(parts[12]))
        return ticks

    def cpu_mhz(self):
        freqs = [rd_num(p, 0.001) for p in glob.glob(SYS + "/sys/devices/system/cpu/cpu[0-9]*/cpufreq/scaling_cur_freq")]
        freqs = [f for f in freqs if f]
        return sum(freqs) / len(freqs) if freqs else None

    def power_profile(self, now):
        """power-profiles-daemon's active profile, re-read at most once a minute."""
        if now - self.profile_read >= 60:
            self.profile_read = now
            tool = shutil.which("powerprofilesctl")
            if tool:
                try:
                    self.profile = subprocess.run([tool, "get"], capture_output=True, text=True, timeout=5).stdout.strip() or None
                except (OSError, subprocess.SubprocessError):
                    pass
        return self.profile

    def sample(self):
        now = int(time.time())
        total, idle = self.cpu_times()
        procs = self.proc_ticks()
        cpu_pct, top = None, []
        if self.prev_cpu:
            dt, di = total - self.prev_cpu[0], idle - self.prev_cpu[1]
            if dt > 0:
                cpu_pct = round(100.0 * (dt - di) / dt, 1)
                grouped = {}
                for pid, (name, used) in procs.items():
                    before = self.prev_procs.get(pid)
                    if before and before[0] == name and used > before[1]:
                        key = pretty_app(app_key(rd("%s/%s/cgroup" % (self.proc, pid)), name))
                        grouped[key] = grouped.get(key, 0) + used - before[1]
                # process ticks are in the same units as /proc/stat, summed over all CPUs
                ranked = sorted(grouped.items(), key=lambda kv: -kv[1])[:TOP_N]
                top = [[name, round(100.0 * ticks / dt, 1)] for name, ticks in ranked if ticks > 0]
        self.prev_cpu, self.prev_procs = (total, idle), procs

        bat = self.battery
        status = (rd(bat + "/status") if bat else None) or "Unknown"
        ac_online = rd(self.ac) if self.ac else None
        ac = int(ac_online) if ac_online in ("0", "1") else (0 if status == "Discharging" else 1)
        energy = rd_num(bat + "/energy_now") if bat else None
        full = rd_num(bat + "/energy_full") if bat else None
        pct = rd_num(bat + "/capacity") if bat else None
        if energy is not None and full:
            pct = round(100.0 * energy / full, 2)
        power = rd_num(bat + "/power_now", 1e-6) if bat else None
        if power is None and bat:
            volts, amps = rd_num(bat + "/voltage_now", 1e-6), rd_num(bat + "/current_now", 1e-6)
            power = abs(volts * amps) if volts and amps else None
        batt_w = None
        if power is not None:
            power = abs(power)
            batt_w = -power if status == "Discharging" else (power if status == "Charging" else 0.0)
        sys_w = rd_num(self.sys_power, 1e-6) if self.sys_power else None
        if sys_w is None and batt_w is not None and status == "Discharging":
            sys_w = -batt_w
        temp = rd_num(bat + "/temp", 0.1) if bat else None
        bright = None
        if self.backlight:
            cur, top_b = rd_num(self.backlight + "/brightness"), rd_num(self.backlight + "/max_brightness")
            bright = round(100.0 * cur / top_b, 1) if cur is not None and top_b else None
        load = rd(self.proc + "/loadavg")
        return {"ts": now, "status": status, "ac": ac, "pct": pct,
                "energy_uwh": int(energy) if energy is not None else None,
                "full_uwh": int(full) if full is not None else None,
                "batt_w": batt_w, "sys_w": sys_w, "cpu_pct": cpu_pct, "cpu_mhz": self.cpu_mhz(),
                "load1": float(load.split()[0]) if load else None, "temp_c": temp, "bright_pct": bright,
                "top": json.dumps(top), "profile": self.power_profile(now)}


COLUMNS = ["ts", "status", "ac", "pct", "energy_uwh", "full_uwh", "batt_w", "sys_w", "cpu_pct", "cpu_mhz",
           "load1", "temp_c", "bright_pct", "top", "profile"]


def store(conn, row):
    conn.execute("INSERT OR REPLACE INTO samples (%s) VALUES (%s)" % (",".join(COLUMNS), ",".join("?" * len(COLUMNS))),
                 [row.get(c) for c in COLUMNS])


def add_event(conn, ts, kind, delta_w=None, before_w=None, after_w=None, cpu_pct=None, culprit=None, detail=None):
    conn.execute("INSERT INTO events VALUES (?,?,?,?,?,?,?,?)",
                 (ts, kind, delta_w, before_w, after_w, cpu_pct, culprit, detail))


def explain_change(prev, row):
    """Best guess at why system power moved, from what else changed between the two samples."""
    if prev["ac"] != row["ac"]:
        return "Charger " + ("connected" if row["ac"] else "unplugged")
    if prev["bright_pct"] is not None and row["bright_pct"] is not None and abs(row["bright_pct"] - prev["bright_pct"]) >= 10:
        return "Screen brightness %d%% → %d%%" % (prev["bright_pct"], row["bright_pct"])
    cpu_a, cpu_b = prev["cpu_pct"], row["cpu_pct"]
    if cpu_a is not None and cpu_b is not None and abs(cpu_b - cpu_a) >= 15:
        return "CPU load %d%% → %d%%" % (cpu_a, cpu_b)
    return "Other (GPU, display, radios, charging)"


class EventDetector:
    """Watches system power for steps and for suspend gaps."""

    def __init__(self):
        self.recent = []
        self.last_row = None
        self.last_event = 0

    def feed(self, conn, row):
        prev = self.last_row
        if prev and row["ts"] - prev["ts"] > GAP_SECONDS:
            lost = (prev["pct"] or 0) - (row["pct"] or 0)
            add_event(conn, prev["ts"], "gap", detail=json.dumps({
                "seconds": row["ts"] - prev["ts"], "pct_from": prev["pct"], "pct_to": row["pct"],
                "on_battery": not row["ac"]}))
            self.recent = []
        watts = row["sys_w"]
        if prev and watts is not None and len(self.recent) >= 3 and row["ts"] - prev["ts"] <= GAP_SECONDS:
            baseline = sum(self.recent[-3:]) / 3
            delta = watts - baseline
            if abs(delta) >= EVENT_STEP_WATTS and row["ts"] - self.last_event >= EVENT_MIN_GAP_SECONDS:
                top = json.loads(row["top"] or "[]")
                add_event(conn, row["ts"], "rise" if delta > 0 else "drop", round(delta, 2), round(baseline, 2),
                          round(watts, 2), row["cpu_pct"], top[0][0] if top else None, explain_change(prev, row))
                self.last_event = row["ts"]
        if watts is not None:
            self.recent = (self.recent + [watts])[-6:]
        self.last_row = row


def prune(conn, days=RETENTION_DAYS):
    cutoff = int(time.time()) - days * 86400
    conn.execute("DELETE FROM samples WHERE ts < ?", (cutoff,))
    conn.execute("DELETE FROM events WHERE ts < ?", (cutoff,))
    conn.commit()


def fetch(conn, since, until=None):
    until = until or int(time.time()) + 1
    return [dict(r) for r in conn.execute("SELECT * FROM samples WHERE ts >= ? AND ts <= ? ORDER BY ts", (since, until))]


def sessions(rows):
    """Unplugged runs: consecutive discharging samples, split by gaps (suspend) and by plugging in."""
    result, current = [], []

    def close():
        if len(current) >= 2:
            first, last = current[0], current[-1]
            span = last["ts"] - first["ts"]
            used = ((first["energy_uwh"] or 0) - (last["energy_uwh"] or 0)) / 1e6
            watts = [r["sys_w"] for r in current if r["sys_w"] is not None]
            cpu = [r["cpu_pct"] for r in current if r["cpu_pct"] is not None]
            full_wh = (last["full_uwh"] or 0) / 1e6
            avg_w = used * 3600 / span if span > 0 and used > 0 else (sum(watts) / len(watts) if watts else None)
            result.append({
                "start": first["ts"], "end": last["ts"], "seconds": span,
                "pct_start": first["pct"], "pct_end": last["pct"], "wh_used": round(used, 2),
                "avg_w": round(avg_w, 2) if avg_w else None,
                "avg_cpu": round(sum(cpu) / len(cpu), 1) if cpu else None,
                "projected_hours": round(full_wh / avg_w, 1) if avg_w and full_wh else None})
        current.clear()

    for row in rows:
        if row["ac"] or row["status"] not in ("Discharging",):
            close()
            continue
        if current and row["ts"] - current[-1]["ts"] > SESSION_GAP_SECONDS:
            close()
        current.append(row)
    close()
    return result


USAGE_BANDS = [("Light", 0, 8), ("Moderate", 8, 15), ("Heavy", 15, 10000)]


def usage_profile(rows):
    """Average draw and full-charge runtime while unplugged, grouped by how hard the machine is working (watts).

    Bands are by system power rather than CPU so they also cover history imported from UPower, which has watts only.
    Each sample is weighted by the time it covers.
    """
    out = []
    full_wh = next((r["full_uwh"] / 1e6 for r in reversed(rows) if r["full_uwh"]), None)
    on = [(i, r) for i, r in enumerate(rows) if not r["ac"] and r["sys_w"] is not None and r["sys_w"] > 0.5]
    for name, low, high in USAGE_BANDS:
        seconds, weighted = 0.0, 0.0
        for i, r in on:
            if low <= r["sys_w"] < high:
                nxt = rows[i + 1]["ts"] if i + 1 < len(rows) else r["ts"] + SAMPLE_SECONDS
                dt = min(nxt - r["ts"], GAP_SECONDS)
                seconds += dt
                weighted += r["sys_w"] * dt
        label = "%d–%d W" % (low, high) if high < 10000 else "%d W and up" % low
        if seconds < 60:
            out.append({"band": name, "cpu_range": label, "samples": 0, "hours": 0.0, "avg_w": None, "runtime_h": None})
            continue
        avg = weighted / seconds
        out.append({"band": name, "cpu_range": label, "samples": int(seconds / SAMPLE_SECONDS), "hours": round(seconds / 3600, 2),
                    "avg_w": round(avg, 2), "runtime_h": round(full_wh / avg, 1) if full_wh and avg > 0 else None})
    return out


def top_culprits(rows, limit=5):
    """Processes that used the most CPU while unplugged, as share of unplugged time."""
    totals, samples = {}, 0
    for r in rows:
        if r["ac"]:
            continue
        samples += 1
        for name, cpu in json.loads(r["top"] or "[]"):
            totals[name] = totals.get(name, 0) + cpu
    if not samples:
        return []
    ranked = sorted(totals.items(), key=lambda kv: -kv[1])[:limit]
    return [{"name": n, "avg_cpu": round(v / samples, 1)} for n, v in ranked]


def typical_draw(rows):
    """Median average draw of past unplugged runs of 30 min or more, for when there is no recent data to go on."""
    draws = sorted(r["avg_w"] for r in sessions(rows) if r["seconds"] >= 1800 and r["avg_w"])
    return draws[len(draws) // 2] if draws else None


def estimate_runtime(rows, window_seconds=3600):
    """Remaining and from-full runtime on battery.

    Blends the last 10 minutes of draw (what you are doing now) with the last hour (what you have been doing); with
    under five minutes on battery it falls back to your typical unplugged draw. Returns a range from the lighter and
    heavier of the two, and what the estimate is based on. Empty while plugged in.
    """
    if not rows or rows[-1]["ac"]:
        return {}
    last = rows[-1]
    on = [r for r in rows if r["ts"] >= last["ts"] - window_seconds and r["sys_w"] is not None and not r["ac"]]
    short = [r["sys_w"] for r in on if r["ts"] >= last["ts"] - 600]
    long_ = [r["sys_w"] for r in on]
    if len(long_) * SAMPLE_SECONDS >= 300:
        short_avg = sum(short) / len(short) if short else sum(long_) / len(long_)
        long_avg = sum(long_) / len(long_)
        avg = 0.6 * short_avg + 0.4 * long_avg
        low_w, high_w = min(short_avg, long_avg), max(short_avg, long_avg)
        basis = "recent use"
    else:
        history = typical_draw(rows)
        if not history:
            return {}
        avg = low_w = high_w = history
        basis = "your typical use"
    if avg <= 0:
        return {}
    energy, full = (last["energy_uwh"] or 0) / 1e6, (last["full_uwh"] or 0) / 1e6
    return {"avg_w": round(avg, 2), "hours_left": round(energy / avg, 2), "hours_full": round(full / avg, 2),
            "left_low": round(energy / max(high_w, 0.1), 2), "left_high": round(energy / max(low_w, 0.1), 2), "basis": basis}


def battery_health():
    bat = find_battery()
    if not bat:
        return {}
    full, design = rd_num(bat + "/energy_full"), rd_num(bat + "/energy_full_design")
    return {"full_wh": full / 1e6 if full else None, "design_wh": design / 1e6 if design else None,
            "health_pct": round(100.0 * full / design, 1) if full and design else None,
            "cycles": rd_num(bat + "/cycle_count"), "model": rd(bat + "/model_name"),
            "limit_pct": rd_num(bat + "/charge_control_end_threshold"),
            "temp_c": rd_num(bat + "/temp", 0.1)}


def events(conn, since, limit=200):
    return [dict(r) for r in conn.execute("SELECT * FROM events WHERE ts >= ? ORDER BY ts DESC LIMIT ?", (since, limit))]


def fmt_hours(hours):
    if hours is None:
        return "—"
    minutes = int(round(hours * 60))
    return "%dh %02dm" % (minutes // 60, minutes % 60) if minutes >= 60 else "%d min" % minutes


# ---- configuration -------------------------------------------------------------------------
def load_config():
    config = dict(DEFAULT_CONFIG)
    try:
        with open(CONFIG_PATH) as handle:
            config.update({k: v for k, v in json.load(handle).items() if k in DEFAULT_CONFIG})
    except (OSError, ValueError):
        pass
    return config


def save_config(config):
    os.makedirs(os.path.dirname(CONFIG_PATH), exist_ok=True)
    with open(CONFIG_PATH, "w") as handle:
        json.dump({k: config[k] for k in DEFAULT_CONFIG}, handle, indent=2)


# ---- daily health snapshots ----------------------------------------------------------------
def snapshot_daily(conn, day=None):
    """One row per day with full-charge capacity and cycle count, for the health trend."""
    health = battery_health()
    if not health.get("full_wh"):
        return
    day = day or time.strftime("%Y-%m-%d")
    conn.execute("INSERT OR REPLACE INTO daily VALUES (?,?,?,?,?)",
                 (day, int(health["full_wh"] * 1e6), int((health["design_wh"] or 0) * 1e6),
                  int(health["cycles"] or 0), int(health["limit_pct"]) if health["limit_pct"] is not None else None))


def health_trend(conn):
    try:
        return [dict(r) for r in conn.execute("SELECT * FROM daily ORDER BY day")]
    except sqlite3.OperationalError:  # database from before the logger added this table
        return []


# ---- energy by app -------------------------------------------------------------------------
def energy_by_app(rows):
    """Estimate where unplugged energy went.

    The idle floor (10th percentile of draw) is charged to the system itself (display, radios, memory). Draw above
    the floor is split among processes by their share of CPU use in each sample. This is an estimate: it sees CPU
    time only, so GPU and display work shows up as the floor or as the app driving it.
    """
    on_battery = [r for r in rows if not r["ac"] and r["sys_w"] is not None and r["cpu_pct"] is not None]
    if len(on_battery) < 4:
        return {"apps": [], "floor_wh": 0.0, "total_wh": 0.0, "floor_w": None, "other_wh": 0.0}
    watts = sorted(r["sys_w"] for r in on_battery)
    floor = watts[len(watts) // 10]
    apps, total, floor_wh, other = {}, 0.0, 0.0, 0.0
    for index, r in enumerate(on_battery):
        nxt = on_battery[index + 1]["ts"] if index + 1 < len(on_battery) else r["ts"] + SAMPLE_SECONDS
        dt = min(nxt - r["ts"], GAP_SECONDS)
        wh = r["sys_w"] * dt / 3600
        total += wh
        base = min(r["sys_w"], floor) * dt / 3600
        floor_wh += base
        dynamic = wh - base
        listed = json.loads(r["top"] or "[]")
        busy = max(r["cpu_pct"], 0.1)
        used = 0.0
        for name, cpu in listed:
            share = min(cpu / busy, 1.0)
            apps[name] = apps.get(name, 0.0) + dynamic * share
            used += share
        other += dynamic * max(0.0, 1.0 - used)
    ranked = sorted(apps.items(), key=lambda kv: -kv[1])
    return {"apps": [{"name": n, "wh": round(w, 3), "share": round(100 * w / total, 1) if total else 0} for n, w in ranked],
            "floor_wh": round(floor_wh, 3), "floor_w": round(floor, 2), "other_wh": round(other, 3), "total_wh": round(total, 3)}


# ---- charging and standby ------------------------------------------------------------------
def charge_sessions(rows):
    """Plugged-in runs where the battery was actually taking charge."""
    result, current = [], []

    def close():
        if len(current) >= 2:
            first, last = current[0], current[-1]
            span = last["ts"] - first["ts"]
            added = ((last["energy_uwh"] or 0) - (first["energy_uwh"] or 0)) / 1e6
            into = [r["batt_w"] for r in current if r["batt_w"]]
            if span >= 120 and added > 0:
                result.append({"start": first["ts"], "end": last["ts"], "seconds": span, "pct_start": first["pct"],
                               "pct_end": last["pct"], "wh_added": round(added, 2),
                               "avg_w": round(added * 3600 / span, 2),
                               "peak_w": round(max(into), 1) if into else None})
        current.clear()

    for row in rows:
        if row["status"] != "Charging":
            close()
            continue
        if current and row["ts"] - current[-1]["ts"] > SESSION_GAP_SECONDS:
            close()
        current.append(row)
    close()
    return result


def standby_drain(conn, since):
    """Charge lost per hour while the machine was asleep or off on battery (from the logged gaps)."""
    seconds, lost = 0.0, 0.0
    for ev in events(conn, since, limit=10000):
        if ev["kind"] != "gap":
            continue
        info = json.loads(ev["detail"])
        drop = (info["pct_from"] or 0) - (info["pct_to"] or 0)
        if info["on_battery"] and info["seconds"] >= 600 and drop >= 0:
            seconds += info["seconds"]
            lost += drop
    return {"hours": round(seconds / 3600, 1), "pct_lost": round(lost, 1),
            "pct_per_hour": round(lost / (seconds / 3600), 2) if seconds >= 3600 else None}


# ---- high drain alert ----------------------------------------------------------------------
class DrainAlert:
    """Decides when sustained high draw on battery deserves a notification."""

    def __init__(self, config):
        self.config = config
        self.window = []
        self.last_alert = None

    def feed(self, row):
        if row["ac"] or row["sys_w"] is None:
            self.window = []
            return None
        self.window = [r for r in self.window + [row] if r["ts"] >= row["ts"] - self.config["alert_minutes"] * 60]
        span = self.window[-1]["ts"] - self.window[0]["ts"]
        if not self.config["alerts"] or span < self.config["alert_minutes"] * 60 - SAMPLE_SECONDS:
            return None
        avg = sum(r["sys_w"] for r in self.window) / len(self.window)
        if avg < self.config["alert_watts"] or (self.last_alert is not None and row["ts"] - self.last_alert < self.config["alert_cooldown_minutes"] * 60):
            return None
        self.last_alert = row["ts"]
        totals = {}
        for r in self.window:
            for name, cpu in json.loads(r["top"] or "[]"):
                totals[name] = totals.get(name, 0) + cpu
        culprit = max(totals, key=totals.get) if totals else None
        left = (row["energy_uwh"] or 0) / 1e6 / avg
        return {"watts": round(avg, 1), "culprit": culprit, "hours_left": round(left, 2)}


# ---- written report ------------------------------------------------------------------------
def report_markdown(conn, days=14):
    now = int(time.time())
    rows = fetch(conn, now - int(days * 86400))
    runs = sessions(rows)
    health = battery_health()
    lines = ["# Battery report", "", "Generated %s, covering the last %g days (%d samples)." % (
        time.strftime("%Y-%m-%d %H:%M"), days, len(rows)), ""]
    if health:
        lines += ["## Health", "", "- Full-charge capacity: %.1f Wh of %.1f Wh design (%s%%)" % (
            health["full_wh"] or 0, health["design_wh"] or 0, health["health_pct"]),
            "- Cycles: %d" % (health["cycles"] or 0),
            "- Charge limit: %s%%" % (int(health["limit_pct"]) if health["limit_pct"] is not None else "none"), ""]
    long_runs = [r for r in runs if r["seconds"] >= 1800 and r["projected_hours"]]
    lines += ["## Runtime", ""]
    if long_runs:
        avg = sum(r["projected_hours"] for r in long_runs) / len(long_runs)
        lines += ["- Average full-charge runtime: **%s** (from %d unplugged runs of 30 min or more)" % (fmt_hours(avg), len(long_runs))]
    else:
        lines += ["- Not enough time on battery yet for a runtime figure."]
    lines += ["- Time on battery: %s over %d runs" % (fmt_hours(sum(r["seconds"] for r in runs) / 3600), len(runs)), ""]
    lines += ["## Runtime by usage", "", "| Usage | CPU | Hours | Avg W | Full-charge runtime |", "|---|---|---|---|---|"]
    for band in usage_profile(rows):
        lines.append("| %s | %s | %.1f | %s | %s |" % (band["band"], band["cpu_range"], band["hours"],
                     band["avg_w"] if band["avg_w"] is not None else "—", fmt_hours(band["runtime_h"])))
    apps = energy_by_app(rows)
    if apps["apps"]:
        lines += ["", "## Where the energy went (estimate, on battery)", "",
                  "Idle floor %.1f W = %.1f Wh; apps share the draw above it by CPU use." % (apps["floor_w"], apps["floor_wh"]), "",
                  "| App | Wh | Share |", "|---|---|---|"]
        lines += ["| %s | %.2f | %s%% |" % (a["name"], a["wh"], a["share"]) for a in apps["apps"][:10]]
    if runs:
        lines += ["", "## Unplugged runs", "", "| Start | Length | Charge | Used | Avg W | Full-charge runtime |", "|---|---|---|---|---|---|"]
        for r in reversed(runs):
            lines.append("| %s | %s | %.0f%% → %.0f%% | %.1f Wh | %s | %s |" % (
                time.strftime("%Y-%m-%d %H:%M", time.localtime(r["start"])), fmt_hours(r["seconds"] / 3600),
                r["pct_start"] or 0, r["pct_end"] or 0, r["wh_used"], r["avg_w"] or "—", fmt_hours(r["projected_hours"])))
    tips = suggestions(conn, rows)
    if tips:
        lines += ["", "## Suggestions", ""] + ["- **%s.** %s" % (t["title"], t["body"]) for t in tips]
    drain = standby_drain(conn, now - int(days * 86400))
    if drain["pct_per_hour"] is not None:
        lines += ["", "## Standby", "", "- Asleep on battery %.1f h, losing %.2f%% per hour" % (drain["hours"], drain["pct_per_hour"])]
    lines += ["", "_Figures are estimates from 15-second samples of the SMC system-power sensor; see mseries/BATTERY-PROFILER.md._", ""]
    return "\n".join(lines)


# ---- power profiles ------------------------------------------------------------------------
PROFILE_NAMES = {"power-saver": "Power Saver", "balanced": "Balanced", "performance": "Performance"}


def profile_breakdown(rows):
    """Time and draw on battery per power profile, also at matching CPU load so the comparison is fair."""
    result = []
    on_battery = [r for r in rows if not r["ac"] and r.get("profile") and r["sys_w"] is not None and r["cpu_pct"] is not None]
    for name in sorted({r["profile"] for r in on_battery}):
        sel = [r for r in on_battery if r["profile"] == name]
        entry = {"profile": name, "label": PROFILE_NAMES.get(name, name), "hours": round(len(sel) * SAMPLE_SECONDS / 3600, 2),
                 "avg_w": round(sum(r["sys_w"] for r in sel) / len(sel), 2)}
        for band, low, high in (("idle_w", 0, 10), ("light_w", 10, 30)):
            part = [r["sys_w"] for r in sel if low <= r["cpu_pct"] < high]
            entry[band] = round(sum(part) / len(part), 2) if len(part) >= 20 else None
        result.append(entry)
    return result


# ---- suggestions ---------------------------------------------------------------------------
def _band_mean(rows, low, high, extra):
    vals = [r["sys_w"] for r in rows if low <= r["cpu_pct"] < high and extra(r)]
    return (sum(vals) / len(vals), len(vals)) if len(vals) >= 20 else (None, len(vals))


def suggestions(conn, rows):
    """Plain-language tips, each backed by this machine's own samples. Empty until there is data to back them."""
    tips = []
    on_battery = [r for r in rows if not r["ac"] and r["sys_w"] is not None and r["cpu_pct"] is not None]
    if len(on_battery) < 240:  # an hour
        return tips
    lit = [r for r in on_battery if r["bright_pct"] is not None]
    if lit:
        avg_bright = sum(r["bright_pct"] for r in lit) / len(lit)
        bright, _ = _band_mean(lit, 0, 30, lambda r: r["bright_pct"] > 70)
        dim, _ = _band_mean(lit, 0, 30, lambda r: r["bright_pct"] <= 50)
        if bright and dim and bright - dim >= 0.8:
            tips.append({"kind": "display", "title": "Dim the screen on battery",
                         "body": "At light CPU load the system drew %.1f W with the screen above 70%% and %.1f W at 50%% or below. "
                                 "Brightness averaged %.0f%% on battery." % (bright, dim, avg_bright)})
    for item in top_culprits(on_battery, 3):
        if item["avg_cpu"] >= 8:
            tips.append({"kind": "app", "title": "%s is busy on battery" % item["name"],
                         "body": "%s averaged %.0f%% of the CPU while unplugged. Closing it or its tabs when away from power "
                                 "extends runtime; see Energy by app." % (item["name"], item["avg_cpu"])})
            break
    profiles = {p["profile"]: p for p in profile_breakdown(rows)}
    if "balanced" in profiles and "power-saver" in profiles:
        a, b = profiles["balanced"]["idle_w"], profiles["power-saver"]["idle_w"]
        if a and b and a - b >= 0.5:
            tips.append({"kind": "profile", "title": "Power Saver idles lower",
                         "body": "At idle, Power Saver drew %.1f W against %.1f W on Balanced." % (b, a)})
    elif "power-saver" not in profiles and "balanced" in profiles:
        tips.append({"kind": "profile", "title": "Try Power Saver on battery",
                     "body": "Every unplugged hour so far was on Balanced. Switch the profile for a while and this page compares "
                             "the two at the same CPU load."})
    drain = standby_drain(conn, rows[0]["ts"])
    if drain["pct_per_hour"] is not None and drain["pct_per_hour"] > 1.0:
        tips.append({"kind": "standby", "title": "Sleep drain is high",
                     "body": "The battery lost %.1f%% per hour while asleep. Something is keeping the machine awake or waking it." % drain["pct_per_hour"]})
    health = battery_health()
    if health.get("health_pct") and health["health_pct"] < 80:
        tips.append({"kind": "health", "title": "Battery capacity is below 80%",
                     "body": "Full-charge capacity is %.0f%% of design after %d cycles." % (health["health_pct"], health["cycles"] or 0)})
    return tips


# ---- summary card for the panel menu ------------------------------------------------------
SUMMARY_PATH = os.path.join(os.environ.get("XDG_CACHE_HOME", os.path.expanduser("~/.cache")), "mseries-battery", "summary.txt")
SPARK_POINTS = 48                     # 6 hours, one point per 7.5 minutes


def summary_card(conn):
    """Three lines for the Wingpanel menu card: headline, detail, and recent system watts for a sparkline.

    "5h 40m left" / "Charging · 1h 24m to full" / "7h 12m typical runtime", then "78% · 7.8 W", then a CSV of watts.
    Empty headline when there is no data yet.
    """
    now = int(time.time())
    rows = fetch(conn, now - 14 * 86400)
    if not rows:
        return "", "", ""
    last = rows[-1]
    est = estimate_runtime(rows)
    if est:
        headline = "%s left" % fmt_hours(est["hours_left"])
        basis = est["basis"]
    elif last["status"] == "Charging" and last["batt_w"] and last["batt_w"] > 0.5 and last["energy_uwh"] and last["full_uwh"]:
        to_full = (last["full_uwh"] - last["energy_uwh"]) / 1e6 / last["batt_w"]
        headline = "Charging · %s to full" % fmt_hours(to_full)
    else:
        long_runs = [r for r in sessions(rows) if r["seconds"] >= 1800 and r["projected_hours"]]
        headline = ("%s typical runtime" % fmt_hours(sum(r["projected_hours"] for r in long_runs) / len(long_runs))
                    if long_runs else "Plugged in")
    detail = ["%.0f%%" % last["pct"]] if last["pct"] is not None else []
    if est:
        spread = "%s–%s, " % (fmt_hours(est["left_low"]), fmt_hours(est["left_high"])) if est["left_low"] != est["left_high"] else ""
        detail.append(spread + est["basis"])
    if last["sys_w"] is not None:
        detail.append("%.1f W" % last["sys_w"])
    recent = [r for r in rows if r["ts"] >= now - 6 * 3600 and r["sys_w"] is not None]
    points = []
    if recent:
        step = max(1, len(recent) // SPARK_POINTS)
        for i in range(0, len(recent), step):
            chunk = recent[i:i + step]
            points.append("%.1f" % (sum(r["sys_w"] for r in chunk) / len(chunk)))
    return headline, " · ".join(detail), ",".join(points)


def summary_extra(conn):
    """Tab-separated rows for the panel dropdown, after the three card lines: stat, run, app and tip rows."""
    now = int(time.time())
    rows = fetch(conn, now - 14 * 86400)
    out = []
    health = battery_health()
    runs = sessions(rows)
    long_runs = [r for r in runs if r["seconds"] >= 1800 and r["projected_hours"]]
    if long_runs:
        out.append("stat\tTypical runtime\t%s\tfull charge, %d runs" % (
            fmt_hours(sum(r["projected_hours"] for r in long_runs) / len(long_runs)), len(long_runs)))
    if health.get("health_pct"):
        out.append("stat\tHealth\t%s%%\t%s · %d cycles" % (health["health_pct"], condition()["label"], health["cycles"] or 0))
    for run in list(reversed([r for r in runs if r["seconds"] >= 600]))[:3]:
        out.append("run\t%s\t%s\t%.0f%% → %.0f%%\t%s" % (
            time.strftime("%a %H:%M", time.localtime(run["start"])), fmt_hours(run["seconds"] / 3600),
            run["pct_start"] or 0, run["pct_end"] or 0, "%.1f W" % run["avg_w"] if run["avg_w"] else "—"))
    for item in top_culprits(rows, 3):
        out.append("app\t%s\t%.1f%% of CPU" % (item["name"], item["avg_cpu"]))
    for dev in device_summary(conn):
        shown = "%.0f%%" % dev["pct"] if dev["pct"] is not None else (dev["level"] or dev["status"] or "—")
        shown = shown + (" · ~%.0f days" % dev["days_left"] if dev["days_left"] else "")
        out.append("device\t%s\t%s\t" % (dev["name"].replace("Advanced Ergonomic ", ""), shown))
    for item in profile_breakdown(rows):
        out.append("profile\t%s\t%.1f W avg\t%s" % (item["label"], item["avg_w"], "idle %.1f W" % item["idle_w"] if item["idle_w"] else "%s on battery" % fmt_hours(item["hours"])))
    for ev in [e for e in events(conn, now - 6 * 3600, limit=50) if e["kind"] in ("rise", "drop")][:3]:
        cause = ev["detail"].replace("Other (GPU, display, radios, charging)", "Display, GPU or radios")
        out.append("event\t%s\t%+.1f W\t%s" % (time.strftime("%H:%M", time.localtime(ev["ts"])), ev["delta_w"], cause))
    drain = standby_drain(conn, now - 14 * 86400)
    if drain["pct_per_hour"] is not None:
        out.append("stat\tSleep drain\t%.2f%% / h\tasleep %.0f h on battery" % (drain["pct_per_hour"], drain["hours"]))
    try:
        tips = suggestions(conn, rows)
    except Exception:
        tips = []
    if tips:
        out.append("tip\t%s\t%s" % (tips[0]["title"], tips[0]["body"]))
    return out


def write_summary(conn, path=SUMMARY_PATH):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as handle:
        handle.write("\n".join(list(summary_card(conn)) + summary_extra(conn)))
    os.replace(tmp, path)


# ---- real history UPower already kept ------------------------------------------------------
UPOWER_DIR = "/var/lib/upower"
UPOWER_STATES = {"discharging": ("Discharging", 0), "charging": ("Charging", 1),
                 "fully-charged": ("Full", 1), "pending-charge": ("Not charging", 1), "empty": ("Discharging", 0)}


def _upower_series(path):
    series = []
    try:
        with open(path) as handle:
            for line in handle:
                parts = line.split()
                if len(parts) == 3:
                    series.append((int(parts[0]), float(parts[1]), parts[2]))
    except OSError:
        pass
    return series


def import_upower(conn, directory=UPOWER_DIR):
    """Backfill samples from UPower's own battery history (charge % and watts, since it was installed).

    These rows have no CPU, brightness or process data (so they feed sessions, runtime and health, not the
    by-usage or by-app views) and never overwrite or overlap rows this logger recorded itself.
    Re-running replaces the earlier import. Returns the number of rows stored.
    """
    bat = find_battery()
    full = rd_num(bat + "/energy_full") if bat else None
    serial = rd(bat + "/serial_number") if bat else None
    if not full or not serial:
        return 0
    charge = rate = None
    for node in glob.glob(os.path.join(directory, "history-charge-*.dat")):
        if serial in node:
            charge, rate = _upower_series(node), _upower_series(node.replace("history-charge-", "history-rate-"))
    if not charge:
        return 0
    own = conn.execute("SELECT MIN(ts) FROM samples WHERE cpu_pct IS NOT NULL").fetchone()[0]
    cutoff = own if own is not None else int(time.time()) + 1
    conn.execute("DELETE FROM samples WHERE cpu_pct IS NULL AND ts < ?", (cutoff,))
    watts, index, count = 0.0, 0, 0
    for ts, pct, state in charge:
        if ts >= cutoff or state not in UPOWER_STATES or pct <= 0:
            continue
        while index < len(rate) and rate[index][0] <= ts:
            watts = rate[index][1]
            index += 1
        status, ac = UPOWER_STATES[state]
        flow = -watts if status == "Discharging" else (watts if status == "Charging" else 0.0)
        conn.execute("INSERT OR IGNORE INTO samples (ts, status, ac, pct, energy_uwh, full_uwh, batt_w, sys_w, top) VALUES (?,?,?,?,?,?,?,?,?)",
                     (ts, status, ac, pct, int(full * pct / 100), int(full), flow,
                      watts if status == "Discharging" and watts > 0 else None, "[]"))
        count += 1
    conn.commit()
    return count


# ---- condition, daily usage and diagnostics ------------------------------------------------
def condition():
    """Apple-style battery condition from the gauge's own health flag and capacity versus design."""
    bat = find_battery()
    if not bat:
        return {"label": "Unknown", "level": "unknown", "why": "No battery found"}
    gauge = (rd(bat + "/health") or "").strip()
    health = battery_health().get("health_pct")
    if gauge and gauge.lower() not in ("good", "unknown"):
        return {"label": "Service Recommended", "level": "bad", "why": "The battery gauge reports: %s" % gauge}
    if health is not None and health < 80:
        return {"label": "Service Recommended", "level": "bad", "why": "Capacity is %.0f%% of design (below 80%%)" % health}
    if health is not None and health < 90:
        return {"label": "Normal", "level": "ok", "why": "Capacity %.0f%% of design; gauge reports %s" % (health, gauge or "no fault")}
    return {"label": "Normal", "level": "good", "why": "Capacity %s%% of design; gauge reports %s" % (health, gauge or "no fault")}


def daily_usage(rows, days=10):
    """Per local day: Wh drawn on battery, hours on battery, hours on the charger, lowest charge reached."""
    out = {}
    for i, r in enumerate(rows):
        day = time.strftime("%Y-%m-%d", time.localtime(r["ts"]))
        entry = out.setdefault(day, {"day": day, "wh": 0.0, "battery_h": 0.0, "charger_h": 0.0, "min_pct": None})
        nxt = rows[i + 1]["ts"] if i + 1 < len(rows) else r["ts"] + SAMPLE_SECONDS
        dt = min(nxt - r["ts"], GAP_SECONDS)
        if r["ac"]:
            entry["charger_h"] += dt / 3600
        else:
            entry["battery_h"] += dt / 3600
            if r["sys_w"]:
                entry["wh"] += r["sys_w"] * dt / 3600
        if r["pct"] is not None and (entry["min_pct"] is None or r["pct"] < entry["min_pct"]):
            entry["min_pct"] = r["pct"]
    return [out[d] for d in sorted(out)][-days:]


def diagnose(conn):
    """Health checks on the battery stack: each (name, ok|warn|fail, detail)."""
    checks = []
    now = int(time.time())

    def add(name, status, detail):
        checks.append({"name": name, "status": status, "detail": detail})

    bat = find_battery()
    add("Battery sensor", "ok" if bat else "fail", os.path.basename(bat) if bat else "no laptop battery under /sys/class/power_supply")
    add("System power sensor", "ok" if find_system_power() else "warn",
        "SMC Total System Power" if find_system_power() else "missing; the logger falls back to battery flow while unplugged")
    last = conn.execute("SELECT MAX(ts) FROM samples WHERE cpu_pct IS NOT NULL").fetchone()[0]
    if last is None:
        add("Logger", "fail", "no samples recorded; is mseries-battery-logger.service running?")
    else:
        age = now - last
        add("Logger", "ok" if age <= GAP_SECONDS * 2 else "warn", "last live sample %d s ago" % age)
    total = conn.execute("SELECT COUNT(*) FROM samples").fetchone()[0]
    first = conn.execute("SELECT MIN(ts) FROM samples").fetchone()[0]
    add("History", "ok" if total else "warn", "%d samples since %s" % (total, time.strftime("%Y-%m-%d %H:%M", time.localtime(first))) if first else "empty")
    imported = conn.execute("SELECT COUNT(*) FROM samples WHERE cpu_pct IS NULL").fetchone()[0]
    add("UPower backfill", "ok" if imported else "warn", "%d imported samples" % imported if imported else "none imported (history unavailable?)")
    span = conn.execute("SELECT COUNT(*) FROM (SELECT 1 FROM samples WHERE cpu_pct IS NOT NULL AND ts > ? AND ac = 0 LIMIT 240)", (now - 14 * 86400,)).fetchone()[0]
    add("Live data while unplugged", "ok" if span >= 240 else "warn",
        "%d samples" % span if span >= 240 else "only %d live samples on battery; usage-level and per-app views need about an hour" % span)
    try:
        size = os.path.getsize(DB_PATH)
        add("Database", "ok" if size < 200e6 else "warn", "%.1f MB, kept %d days" % (size / 1e6, RETENTION_DAYS))
    except OSError:
        add("Database", "warn", "size unknown")
    cond = condition()
    add("Battery condition", "ok" if cond["level"] in ("good", "ok") else "fail", "%s: %s" % (cond["label"], cond["why"]))
    health = battery_health()
    if health.get("temp_c") is not None:
        add("Temperature", "ok" if health["temp_c"] < 40 else "warn", "%.0f °C" % health["temp_c"])
    if health.get("limit_pct") is not None:
        add("Charge limit", "ok", "charging stops at %d%%" % health["limit_pct"])
    tool = shutil.which("notify-send")
    add("Notifications", "ok" if tool else "warn", "notify-send available" if tool else "notify-send missing; high-drain alerts are silent")
    return checks


def diagnostics_markdown(conn):
    lines = ["# Battery diagnostics", "", "Generated %s." % time.strftime("%Y-%m-%d %H:%M"), "",
             "| Check | Result | Detail |", "|---|---|---|"]
    for c in diagnose(conn):
        lines.append("| %s | %s | %s |" % (c["name"], c["status"].upper(), c["detail"]))
    lines += ["", report_markdown(conn, 14)]
    return "\n".join(lines)
