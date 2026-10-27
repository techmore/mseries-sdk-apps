#!/usr/bin/python3 -I
"""Offline tests for battery_core: sessions, usage bands, power events, suspend gaps, runtime estimates."""
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import battery_core as core  # noqa: E402

FULL = 60_000_000  # 60 Wh in µWh


def row(ts, pct, watts, cpu=10.0, ac=0, bright=50.0, top=None, profile="balanced"):
    return {"ts": ts, "status": "Charging" if ac else "Discharging", "ac": ac, "pct": pct,
            "energy_uwh": int(FULL * pct / 100), "full_uwh": FULL, "batt_w": -watts, "sys_w": watts,
            "cpu_pct": cpu, "cpu_mhz": 1500.0, "load1": 1.0, "temp_c": 30.0, "bright_pct": bright,
            "top": json.dumps(top or [["firefox", cpu / 2]]), "profile": profile}


class CoreTests(unittest.TestCase):
    def test_session_energy_and_projection(self):
        rows = [row(1000 + i * 15, 100 - i * 0.0125, 10.0) for i in range(241)]  # one hour, 3 %
        found = core.sessions(rows)
        self.assertEqual(len(found), 1)
        run = found[0]
        self.assertEqual(run["seconds"], 3600)
        self.assertAlmostEqual(run["wh_used"], 1.8, places=1)  # 3 % of 60 Wh
        self.assertAlmostEqual(run["avg_w"], 1.8, places=1)
        self.assertAlmostEqual(run["projected_hours"], 60 / run["avg_w"], places=0)

    def test_sessions_split_on_gap_and_on_charger(self):
        rows = [row(0, 90, 8), row(15, 89.9, 8), row(5000, 80, 8), row(5015, 79.9, 8),
                row(5030, 79.9, 8, ac=1), row(5045, 80, 8, ac=1)]
        self.assertEqual(len(core.sessions(rows)), 2)

    def test_usage_profile_bands_by_draw(self):
        rows = [row(i * 15, 90, 5.0, cpu=10.0) for i in range(240)] + [row(10000 + i * 15, 70, 20.0, cpu=10.0) for i in range(240)]
        bands = {b["band"]: b for b in core.usage_profile(rows)}
        self.assertAlmostEqual(bands["Light"]["avg_w"], 5.0)
        self.assertAlmostEqual(bands["Heavy"]["runtime_h"], 3.0)
        self.assertEqual(bands["Moderate"]["samples"], 0)

    def test_power_step_event_names_cause(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        det = core.EventDetector()
        for i in range(5):
            det.feed(conn, row(i * 15, 90, 5.0, cpu=5))
        det.feed(conn, row(75, 90, 17.0, cpu=70, top=[["ffmpeg", 60.0]]))
        found = core.events(conn, 0)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0]["kind"], "rise")
        self.assertEqual(found[0]["culprit"], "ffmpeg")
        self.assertIn("CPU load", found[0]["detail"])
        for i in range(5):  # settle, then drop
            det.feed(conn, row(200 + i * 15, 90, 17.0, cpu=70))
        det.feed(conn, row(300, 90, 5.0, cpu=5))
        self.assertEqual(core.events(conn, 0)[0]["kind"], "drop")

    def test_suspend_gap_recorded(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        det = core.EventDetector()
        det.feed(conn, row(0, 90, 5.0))
        det.feed(conn, row(8 * 3600, 88, 5.0))
        gap = core.events(conn, 0)[0]
        self.assertEqual(gap["kind"], "gap")
        self.assertEqual(json.loads(gap["detail"])["seconds"], 8 * 3600)

    def test_runtime_estimate_only_on_battery(self):
        rows = [row(i * 15, 50, 10.0) for i in range(20)]
        est = core.estimate_runtime(rows)
        self.assertAlmostEqual(est["hours_left"], 3.0)
        self.assertAlmostEqual(est["hours_full"], 6.0)
        self.assertEqual(core.estimate_runtime(rows + [row(400, 50, 10.0, ac=1)]), {})

    def test_sampler_reads_this_machine(self):
        sampler = core.Sampler()
        sampler.sample()
        snap = sampler.sample()
        self.assertEqual(set(snap), set(core.COLUMNS))

    def test_energy_by_app_splits_dynamic_draw(self):
        rows = [row(i * 15, 90, 5.0, cpu=2, top=[["idle-app", 2.0]]) for i in range(40)]
        rows += [row(1000 + i * 15, 80, 15.0, cpu=50, top=[["ffmpeg", 40.0], ["brave", 10.0]]) for i in range(40)]
        data = core.energy_by_app(rows)
        names = {a["name"]: a for a in data["apps"]}
        self.assertAlmostEqual(data["floor_w"], 5.0)
        self.assertGreater(names["ffmpeg"]["wh"], 3 * names["brave"]["wh"])
        self.assertAlmostEqual(sum(a["wh"] for a in data["apps"]) + data["floor_wh"] + data["other_wh"], data["total_wh"], delta=0.2)
        self.assertEqual(core.energy_by_app(rows[:2])["apps"], [])

    def test_charge_sessions(self):
        rows = []
        for i in range(60):
            r = row(i * 15, 50 + i * 0.1, 30.0, ac=1)
            r["status"], r["batt_w"] = "Charging", 30.0
            rows.append(r)
        found = core.charge_sessions(rows)
        self.assertEqual(len(found), 1)
        self.assertGreater(found[0]["wh_added"], 0.3)

    def test_standby_drain(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        det = core.EventDetector()
        det.feed(conn, row(0, 90, 5.0))
        det.feed(conn, row(8 * 3600, 88, 5.0))   # 2 % lost over 8 h unplugged
        drain = core.standby_drain(conn, 0)
        self.assertAlmostEqual(drain["pct_per_hour"], 0.25)

    def test_drain_alert_needs_sustained_draw_and_respects_cooldown(self):
        alert = core.DrainAlert(dict(core.DEFAULT_CONFIG))
        fired = [alert.feed(row(i * 15, 70, 20.0, top=[["ffmpeg", 80.0]])) for i in range(40)]
        hits = [f for f in fired if f]
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0]["culprit"], "ffmpeg")
        self.assertIsNone(core.DrainAlert(dict(core.DEFAULT_CONFIG)).feed(row(0, 70, 20.0)))
        quiet = core.DrainAlert(dict(core.DEFAULT_CONFIG))
        self.assertFalse(any(quiet.feed(row(i * 15, 70, 6.0)) for i in range(40)))
        plugged = core.DrainAlert(dict(core.DEFAULT_CONFIG))
        self.assertFalse(any(plugged.feed(row(i * 15, 70, 20.0, ac=1)) for i in range(40)))

    def test_report_and_health_trend(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        now = int(__import__("time").time())
        for i in range(240):
            core.store(conn, row(now - 3600 + i * 15, 100 - i * 0.0125, 10.0))
        conn.execute("INSERT INTO daily VALUES ('2026-10-17', 62000000, 69000000, 95, 80)")
        self.assertEqual(len(core.health_trend(conn)), 1)
        text = core.report_markdown(conn, 1)
        self.assertIn("# Battery report", text)
        self.assertIn("Unplugged runs", text)

    def test_profile_breakdown_compares_at_same_load(self):
        rows = [row(i * 15, 90, 6.0, cpu=3, profile="balanced") for i in range(60)]
        rows += [row(2000 + i * 15, 80, 4.5, cpu=3, profile="power-saver") for i in range(60)]
        found = {p["profile"]: p for p in core.profile_breakdown(rows)}
        self.assertAlmostEqual(found["balanced"]["idle_w"], 6.0)
        self.assertAlmostEqual(found["power-saver"]["idle_w"], 4.5)

    def test_suggestions_are_backed_by_data(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        self.assertEqual(core.suggestions(conn, [row(i, 90, 5.0) for i in range(10)]), [])  # too little data
        rows = [row(i * 15, 90, 7.5, cpu=10, bright=90, top=[["brave", 12.0]]) for i in range(150)]
        rows += [row(5000 + i * 15, 85, 5.5, cpu=10, bright=40, top=[["brave", 12.0]]) for i in range(150)]
        kinds = {t["kind"] for t in core.suggestions(conn, rows)}
        self.assertIn("display", kinds)
        self.assertIn("app", kinds)
        self.assertIn("profile", kinds)

    def test_config_roundtrip(self):
        core.CONFIG_PATH = os.path.join(tempfile.mkdtemp(), "c.json")
        config = dict(core.DEFAULT_CONFIG, alert_watts=22.0)
        core.save_config(config)
        self.assertEqual(core.load_config()["alert_watts"], 22.0)

    def test_devices_logged_on_change_and_summarised(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        for ts, level, pct in ((0, "Normal", None), (600, "Normal", None), (900, "Low", None)):
            conn.execute("INSERT INTO devices VALUES (?,?,?,?,?)", (ts, "Mouse", level, pct, "Discharging"))
        for i in range(5):  # a percentage device falling 10 % a day
            conn.execute("INSERT INTO devices VALUES (?,?,?,?,?)", (i * 86400, "Keyboard", "Normal", 90 - i * 10.0, "Discharging"))
        found = {d["name"]: d for d in core.device_summary(conn)}
        self.assertEqual(found["Mouse"]["level"], "Low")
        self.assertEqual(found["Mouse"]["since"], 900)
        self.assertAlmostEqual(found["Keyboard"]["days_left"], 5.0)

    def test_read_devices_ignores_laptop_battery(self):
        names = [d["name"] for d in core.read_devices()]
        self.assertNotIn("bq40z651", names)

    def test_summary_card(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        self.assertEqual(core.summary_card(conn), ("", "", ""))
        now = int(__import__("time").time())
        for i in range(20):
            core.store(conn, row(now - 300 + i * 15, 50, 10.0))
        headline, detail, points = core.summary_card(conn)
        self.assertEqual(headline, "3h 00m left")
        self.assertEqual(detail, "50% · recent use · 10.0 W")
        self.assertTrue(points.startswith("10.0"))
        path = os.path.join(tempfile.mkdtemp(), "x", "summary.txt")
        core.write_summary(conn, path)
        with open(path) as handle:
            self.assertGreaterEqual(len(handle.read().split("\n")), 3)

    def test_discharging_sample_from_fake_sysfs(self):
        root = tempfile.mkdtemp()

        def put(path, value):
            full = os.path.join(root, path)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w") as handle:
                handle.write(str(value))

        bat = "sys/class/power_supply/BAT0/"
        for name, value in (("type", "Battery"), ("status", "Discharging"), ("energy_now", 30000000), ("energy_full", 60000000),
                            ("power_now", 8000000), ("capacity", 50), ("temp", 310)):
            put(bat + name, value)
        put("sys/class/power_supply/AC/type", "Mains")
        put("sys/class/power_supply/AC/online", 0)
        put("sys/class/hwmon/hwmon2/power1_label", "Total System Power")
        put("sys/class/hwmon/hwmon2/power1_input", 9500000)
        put("sys/class/power_supply/MOUSE/type", "Battery")
        put("sys/class/power_supply/MOUSE/scope", "Device")
        put("sys/class/power_supply/MOUSE/model_name", "Test Mouse")
        put("sys/class/power_supply/MOUSE/capacity_level", "Low")
        proc = os.path.join(root, "proc")
        put("proc/stat", "cpu 100 0 100 800 0 0 0 0")
        put("proc/loadavg", "1.50 1.0 1.0 1/100 1")
        old = core.SYS
        core.SYS = root
        try:
            sampler = core.Sampler(proc=proc)
            sampler.sample()
            put("proc/stat", "cpu 200 0 200 1000 0 0 0 0")   # 200 busy of 400 ticks
            snap = sampler.sample()
            self.assertEqual(snap["status"], "Discharging")
            self.assertEqual(snap["ac"], 0)
            self.assertAlmostEqual(snap["pct"], 50.0)
            self.assertAlmostEqual(snap["batt_w"], -8.0)
            self.assertAlmostEqual(snap["sys_w"], 9.5)
            self.assertAlmostEqual(snap["cpu_pct"], 50.0)
            self.assertAlmostEqual(snap["temp_c"], 31.0)
            self.assertEqual([d["name"] for d in core.read_devices()], ["Test Mouse"])
        finally:
            core.SYS = old

    def test_daily_usage_groups_by_day(self):
        base = int(__import__("time").mktime((2026, 10, 10, 9, 0, 0, 0, 0, -1)))
        rows = [row(base + i * 15, 90, 12.0) for i in range(240)]                 # one hour on battery
        rows += [row(base + 3600 + i * 15, 80, 5.0, ac=1) for i in range(120)]    # half an hour plugged in
        day = core.daily_usage(rows)[0]
        self.assertAlmostEqual(day["wh"], 12.0, delta=0.3)
        self.assertAlmostEqual(day["battery_h"], 1.0, delta=0.02)
        self.assertAlmostEqual(day["charger_h"], 0.5, delta=0.02)

    def test_diagnose_flags_empty_history(self):
        conn = core.connect(os.path.join(tempfile.mkdtemp(), "h.db"))
        found = {c["name"]: c for c in core.diagnose(conn)}
        self.assertEqual(found["Logger"]["status"], "fail")
        self.assertIn("# Battery diagnostics", core.diagnostics_markdown(conn))

    def test_app_key_groups_by_cgroup(self):
        base = "0::/user.slice/user@1000.service/app.slice/"
        self.assertEqual(core.app_key(base + "app-org.chromium.Chromium-127328.scope", "brave"), "org.chromium.Chromium")
        self.assertEqual(core.app_key(base + "app-gnome-mseries\\x2dlauncher-1.scope", "brave"), "brave")
        self.assertEqual(core.app_key("0::/user.slice/user@1000.service/session.slice/pipewire.service", "pipewire"), "pipewire")
        self.assertEqual(core.app_key("0::/user.slice/user@1000.service/session.slice/_io.elementary.gala@wayland.service", "io.elementary.wingpanel"), "io.elementary.wingpanel")
        self.assertEqual(core.app_key("0::/system.slice/ssh.service", "sshd"), "ssh")
        self.assertEqual(core.app_key("", "bash"), "bash")

    def test_estimate_blends_recent_and_hourly_and_falls_back(self):
        rows = [row(i * 15, 60, 8.0) for i in range(200)]                 # steady 8 W, then a heavy last 10 minutes
        rows += [row(3000 + i * 15, 50, 20.0) for i in range(40)]
        est = core.estimate_runtime(rows)
        self.assertEqual(est["basis"], "recent use")
        self.assertLess(est["left_low"], est["hours_left"])
        self.assertGreater(est["left_high"], est["hours_left"])
        fresh = [row(i * 15, 60, 8.0, ac=1) for i in range(100)] + [row(2000 + i * 15, 59, 9.0) for i in range(3)]
        self.assertEqual(core.estimate_runtime(fresh), {})                # no history of runs, no recent data: say nothing
        history = [row(i * 15, 90 - i * 0.01, 10.0) for i in range(240)] + [row(4000, 80, 5.0, ac=1)] + [row(20000 + i * 15, 70, 9.0) for i in range(3)]
        est = core.estimate_runtime(history)
        self.assertEqual(est["basis"], "your typical use")

    def test_fmt_hours(self):
        self.assertEqual(core.fmt_hours(7.5), "7h 30m")
        self.assertEqual(core.fmt_hours(0.5), "30 min")
        self.assertEqual(core.fmt_hours(None), "—")


if __name__ == "__main__":
    unittest.main()
