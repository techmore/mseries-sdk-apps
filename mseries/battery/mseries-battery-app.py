#!/usr/bin/python3
"""Battery: elementary-style window over the history that mseries-battery-logger keeps.

Overview (health, runtime by usage), History (power and charge chart with event markers),
Sessions (each unplugged run) and Events (power steps and suspend gaps with the likely cause).
Read-only: it never changes charge limits or any other setting.
"""
import json
import math
import os
import sys
import time
import warnings

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
warnings.filterwarnings("ignore", category=DeprecationWarning)
import cairo  # noqa: E402
import gi  # noqa: E402

gi.require_version("Gtk", "4.0")
gi.require_version("Granite", "7.0")
from gi.repository import Gio, GLib, Granite, Gtk  # noqa: E402

import battery_core as core  # noqa: E402

# elementary palette
BLUEBERRY, LIME, ORANGE, STRAWBERRY, GRAPE = (0.21, 0.54, 0.90), (0.41, 0.72, 0.14), (0.95, 0.45, 0.16), (0.78, 0.15, 0.18), (0.65, 0.44, 0.78)
RANGES = [("1 h", 3600), ("6 h", 6 * 3600), ("24 h", 86400), ("7 d", 7 * 86400)]
CSS = b"""
.page { padding: 24px; }
.hero-percent { font-size: 3.4em; font-weight: 800; }
.stat-value { font-size: 1.7em; font-weight: 700; }
.section-title { font-weight: 700; font-size: 0.85em; opacity: 0.65; letter-spacing: 0.04em; }
.row-title { font-weight: 600; }
.legend { font-size: 0.9em; }
.tile { padding: 14px 16px; border-radius: 9px; background: alpha(@fg_color, 0.05); box-shadow: inset 0 0 0 1px alpha(@fg_color, 0.08); }
.good-level trough block.filled { background: #68b723; }
.low-level trough block.filled { background: #f37329; }
.crit-level trough block.filled { background: #c6262e; }
.usage-bar trough block.filled { background: #3689e6; }
.rise-icon { color: #f37329; }
.drop-icon { color: #a56de2; }
.sleep-icon { color: alpha(@fg_color, 0.6); }
"""


STATE_PATH = os.path.join(os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")), "mseries-battery", "ui.json")


def load_ui():
    try:
        with open(STATE_PATH) as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return {}


def save_ui(state):
    try:
        os.makedirs(os.path.dirname(STATE_PATH), exist_ok=True)
        with open(STATE_PATH, "w") as handle:
            json.dump(state, handle)
    except OSError:
        pass


def when(ts, with_date=True):
    return time.strftime("%a %H:%M" if with_date else "%H:%M", time.localtime(ts))


def label(text, *classes, xalign=0.0, wrap=False, hexpand=False):
    widget = Gtk.Label(label=text, xalign=xalign, wrap=wrap, hexpand=hexpand)
    for name in classes:
        widget.add_css_class(name)
    return widget


def section(title, child=None):
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    box.append(label(title.upper(), "section-title"))
    if child is not None:
        box.append(child)
    return box


def tile(title, value, sub=""):
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3, hexpand=True)
    box.add_css_class("tile")
    box.append(label(title.upper(), "section-title"))
    box.append(label(value, "stat-value"))
    if sub:
        box.append(label(sub, "dim", wrap=True))
    return box


def level_bar(fraction, *classes):
    bar = Gtk.LevelBar(min_value=0, max_value=1, value=max(0.0, min(1.0, fraction)), hexpand=True)
    bar.set_size_request(-1, 8)
    for name in classes:
        bar.add_css_class(name)
    return bar


# charge level -> soft colour: lime while healthy, easing through amber to a muted strawberry
LEVEL_STOPS = [(100, (0.45, 0.74, 0.30)), (60, (0.45, 0.74, 0.30)), (35, (0.90, 0.72, 0.24)),
               (18, (0.93, 0.52, 0.22)), (8, (0.82, 0.30, 0.30)), (0, (0.82, 0.30, 0.30))]


def level_colour(pct):
    pct = max(0.0, min(100.0, pct or 0))
    for (hi, hi_rgb), (lo, lo_rgb) in zip(LEVEL_STOPS, LEVEL_STOPS[1:]):
        if pct >= lo:
            t = 1.0 if hi == lo else (pct - lo) / (hi - lo)
            return tuple(l + (h - l) * t for l, h in zip(lo_rgb, hi_rgb))
    return LEVEL_STOPS[-1][1]


class BatteryGauge(Gtk.DrawingArea):
    """A battery outline whose fill drops with the charge and shifts colour softly as it gets low."""

    def __init__(self, pct, charging, width=40, height=64, number=True):
        super().__init__(content_width=width, content_height=height, valign=Gtk.Align.CENTER, halign=Gtk.Align.CENTER)
        self.pct, self.charging, self.number = max(0.0, min(100.0, pct or 0)), charging, number
        self.set_draw_func(self.draw)

    @staticmethod
    def round_rect(cr, x, y, w, h, r):
        cr.new_sub_path()
        cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
        cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
        cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
        cr.arc(x + r, y + r, r, math.pi, 3 * math.pi / 2)
        cr.close_path()

    def draw(self, _area, cr, width, height):
        fg = self.get_color()
        cap_h, line = height * 0.09, 2.0
        body_x, body_y, body_w = line / 2, cap_h + line / 2, width - line
        body_h = height - cap_h - line
        cr.set_line_width(line)
        self.round_rect(cr, width * 0.32, 0, width * 0.36, cap_h + 1, 1.5)  # terminal
        cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.55)
        cr.fill()
        self.round_rect(cr, body_x, body_y, body_w, body_h, 6)
        cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.07)
        cr.fill_preserve()
        cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.55)
        cr.stroke()
        inset = line + 2.0
        inner_x, inner_y, inner_w, inner_h = body_x + inset, body_y + inset, body_w - 2 * inset, body_h - 2 * inset
        fill_h = inner_h * self.pct / 100.0
        if fill_h >= 1:
            red, green, blue = level_colour(self.pct)
            cr.save()
            self.round_rect(cr, inner_x, inner_y, inner_w, inner_h, 3.5)
            cr.clip()
            top = inner_y + inner_h - fill_h
            gradient = cairo.LinearGradient(0, top, 0, inner_y + inner_h)
            gradient.add_color_stop_rgba(0, min(1, red + 0.08), min(1, green + 0.08), min(1, blue + 0.08), 0.92)
            gradient.add_color_stop_rgba(1, red, green, blue, 0.92)
            cr.set_source(gradient)
            cr.rectangle(inner_x, top, inner_w, fill_h)
            cr.fill()
            cr.restore()
        text_y = body_y + body_h / 2
        if self.number:
            text_y = body_y + body_h * (0.68 if self.charging else 0.5)
        if self.charging:
            cx, cy, s = width / 2, body_y + body_h * (0.32 if self.number else 0.5), width * (0.12 if self.number else 0.17)
            cr.move_to(cx + s * 0.35, cy - s * 1.9)
            for dx, dy in ((-s * 0.95, s * 1.15), (s * 0.55, s * 0.0), (-s * 0.35, s * 1.9), (s * 0.95, -s * 1.15), (-s * 0.55, 0.0)):
                cr.rel_line_to(dx, dy)
            cr.close_path()
            cr.set_source_rgba(1, 1, 1, 0.92)
            cr.fill_preserve()
            cr.set_source_rgba(0, 0, 0, 0.35)
            cr.set_line_width(1)
            cr.stroke()
        if self.number:
            text = "%d" % round(self.pct)
            cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
            cr.set_font_size(inner_w * (0.62 if len(text) < 3 else 0.46))
            extents = cr.text_extents(text)
            fill_top = inner_y + inner_h - fill_h
            top, bottom = text_y - extents.height / 2, text_y + extents.height / 2
            cr.move_to(width / 2 - extents.x_bearing - extents.width / 2, text_y - extents.y_bearing - extents.height / 2)
            cr.text_path(text)
            if fill_top > bottom:  # digits sit on the empty part of the battery
                cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.9)
                cr.fill()
            else:
                if fill_top > top:  # fill edge cuts through the digits: halo keeps them legible on both sides
                    cr.set_source_rgba(1, 1, 1, 0.6)
                    cr.set_line_width(2.2)
                    cr.stroke_preserve()
                cr.set_source_rgba(0.08, 0.1, 0.12, 0.9)
                cr.fill()


def boxed_list():
    listing = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
    listing.add_css_class("boxed-list")
    listing.add_css_class("rich-list")
    return listing


def empty(title, text, icon="battery-symbolic"):
    placeholder = Granite.Placeholder(title=title, description=text)
    placeholder.set_icon(Gio.ThemedIcon.new(icon)) if hasattr(placeholder, "set_icon") else None
    placeholder.set_vexpand(True)
    return placeholder


class Chart(Gtk.DrawingArea):
    """System power (filled line, W) and charge (line, %) over a time range, with event markers."""

    def __init__(self):
        super().__init__(hexpand=True, vexpand=True, content_height=260)
        self.rows, self.events, self.start, self.end = [], [], 0, 1
        self.hover_x = None
        self.set_draw_func(self.draw)
        motion = Gtk.EventControllerMotion()
        motion.connect("motion", self.on_motion)
        motion.connect("leave", self.on_leave)
        self.add_controller(motion)

    def on_motion(self, _controller, x, _y):
        self.hover_x = x
        self.queue_draw()

    def on_leave(self, _controller):
        self.hover_x = None
        self.queue_draw()

    def set_data(self, rows, events, start, end):
        self.rows, self.events, self.start, self.end = rows, events, start, end
        self.queue_draw()

    def draw(self, _area, cr, width, height):
        fg = self.get_color()
        left, right, top, bottom = 44, 44, 12, 26
        plot_w, plot_h = width - left - right, height - top - bottom
        span = max(1, self.end - self.start)
        points = [r for r in self.rows if r["sys_w"] is not None]
        peak = max([12.0] + [r["sys_w"] for r in points])
        peak = math.ceil(peak / 10.0) * 10.0

        def x_of(ts):
            return left + plot_w * (ts - self.start) / span

        def y_power(watts):
            return top + plot_h * (1 - min(watts, peak) / peak)

        def y_pct(pct):
            return top + plot_h * (1 - pct / 100.0)

        cr.set_font_size(11)
        cr.set_line_width(1)
        for step in range(5):  # horizontal grid with both axes
            y = top + plot_h * step / 4
            cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.12)
            cr.move_to(left, y)
            cr.line_to(left + plot_w, y)
            cr.stroke()
            cr.set_source_rgba(*BLUEBERRY, 0.9)
            cr.move_to(4, y + 4)
            cr.show_text("%d W" % (peak * (1 - step / 4)))
            cr.set_source_rgba(*LIME, 0.9)
            cr.move_to(left + plot_w + 6, y + 4)
            cr.show_text("%d%%" % (100 * (1 - step / 4)))
        cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.7)  # time labels
        for step in range(5):
            ts = self.start + span * step / 4
            text = when(ts, span > 86400)
            cr.move_to(left + plot_w * step / 4 - 14, height - 6)
            cr.show_text(text)

        # shade time spent on the charger
        cr.set_source_rgba(*LIME, 0.07)
        run_start = None
        for r in self.rows + [None]:
            if r is not None and r["ac"] and run_start is None:
                run_start = r["ts"]
            elif (r is None or not r["ac"]) and run_start is not None:
                end = self.rows[-1]["ts"] if r is None else r["ts"]
                cr.rectangle(x_of(run_start), top, max(1, x_of(end) - x_of(run_start)), plot_h)
                cr.fill()
                run_start = None

        def trace(series, y_of, colour, fill):
            segment = []
            def flush():
                if len(segment) < 2:
                    segment.clear()
                    return
                cr.new_path()
                cr.move_to(x_of(segment[0][0]), y_of(segment[0][1]))
                for ts, value in segment[1:]:
                    cr.line_to(x_of(ts), y_of(value))
                cr.set_source_rgba(*colour, 1)
                cr.set_line_width(1.6)
                cr.stroke_preserve() if fill else cr.stroke()
                if fill:
                    cr.line_to(x_of(segment[-1][0]), top + plot_h)
                    cr.line_to(x_of(segment[0][0]), top + plot_h)
                    cr.close_path()
                    cr.set_source_rgba(*colour, 0.16)
                    cr.fill()
                segment.clear()
            last = None
            for ts, value in series:
                if last is not None and ts - last > core.GAP_SECONDS:
                    flush()  # asleep: do not draw a line across the gap
                segment.append((ts, value))
                last = ts
            flush()

        trace([(r["ts"], r["sys_w"]) for r in points], y_power, BLUEBERRY, True)
        trace([(r["ts"], r["pct"]) for r in self.rows if r["pct"] is not None], y_pct, LIME, False)

        for ev in self.events:  # triangles on the power line: rises in orange, drops in grape
            if ev["kind"] not in ("rise", "drop") or not self.start <= ev["ts"] <= self.end:
                continue
            x, y = x_of(ev["ts"]), y_power(ev["after_w"])
            cr.set_source_rgba(*(ORANGE if ev["kind"] == "rise" else GRAPE), 1)
            cr.move_to(x, y - 7)
            cr.line_to(x - 4, y - 1)
            cr.line_to(x + 4, y - 1)
            cr.close_path()
            cr.fill()
        if self.hover_x is not None and points and left <= self.hover_x <= left + plot_w:
            ts = self.start + span * (self.hover_x - left) / plot_w
            near = min(self.rows, key=lambda r: abs(r["ts"] - ts))
            x = x_of(near["ts"])
            cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.35)
            cr.set_line_width(1)
            cr.move_to(x, top)
            cr.line_to(x, top + plot_h)
            cr.stroke()
            lines = [time.strftime("%a %H:%M", time.localtime(near["ts"]))]
            if near["sys_w"] is not None:
                lines.append("%.1f W" % near["sys_w"])
            if near["pct"] is not None:
                lines.append("%.0f%% charge" % near["pct"])
            if near["cpu_pct"] is not None:
                lines.append("CPU %.0f%%" % near["cpu_pct"])
            top_proc = json.loads(near["top"] or "[]")
            if top_proc:
                lines.append(top_proc[0][0])
            box_w, box_h = 118, 8 + 15 * len(lines)
            bx = x + 12 if x + 12 + box_w < width - 4 else x - 12 - box_w
            by = top + 8
            cr.set_source_rgba(0.1, 0.1, 0.1, 0.92)
            cr.rectangle(bx, by, box_w, box_h)
            cr.fill()
            cr.set_source_rgba(1, 1, 1, 0.95)
            for i, text in enumerate(lines):
                cr.move_to(bx + 8, by + 17 + 15 * i)
                cr.show_text(text)
        if not points:
            cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.6)
            cr.move_to(left + 20, top + plot_h / 2)
            cr.show_text("No samples in this range yet")


class RangeBar(Gtk.Box):
    """Segment bar showing where a session started and ended on the 0-100% scale."""

    def __init__(self, start, end):
        super().__init__()
        area = Gtk.DrawingArea(content_width=96, content_height=8, valign=Gtk.Align.CENTER)
        area.set_draw_func(self.draw, (start or 0, end or 0))
        self.append(area)

    @staticmethod
    def draw(area, cr, width, height, span):
        fg = area.get_color()
        top, bottom = max(span), min(span)
        cr.set_source_rgba(fg.red, fg.green, fg.blue, 0.14)
        cr.rectangle(0, 0, width, height)
        cr.fill()
        cr.set_source_rgba(*BLUEBERRY, 1)
        cr.rectangle(width * bottom / 100.0, 0, max(2, width * (top - bottom) / 100.0), height)
        cr.fill()


class PreferencesWindow(Gtk.Window):
    """High-drain notification settings; the logger re-reads the file every sample, so changes apply at once."""

    def __init__(self, parent):
        super().__init__(transient_for=parent, modal=True, title="Preferences", default_width=420, resizable=False)
        self.config = core.load_config()
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14)
        box.add_css_class("page")
        box.append(label("HIGH POWER USE ALERT", "section-title"))
        listing = boxed_list()
        self.switch = Gtk.Switch(active=self.config["alerts"], valign=Gtk.Align.CENTER)
        self.watts = Gtk.SpinButton.new_with_range(5, 60, 1)
        self.watts.set_value(self.config["alert_watts"])
        self.minutes = Gtk.SpinButton.new_with_range(1, 60, 1)
        self.minutes.set_value(self.config["alert_minutes"])
        self.cooldown = Gtk.SpinButton.new_with_range(5, 240, 5)
        self.cooldown.set_value(self.config["alert_cooldown_minutes"])
        for title, sub, widget in (("Notify me", "Only while unplugged", self.switch),
                                   ("Average draw (W)", "At or above this", self.watts),
                                   ("For at least (min)", "Sustained, not a spike", self.minutes),
                                   ("Not again for (min)", "Quiet period after an alert", self.cooldown)):
            line = Gtk.Box(spacing=12)
            line.set_margin_top(10)
            line.set_margin_bottom(10)
            line.set_margin_start(14)
            line.set_margin_end(14)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1, hexpand=True)
            text.append(label(title, "row-title"))
            text.append(label(sub, "dim", "caption"))
            line.append(text)
            line.append(widget)
            listing.append(line)
        box.append(listing)
        done = Gtk.Button(label="Done", halign=Gtk.Align.END)
        done.add_css_class("suggested-action")
        done.connect("clicked", self.on_done)
        box.append(done)
        self.set_child(box)

    def on_done(self, _button):
        self.config.update(alerts=self.switch.get_active(), alert_watts=self.watts.get_value(),
                           alert_minutes=int(self.minutes.get_value()), alert_cooldown_minutes=int(self.cooldown.get_value()))
        core.save_config(self.config)
        self.close()


class BatteryWindow(Gtk.ApplicationWindow):
    PAGES = (("overview", "Overview", "battery-symbolic"),
             ("history", "History", "view-statistics-symbolic"),
             ("sessions", "Sessions", "document-open-recent-symbolic"),
             ("apps", "Energy by app", "applications-system-symbolic"),
             ("devices", "Accessories", "input-mouse-symbolic"),
             ("health", "Health", "emblem-default-symbolic"),
             ("diagnostics", "Diagnostics", "dialog-information-symbolic"),
             ("events", "Power events", "dialog-information-symbolic"))

    def __init__(self, app):
        ui = load_ui()
        super().__init__(application=app, title="Battery", default_width=ui.get("width", 980), default_height=ui.get("height", 680))
        self.range_seconds = ui.get("range", RANGES[1][1])
        if self.range_seconds not in [r[1] for r in RANGES]:
            self.range_seconds = RANGES[1][1]
        self.conn = None
        self.chart = Chart()
        header = Gtk.HeaderBar()
        header.add_css_class("titlebar")
        header.set_title_widget(Gtk.Label(label="Battery", css_classes=["title"]))
        export = Gtk.Button(icon_name="document-save-as-symbolic", tooltip_text="Save a Markdown report")
        export.update_property([Gtk.AccessibleProperty.LABEL], ["Save a Markdown report"])
        export.connect("clicked", self.on_export)
        header.pack_end(export)
        prefs = Gtk.Button(icon_name="preferences-system-symbolic", tooltip_text="Preferences")
        prefs.update_property([Gtk.AccessibleProperty.LABEL], ["Preferences"])
        prefs.connect("clicked", lambda _b: PreferencesWindow(self).present())
        header.pack_end(prefs)
        self.set_titlebar(header)

        self.stack = Gtk.Stack(transition_type=Gtk.StackTransitionType.CROSSFADE, hexpand=True, vexpand=True)
        self.pages = {}
        for name, title, _icon in self.PAGES:
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=20)
            box.add_css_class("page")
            self.pages[name] = box
            self.stack.add_titled(Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER, child=box), name, title)
        sidebar = Gtk.StackSidebar(stack=self.stack, vexpand=True)
        sidebar.set_size_request(190, -1)
        sidebar.add_css_class("sidebar")
        layout = Gtk.Box()
        layout.append(sidebar)
        layout.append(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL))
        layout.append(self.stack)
        self.set_child(layout)
        self.refresh()
        if ui.get("page") in self.pages:
            self.stack.set_visible_child_name(ui["page"])
        GLib.timeout_add_seconds(core.SAMPLE_SECONDS, self.tick)
        self.connect("close-request", self.on_close)
        keys = Gtk.ShortcutController()
        keys.set_scope(Gtk.ShortcutScope.GLOBAL)
        for number, (name, _t, _i) in enumerate(self.PAGES, start=1):
            keys.add_shortcut(Gtk.Shortcut.new(Gtk.ShortcutTrigger.parse_string("<Control>%d" % number),
                                               Gtk.CallbackAction.new(lambda *_a, n=name: self.stack.set_visible_child_name(n) or True)))
        keys.add_shortcut(Gtk.Shortcut.new(Gtk.ShortcutTrigger.parse_string("<Control>r"), Gtk.CallbackAction.new(lambda *_a: self.refresh() or True)))
        keys.add_shortcut(Gtk.Shortcut.new(Gtk.ShortcutTrigger.parse_string("<Control>comma"), Gtk.CallbackAction.new(lambda *_a: PreferencesWindow(self).present() or True)))
        keys.add_shortcut(Gtk.Shortcut.new(Gtk.ShortcutTrigger.parse_string("<Control>q"), Gtk.CallbackAction.new(lambda *_a: self.close() or True)))
        self.add_controller(keys)

    def tick(self):
        if self.is_active() or self.get_surface() is not None and self.get_mapped():
            self.refresh()
        return True

    def on_close(self, _window):
        save_ui({"page": self.stack.get_visible_child_name(), "range": self.range_seconds,
                 "width": self.get_width(), "height": self.get_height()})
        return False

    # ---- data -------------------------------------------------------------------------------
    def open(self):
        if self.conn is None:
            try:
                self.conn = core.connect(readonly=True)
            except Exception:
                return None
        return self.conn

    @staticmethod
    def clear(box):
        child = box.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            box.remove(child)
            child = nxt

    def refresh(self):
        for box in self.pages.values():
            if self.chart.get_parent() is box:
                box.remove(self.chart)
            self.clear(box)
        conn = self.open()
        if conn is None:
            for box in self.pages.values():
                box.append(empty("No battery history yet", "Start the logger: systemctl --user enable --now mseries-battery-logger"))
            return True
        now = int(time.time())
        rows = core.fetch(conn, now - 14 * 86400)
        self.build_overview(rows)
        self.build_history(conn, now)
        self.build_sessions(rows)
        self.build_apps(rows)
        self.build_devices(conn, now)
        self.build_health(conn, rows, now)
        self.build_diagnostics(conn)
        self.build_events(conn, now)
        return True

    def on_export(self, _button, diagnostics=False):
        dialog = Gtk.FileDialog(title="Save battery report", initial_name="battery-%s-%s.md" % ("diagnostics" if diagnostics else "report", time.strftime("%Y-%m-%d")))
        docs = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_DOCUMENTS)
        if docs:
            dialog.set_initial_folder(Gio.File.new_for_path(docs))

        def done(source, result):
            try:
                target = source.save_finish(result)
            except GLib.Error:
                return
            conn = self.open()
            if conn is not None:
                target.replace_contents((core.diagnostics_markdown(conn) if diagnostics else core.report_markdown(conn, 14)).encode(), None, False, Gio.FileCreateFlags.NONE, None)

        dialog.save(self, None, done)

    # ---- pages ------------------------------------------------------------------------------
    def build_overview(self, rows):
        page = self.pages["overview"]
        health = core.battery_health()
        last = rows[-1] if rows else None
        pct = last["pct"] if last and last["pct"] is not None else 0
        charging = bool(last and last["status"] == "Charging")

        hero = Gtk.Box(spacing=20)
        hero.append(BatteryGauge(pct, charging))
        middle = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, hexpand=True, valign=Gtk.Align.CENTER)
        top = Gtk.Box(spacing=12)
        top.append(label("%.0f%%" % pct, "hero-percent"))
        state = {"Discharging": "On battery", "Charging": "Charging", "Full": "Fully charged"}.get(
            last["status"] if last else "", "Plugged in, holding charge" if last and last["ac"] else "No data yet")
        top.append(label(state, "dim", hexpand=True, xalign=0.0))
        middle.append(top)
        middle.append(level_bar(pct / 100.0, "crit-level" if pct <= 10 else "low-level" if pct <= 20 else "good-level"))
        hero.append(middle)
        est = core.estimate_runtime(rows)
        right = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, valign=Gtk.Align.CENTER, halign=Gtk.Align.END)
        if est:
            right.append(label(core.fmt_hours(est["hours_left"]), "stat-value", xalign=1.0))
            right.append(label("remaining at %.1f W" % est["avg_w"], "dim", xalign=1.0))
        elif last and last["sys_w"] is not None:
            right.append(label("%.1f W" % last["sys_w"], "stat-value", xalign=1.0))
            right.append(label("system draw", "dim", xalign=1.0))
        hero.append(right)
        page.append(hero)

        runs = core.sessions(rows)
        recent = [r for r in runs if r["avg_w"] and r["seconds"] >= 1800 and r["projected_hours"]]
        typical = sum(r["projected_hours"] for r in recent) / len(recent) if recent else None
        total_h = sum(r["seconds"] for r in runs) / 3600
        tiles = Gtk.Box(spacing=12, homogeneous=True)
        tiles.append(tile("Typical runtime", core.fmt_hours(typical),
                          "full charge, %d recent runs" % len(recent) if recent else "needs a 30 min run on battery"))
        tiles.append(tile("Health", "%s%%" % health.get("health_pct", "—"),
                          "%.1f of %.1f Wh · %d cycles" % (health.get("full_wh") or 0, health.get("design_wh") or 0, health.get("cycles") or 0)))
        tiles.append(tile("On battery", core.fmt_hours(total_h), "%d runs in 14 days" % len(runs)))
        drain = core.standby_drain(self.open(), int(time.time()) - 14 * 86400) if self.open() else {}
        tiles.append(tile("Sleep drain", "%.2f%% / h" % drain["pct_per_hour"] if drain.get("pct_per_hour") is not None else "—",
                          "asleep %.0f h on battery" % drain["hours"] if drain.get("hours") else "needs a night asleep on battery"))
        page.append(tiles)
        if health.get("limit_pct") is not None:
            page.append(label("Charging stops at %d%% to preserve battery health." % health["limit_pct"], "dim"))

        profile = core.usage_profile(rows)
        peak = max([b["avg_w"] or 0 for b in profile] + [1])
        listing = boxed_list()
        for band in profile:
            line = Gtk.Box(spacing=16)
            for side in ("top", "bottom"):
                getattr(line, "set_margin_" + side)(10)
            line.set_margin_start(14)
            line.set_margin_end(14)
            name = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1)
            name.set_size_request(120, -1)
            name.append(label(band["band"], "row-title"))
            name.append(label(band["cpu_range"], "dim", "caption"))
            line.append(name)
            line.append(level_bar((band["avg_w"] or 0) / peak, "usage-bar"))
            for text, width in (("%.1f W" % band["avg_w"] if band["avg_w"] is not None else "—", 64),
                                (core.fmt_hours(band["runtime_h"]), 76)):
                cell = label(text, "numeric", xalign=1.0)
                cell.set_size_request(width, -1)
                line.append(cell)
            listing.append(line)
        header = Gtk.Box(spacing=16)
        header.set_margin_start(14)
        header.set_margin_end(14)
        spacer = Gtk.Box(hexpand=True)
        header.append(spacer)
        for text, width in (("Average draw", 64), ("Full charge", 76)):
            cell = label(text, "dim", "caption", xalign=1.0)
            cell.set_size_request(width, -1)
            header.append(cell)
        block = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        block.append(header)
        block.append(listing)
        page.append(section("Runtime by usage", block))

        profiles = core.profile_breakdown(rows)
        if profiles:
            listing = boxed_list()
            for item in profiles:
                line = Gtk.Box(spacing=16)
                line.set_margin_top(10)
                line.set_margin_bottom(10)
                line.set_margin_start(14)
                line.set_margin_end(14)
                text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1, hexpand=True)
                text.append(label(item["label"], "row-title"))
                text.append(label("%s on battery" % core.fmt_hours(item["hours"]), "dim", "caption"))
                line.append(text)
                for value, sub in (("%.1f W" % item["avg_w"], "average"),
                                   ("%.1f W" % item["idle_w"] if item["idle_w"] else "—", "at idle"),
                                   ("%.1f W" % item["light_w"] if item["light_w"] else "—", "at light use")):
                    col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END)
                    col.set_size_request(84, -1)
                    col.append(label(value, "numeric", xalign=1.0))
                    col.append(label(sub, "dim", "caption", xalign=1.0))
                    line.append(col)
                listing.append(line)
            page.append(section("Power profiles", listing))

        tips = core.suggestions(self.open(), rows) if self.open() else []
        if tips:
            listing = boxed_list()
            icons = {"display": "display-brightness-symbolic", "app": "applications-system-symbolic",
                     "profile": "battery-level-50-symbolic", "standby": "weather-clear-night-symbolic", "health": "emblem-important-symbolic"}
            for tip in tips:
                line = Gtk.Box(spacing=14)
                line.set_margin_top(12)
                line.set_margin_bottom(12)
                line.set_margin_start(14)
                line.set_margin_end(14)
                line.append(Gtk.Image(icon_name=icons.get(tip["kind"], "dialog-information-symbolic"), pixel_size=24, valign=Gtk.Align.START))
                text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3, hexpand=True)
                text.append(label(tip["title"], "row-title"))
                text.append(label(tip["body"], "dim", wrap=True))
                line.append(text)
                listing.append(line)
            page.append(section("Suggestions from your own data", listing))

        culprits = core.top_culprits(rows)
        if culprits:
            listing = boxed_list()
            for item in culprits:
                line = Gtk.Box(spacing=12)
                line.set_margin_top(9)
                line.set_margin_bottom(9)
                line.set_margin_start(14)
                line.set_margin_end(14)
                line.append(label(item["name"], "row-title", hexpand=True))
                line.append(label("%.1f%% of CPU" % item["avg_cpu"], "dim", "numeric"))
                listing.append(line)
            page.append(section("Biggest CPU users on battery", listing))

    def build_history(self, conn, now):
        page = self.pages["history"]
        bar = Gtk.Box(spacing=0, halign=Gtk.Align.CENTER)
        bar.add_css_class("linked")
        first = None
        for text, seconds in RANGES:
            button = Gtk.ToggleButton(label=text, active=seconds == self.range_seconds)
            if first:
                button.set_group(first)
            first = first or button
            button.connect("toggled", self.on_range, seconds)
            bar.append(button)
        page.append(bar)
        start = now - self.range_seconds
        rows = core.fetch(conn, start, now)
        if self.range_seconds > 86400:  # thin out long ranges so drawing stays quick
            rows = rows[::max(1, len(rows) // 1500)]
        self.chart.set_data(rows, core.events(conn, start), start, now)
        page.append(self.chart)
        legend = Gtk.Box(spacing=18, halign=Gtk.Align.CENTER)
        for text, colour in (("System power", BLUEBERRY), ("Charge", LIME), ("Power rise", ORANGE), ("Power drop", GRAPE)):
            item = label("", "legend")
            item.set_markup('<span foreground="#%02x%02x%02x">●</span>  %s' % (*[int(c * 255) for c in colour], text))
            legend.append(item)
        page.append(legend)
        page.append(label("Move over the chart for exact values. Green shading is time on the charger; triangles are sudden "
                          "power changes, listed under Power events.", "dim", "caption", wrap=True, xalign=0.5))

    def on_range(self, button, seconds):
        if button.get_active() and seconds != self.range_seconds:
            self.range_seconds = seconds
            GLib.idle_add(self.refresh)

    def build_sessions(self, rows):
        page = self.pages["sessions"]
        runs = list(reversed(core.sessions(rows)))
        if not runs:
            page.append(empty("No runs on battery yet", "Unplug the charger and each run is recorded here.", "document-open-recent-symbolic"))
            return
        listing = boxed_list()
        for run in runs:
            line = Gtk.Box(spacing=16)
            line.set_margin_top(10)
            line.set_margin_bottom(10)
            line.set_margin_start(14)
            line.set_margin_end(14)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True)
            text.append(label(time.strftime("%A, %d %B · %H:%M", time.localtime(run["start"])), "row-title"))
            text.append(label("%s on battery · %.0f%% → %.0f%%" % (core.fmt_hours(run["seconds"] / 3600), run["pct_start"] or 0, run["pct_end"] or 0), "dim"))
            line.append(text)
            line.append(RangeBar(run["pct_start"], run["pct_end"]))
            for value, sub, width in (("%.1f Wh" % run["wh_used"], "used", 64),
                                      ("%s W" % (run["avg_w"] if run["avg_w"] is not None else "—"), "average", 56),
                                      (core.fmt_hours(run["projected_hours"]), "full charge", 76)):
                col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END)
                col.set_size_request(width, -1)
                col.append(label(value, "numeric", xalign=1.0))
                col.append(label(sub, "dim", "caption", xalign=1.0))
                line.append(col)
            listing.append(line)
        page.append(section("On battery", listing))
        self.build_charging(page, rows)

    def build_charging(self, page, rows):
        charges = list(reversed(core.charge_sessions(rows)))
        if not charges:
            return
        listing = boxed_list()
        for run in charges:
            line = Gtk.Box(spacing=16)
            line.set_margin_top(10)
            line.set_margin_bottom(10)
            line.set_margin_start(14)
            line.set_margin_end(14)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True)
            text.append(label(time.strftime("%A, %d %B · %H:%M", time.localtime(run["start"])), "row-title"))
            text.append(label("%s charging · %.0f%% → %.0f%%" % (core.fmt_hours(run["seconds"] / 3600), run["pct_start"] or 0, run["pct_end"] or 0), "dim"))
            line.append(text)
            line.append(RangeBar(run["pct_start"], run["pct_end"]))
            for value, sub, width in (("+%.1f Wh" % run["wh_added"], "added", 64), ("%.1f W" % run["avg_w"], "average", 56)):
                col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END)
                col.set_size_request(width, -1)
                col.append(label(value, "numeric", xalign=1.0))
                col.append(label(sub, "dim", "caption", xalign=1.0))
                line.append(col)
            listing.append(line)
        page.append(section("Charging", listing))

    def build_apps(self, rows):
        page = self.pages["apps"]
        data = core.energy_by_app(rows)
        if not data["apps"]:
            page.append(empty("No data yet", "After a little time on battery, this shows which apps used the energy.", "applications-system-symbolic"))
            return
        page.append(label("Estimated from the last 14 days on battery. The idle floor (%.1f W: display, radios, memory) is shown "
                          "separately; draw above it is shared between apps by how much CPU each used." % data["floor_w"], "dim", wrap=True))
        total = max(data["total_wh"], 0.01)
        entries = [(a["name"], a["wh"]) for a in data["apps"][:12]]
        entries.append(("Other CPU and kernel", data["other_wh"]))
        entries.append(("Idle floor", data["floor_wh"]))
        peak = max(w for _n, w in entries) or 1
        listing = boxed_list()
        for name, wh in entries:
            line = Gtk.Box(spacing=16)
            line.set_margin_top(9)
            line.set_margin_bottom(9)
            line.set_margin_start(14)
            line.set_margin_end(14)
            title = label(name, "row-title")
            title.set_size_request(170, -1)
            line.append(title)
            line.append(level_bar(wh / peak, "usage-bar"))
            for text, width in (("%.1f Wh" % wh, 64), ("%.0f%%" % (100 * wh / total), 48)):
                cell = label(text, "numeric", xalign=1.0)
                cell.set_size_request(width, -1)
                line.append(cell)
            listing.append(line)
        page.append(section("Where the energy went", listing))

    def build_devices(self, conn, now):
        page = self.pages["devices"]
        devices = core.device_summary(conn)
        if not devices:
            page.append(empty("No accessories seen", "Bluetooth mice and keyboards that report a battery appear here.", "input-mouse-symbolic"))
            return
        listing = boxed_list()
        for dev in devices:
            line = Gtk.Box(spacing=14)
            line.set_margin_top(12)
            line.set_margin_bottom(12)
            line.set_margin_start(14)
            line.set_margin_end(14)
            low = (dev["level"] or "").lower() in ("low", "critical")
            line.append(BatteryGauge(dev["pct"] if dev["pct"] is not None else (15 if low else 80), False, 24, 38, number=dev["pct"] is not None))
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True)
            text.append(label(dev["name"], "row-title"))
            text.append(label("%s since %s" % (dev["level"] or dev["status"] or "—", time.strftime("%a %d %b, %H:%M", time.localtime(dev["since"]))), "dim"))
            line.append(text)
            right = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END)
            right.append(label("%.0f%%" % dev["pct"] if dev["pct"] is not None else (dev["level"] or "—"), "stat-value" if dev["pct"] is not None else "numeric", xalign=1.0))
            if dev["days_left"] is not None:
                right.append(label("about %.0f days left" % dev["days_left"], "dim", "caption", xalign=1.0))
            line.append(right)
            listing.append(line)
        page.append(section("Accessories", listing))
        page.append(label("Some devices report only a level (Normal, Low, Critical) instead of a percentage; those show how long they have been at the current level.", "dim", "caption", wrap=True))

    def build_diagnostics(self, conn):
        page = self.pages["diagnostics"]
        checks = core.diagnose(conn)
        bad = sum(1 for c in checks if c["status"] != "ok")
        page.append(label("%d checks, %s" % (len(checks), "all passing" if not bad else "%d need attention" % bad), "stat-value"))
        listing = boxed_list()
        icons = {"ok": "emblem-ok-symbolic", "warn": "dialog-warning-symbolic", "fail": "dialog-error-symbolic"}
        for c in checks:
            line = Gtk.Box(spacing=12)
            line.set_margin_top(9)
            line.set_margin_bottom(9)
            line.set_margin_start(14)
            line.set_margin_end(14)
            line.append(Gtk.Image(icon_name=icons[c["status"]], pixel_size=18))
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1, hexpand=True)
            text.append(label(c["name"], "row-title"))
            text.append(label(c["detail"], "dim", wrap=True))
            line.append(text)
            listing.append(line)
        page.append(listing)
        save = Gtk.Button(label="Save Diagnostics Report…", halign=Gtk.Align.START)
        save.connect("clicked", lambda _b: self.on_export(None, diagnostics=True))
        page.append(save)

    def build_health(self, conn, rows, now):
        page = self.pages["health"]
        health = core.battery_health()
        trend = core.health_trend(conn)
        cond = core.condition()
        banner = Gtk.Box(spacing=12)
        banner.add_css_class("tile")
        banner.append(Gtk.Image(icon_name="emblem-default-symbolic" if cond["level"] in ("good", "ok") else "dialog-warning-symbolic", pixel_size=28))
        words = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        words.append(label("Battery condition: %s" % cond["label"], "stat-value"))
        words.append(label(cond["why"], "dim", wrap=True))
        banner.append(words)
        page.append(banner)
        tiles = Gtk.Box(spacing=12, homogeneous=True)
        tiles.append(tile("Capacity", "%s%%" % health.get("health_pct", "—"), "%.1f of %.1f Wh design" % (health.get("full_wh") or 0, health.get("design_wh") or 0)))
        tiles.append(tile("Cycles", "%d" % (health.get("cycles") or 0), "full charge equivalents"))
        tiles.append(tile("Temperature", "%.0f °C" % health["temp_c"] if health.get("temp_c") is not None else "—",
                          "limit %d%%" % health["limit_pct"] if health.get("limit_pct") is not None else ""))
        page.append(tiles)
        days = core.daily_usage(core.fetch(conn, now - 12 * 86400), 10)
        if days:
            peak = max([d["wh"] for d in days] + [1])
            usage = boxed_list()
            for d in reversed(days):
                line = Gtk.Box(spacing=16)
                line.set_margin_top(9)
                line.set_margin_bottom(9)
                line.set_margin_start(14)
                line.set_margin_end(14)
                title = label(time.strftime("%a %d %b", time.strptime(d["day"], "%Y-%m-%d")), "row-title")
                title.set_size_request(110, -1)
                line.append(title)
                line.append(level_bar(d["wh"] / peak, "usage-bar"))
                for text, width in (("%.1f Wh" % d["wh"], 66), (core.fmt_hours(d["battery_h"]), 70),
                                    ("low %.0f%%" % d["min_pct"] if d["min_pct"] is not None else "—", 64)):
                    cell = label(text, "numeric", xalign=1.0)
                    cell.set_size_request(width, -1)
                    line.append(cell)
                usage.append(line)
            page.append(section("Last 10 days on battery", usage))
        if len(trend) < 2:
            page.append(label("The capacity trend builds up one point per day; check back tomorrow.", "dim"))
            return
        listing = boxed_list()
        for day in reversed(trend[-30:]):
            line = Gtk.Box(spacing=16)
            line.set_margin_top(9)
            line.set_margin_bottom(9)
            line.set_margin_start(14)
            line.set_margin_end(14)
            line.append(label(day["day"], "row-title", hexpand=True))
            pct = 100.0 * day["full_uwh"] / day["design_uwh"] if day["design_uwh"] else 0
            for text, width in (("%.1f%%" % pct, 56), ("%.1f Wh" % (day["full_uwh"] / 1e6), 70), ("%d cycles" % day["cycles"], 80)):
                cell = label(text, "numeric", xalign=1.0)
                cell.set_size_request(width, -1)
                line.append(cell)
            listing.append(line)
        page.append(section("Capacity by day", listing))

    def build_events(self, conn, now):
        page = self.pages["events"]
        found = core.events(conn, now - 2 * 86400)
        if not found:
            page.append(empty("No power events", "Sudden changes in power use and sleep gaps appear here with their likely cause.", "dialog-information-symbolic"))
            return
        page.append(label("Last 48 hours. A step of 4 W or more is logged with what changed alongside it.", "dim"))
        listing = boxed_list()
        for ev in found:
            if ev["kind"] == "gap":
                info = json.loads(ev["detail"])
                title = "Asleep or off for %s" % core.fmt_hours(info["seconds"] / 3600)
                lost = (info["pct_from"] or 0) - (info["pct_to"] or 0)
                sub = "Charge %.0f%% → %.0f%%%s" % (info["pct_from"] or 0, info["pct_to"] or 0, " · standby drain" if info["on_battery"] and lost > 0 else "")
                icon, css, trailing = "weather-clear-night-symbolic", "sleep-icon", ""
            else:
                up = ev["kind"] == "rise"
                title = "Power %s from %.1f W to %.1f W" % ("rose" if up else "dropped", ev["before_w"], ev["after_w"])
                sub = "%s · top process: %s" % (ev["detail"], ev["culprit"] or "—")
                icon, css, trailing = ("go-up-symbolic", "rise-icon", "") if up else ("go-down-symbolic", "drop-icon", "")
                trailing = "%+.1f W" % ev["delta_w"]
            line = Gtk.Box(spacing=12)
            line.set_margin_top(10)
            line.set_margin_bottom(10)
            line.set_margin_start(14)
            line.set_margin_end(14)
            image = Gtk.Image(icon_name=icon, pixel_size=20)
            image.add_css_class(css)
            line.append(image)
            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, hexpand=True)
            text.append(label(title, "row-title"))
            text.append(label(sub, "dim", wrap=True))
            line.append(text)
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, halign=Gtk.Align.END)
            if trailing:
                col.append(label(trailing, "numeric", xalign=1.0))
            col.append(label(when(ev["ts"]), "dim", "caption", xalign=1.0))
            line.append(col)
            listing.append(line)
        page.append(listing)


class BatteryApp(Gtk.Application):
    def __init__(self):
        super().__init__(application_id="org.mseries.Battery", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_startup(self):
        Gtk.Application.do_startup(self)
        if hasattr(Granite, "init"):
            Granite.init()
        self.desktop_settings = Granite.Settings.get_default()
        self.desktop_settings.connect("notify::prefers-color-scheme", self.update_color_scheme)
        self.update_color_scheme()
        provider = Gtk.CssProvider()
        provider.load_from_data(CSS)
        from gi.repository import Gdk
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def update_color_scheme(self, *_args):
        Gtk.Settings.get_default().set_property(
            "gtk-application-prefer-dark-theme",
            self.desktop_settings.get_prefers_color_scheme() == Granite.SettingsColorScheme.DARK)

    def do_activate(self):
        window = self.props.active_window or BatteryWindow(self)
        window.present()


if __name__ == "__main__":
    sys.exit(BatteryApp().run(sys.argv))
