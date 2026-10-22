/*
 * Claude for elementary OS: Wingpanel indicator.
 *
 * T3 Code has no desktop tray or menubar indicator (only a phone widget), so this is the one
 * at-a-glance view of the server: a badge with the number of pending permission prompts (a dot while
 * a thread runs), and a popover with the plan window nearest exhaustion and its reset countdown, every
 * window as a gauge, the rotation decision, running threads and pending prompts (clicking one opens
 * that thread in the app, which answers it). It reads ~/.cache/claude-pantheon-rs/status.json, written
 * by the client whenever the shell or the provider snapshot changes (src/status.rs), watches it with a
 * FileMonitor, never polls and never talks to the server. Layout follows the CodexBar indicator in
 * mseries/codexbar-wingpanel: dense rows, a bottom bar with freshness and an open button.
 */

public class ClaudePantheon.Feed : Object {
    public bool present = false;
    public bool connected = false;
    public string server = "";
    public string written_at = "";
    public Gee.ArrayList<Window> windows = new Gee.ArrayList<Window> ();
    public Gee.ArrayList<Thread> running = new Gee.ArrayList<Thread> ();
    public Gee.ArrayList<Thread> pending = new Gee.ArrayList<Thread> ();
    public string next_task = "";

    public class Window : Object {
        public string provider = "";
        public string kind = "";
        public string label = "";
        public double used_percent = 0;
        public string resets_at = "";
    }

    public class Thread : Object {
        public string id = "";
        public string title = "";
        public string provider = "";
        public int count = 0;
    }

    public static Feed load (string path) {
        var feed = new Feed ();
        var parser = new Json.Parser ();
        try {
            parser.load_from_file (path);
        } catch (Error e) {
            return feed;
        }
        var root = parser.get_root ();
        if (root == null || root.get_node_type () != Json.NodeType.OBJECT) {
            return feed;
        }
        var o = root.get_object ();
        feed.present = true;
        feed.connected = o.has_member ("connected") && o.get_boolean_member ("connected");
        feed.server = o.has_member ("server") ? o.get_string_member ("server") : "";
        feed.written_at = o.has_member ("writtenAt") ? o.get_string_member ("writtenAt") : "";
        if (o.has_member ("windows")) {
            foreach (var n in o.get_array_member ("windows").get_elements ()) {
                var w = n.get_object ();
                var win = new Window ();
                win.provider = w.get_string_member ("provider");
                win.kind = w.get_string_member ("kind");
                win.label = w.get_string_member ("label");
                win.used_percent = w.get_double_member ("usedPercent");
                win.resets_at = w.has_member ("resetsAt") && !w.get_null_member ("resetsAt") ? w.get_string_member ("resetsAt") : "";
                feed.windows.add (win);
            }
        }
        foreach (var key in new string[] { "running", "pendingPrompts" }) {
            if (!o.has_member (key)) continue;
            foreach (var n in o.get_array_member (key).get_elements ()) {
                var t = n.get_object ();
                var thread = new Thread ();
                thread.id = t.get_string_member ("id");
                thread.title = t.get_string_member ("title");
                thread.provider = t.has_member ("provider") ? t.get_string_member ("provider") : "";
                thread.count = t.has_member ("count") ? (int) t.get_int_member ("count") : 0;
                (key == "running" ? feed.running : feed.pending).add (thread);
            }
        }
        if (o.has_member ("nextTask") && !o.get_null_member ("nextTask")) {
            feed.next_task = o.get_object_member ("nextTask").get_string_member ("reason");
        }
        return feed;
    }

    public int prompt_count () {
        int n = 0;
        foreach (var t in pending) n += t.count > 0 ? t.count : 1;
        return n;
    }

    /** The window closest to its limit, or null without any. */
    public Window? nearest () {
        Window? best = null;
        foreach (var w in windows) {
            if (best == null || w.used_percent > best.used_percent) best = w;
        }
        return best;
    }

    /** "resets in 1 h 24 min (22:00)" for an ISO timestamp; "" when unknown. */
    public static string countdown (string iso, DateTime now) {
        if (iso == "") return "";
        var at = new DateTime.from_iso8601 (iso, new TimeZone.utc ());
        if (at == null) return "";
        var local = at.to_local ();
        var left = at.difference (now) / TimeSpan.SECOND;
        string when;
        if (left <= 0) {
            when = "now";
        } else {
            int64 d = left / 86400, h = (left % 86400) / 3600, m = (left % 3600) / 60;
            when = d > 0 ? "in %lld d %lld h".printf (d, h) : h > 0 ? "in %lld h %lld min".printf (h, m) : "in %lld min".printf (m);
        }
        bool today = local.get_day_of_year () == now.to_local ().get_day_of_year () && local.get_year () == now.to_local ().get_year ();
        return "resets %s (%s)".printf (when, local.format (today ? "%H:%M" : "%a %H:%M"));
    }
}

public class ClaudePantheon.Indicator : Wingpanel.Indicator {
    private const string APP_ID = "io.github.techmore.ClaudePantheonRs";
    private Gtk.Box? display_widget = null;
    private Gtk.Label badge;
    private Gtk.Image icon;
    private Gtk.Box? popover_widget = null;
    private Gtk.Label headline;
    private Gtk.Label headline_detail;
    private Gtk.Box windows_box;
    private Gtk.Label next_label;
    private Gtk.Box threads_box;
    private Gtk.Label freshness;
    private GLib.FileMonitor? monitor = null;
    private string status_path;

    public Indicator (string? path = null) {
        Object (code_name: "claude-pantheon");
        status_path = path ?? Path.build_filename (Environment.get_user_cache_dir (), "claude-pantheon-rs", "status.json");
        visible = true;
        try {
            monitor = GLib.File.new_for_path (status_path).monitor_file (GLib.FileMonitorFlags.NONE);
            monitor.changed.connect (() => reload ());
        } catch (Error e) {
            warning ("status feed monitor: %s", e.message);
        }
    }

    public override Gtk.Widget get_display_widget () {
        if (display_widget == null) {
            icon = new Gtk.Image.from_icon_name (APP_ID + "-symbolic");
            badge = new Gtk.Label ("");
            badge.add_css_class ("caption");
            display_widget = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 3);
            display_widget.append (icon);
            display_widget.append (badge);
            reload ();
        }
        return display_widget;
    }

    public override Gtk.Widget? get_widget () {
        if (popover_widget == null) {
            popover_widget = new Gtk.Box (Gtk.Orientation.VERTICAL, 4) { width_request = 340 };
            headline = new Gtk.Label ("") { xalign = 0, ellipsize = Pango.EllipsizeMode.END };
            headline.add_css_class (Granite.STYLE_CLASS_H3_LABEL);
            headline_detail = new Gtk.Label ("") { xalign = 0, ellipsize = Pango.EllipsizeMode.END };
            headline_detail.add_css_class ("caption");
            headline_detail.add_css_class (Granite.STYLE_CLASS_DIM_LABEL);
            windows_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 3);
            next_label = new Gtk.Label ("") { xalign = 0, wrap = true };
            next_label.add_css_class ("caption");
            var threads_header = new Granite.HeaderLabel ("Threads");
            threads_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 2);
            freshness = new Gtk.Label ("") { xalign = 0, hexpand = true, ellipsize = Pango.EllipsizeMode.END };
            freshness.add_css_class ("caption");
            freshness.add_css_class (Granite.STYLE_CLASS_DIM_LABEL);
            var open = new Gtk.Button.with_label ("Open Claude") { halign = Gtk.Align.END };
            open.clicked.connect (() => {
                launch (null);
                close ();
            });
            var bottom = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 6);
            bottom.append (freshness);
            bottom.append (open);
            foreach (var w in new Gtk.Widget[] { headline, headline_detail, windows_box, next_label, threads_header, threads_box, bottom }) {
                w.margin_start = 12;
                w.margin_end = 12;
                popover_widget.append (w);
            }
            headline.margin_top = 8;
            bottom.margin_bottom = 8;
            bottom.margin_top = 4;
            reload ();
        }
        return popover_widget;
    }

    public override void opened () { reload (); }
    public override void closed () { }

    /** Opens the app, on a thread when `thread_id` is given, through its GApplication actions. */
    private void launch (string? thread_id) {
        try {
            string[] argv = thread_id == null
                ? new string[] { "gapplication", "launch", APP_ID }
                : new string[] { "gapplication", "action", APP_ID, "open-thread", "'" + thread_id.replace ("'", "") + "'" };
            Process.spawn_async (null, argv, null, SpawnFlags.SEARCH_PATH, null, null);
        } catch (Error e) {
            warning ("launch: %s", e.message);
        }
    }

    private static void clear (Gtk.Box box) {
        Gtk.Widget? child = box.get_first_child ();
        while (child != null) {
            var next = child.get_next_sibling ();
            box.remove (child);
            child = next;
        }
    }

    public void reload () {
        var feed = Feed.load (status_path);
        render (feed, new DateTime.now_utc ());
    }

    /** Fills every widget from a feed; separate from reload so the probe can render a fixture at a fixed time. */
    public void render (Feed feed, DateTime now) {
        int prompts = feed.prompt_count ();
        if (badge != null) {
            badge.label = prompts > 0 ? "%d".printf (prompts) : (feed.running.size > 0 ? "•" : "");
            icon.icon_name = prompts > 0 ? "dialog-question-symbolic" : APP_ID + "-symbolic";
        }
        if (popover_widget == null) return;
        var nearest = feed.nearest ();
        if (!feed.present) {
            headline.label = "Claude is not running";
            headline_detail.label = "Open the app to pair with the T3 server";
        } else if (nearest == null) {
            headline.label = feed.connected ? "No plan windows reported" : "Not connected";
            headline_detail.label = feed.connected ? feed.server : "Reconnecting from the app";
        } else {
            headline.label = "%s %s %.0f%%".printf (nearest.provider, nearest.label.down (), nearest.used_percent);
            headline_detail.label = Feed.countdown (nearest.resets_at, now);
        }
        clear (windows_box);
        foreach (var w in feed.windows) {
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            var name = new Gtk.Label ("%s · %s".printf (w.provider, w.label)) { xalign = 0, width_chars = 16, ellipsize = Pango.EllipsizeMode.END };
            name.add_css_class ("caption");
            var bar = new Gtk.LevelBar () { min_value = 0, max_value = 100, hexpand = true, valign = Gtk.Align.CENTER };
            bar.add_offset_value ("low", 60);
            bar.add_offset_value ("high", 90);
            bar.add_offset_value ("full", 100);
            bar.value = w.used_percent;
            var pct = new Gtk.Label ("%.0f%%".printf (w.used_percent)) { xalign = 1, width_chars = 4 };
            pct.add_css_class ("caption");
            pct.tooltip_text = Feed.countdown (w.resets_at, now);
            row.append (name);
            row.append (bar);
            row.append (pct);
            windows_box.append (row);
        }
        next_label.label = feed.next_task != "" ? feed.next_task : (feed.windows.size > 0 ? "every window is full; the runner idles until the earliest reset" : "");
        next_label.visible = next_label.label != "";
        clear (threads_box);
        foreach (var t in feed.pending) {
            var b = new Gtk.Button () { halign = Gtk.Align.FILL };
            b.add_css_class (Granite.STYLE_CLASS_FLAT);
            var l = new Gtk.Label ("? %s needs you (%d)".printf (t.title, t.count)) { xalign = 0, ellipsize = Pango.EllipsizeMode.END };
            b.child = l;
            b.tooltip_text = "Open this thread to answer";
            var id = t.id;
            b.clicked.connect (() => {
                launch (id);
                close ();
            });
            threads_box.append (b);
        }
        foreach (var t in feed.running) {
            var b = new Gtk.Button () { halign = Gtk.Align.FILL };
            b.add_css_class (Granite.STYLE_CLASS_FLAT);
            var l = new Gtk.Label ("▶ %s · %s".printf (t.title, t.provider)) { xalign = 0, ellipsize = Pango.EllipsizeMode.END };
            b.child = l;
            var id = t.id;
            b.clicked.connect (() => {
                launch (id);
                close ();
            });
            threads_box.append (b);
        }
        if (feed.running.size == 0 && feed.pending.size == 0) {
            var l = new Gtk.Label ("Nothing running") { xalign = 0, margin_start = 6 };
            l.add_css_class (Granite.STYLE_CLASS_DIM_LABEL);
            threads_box.append (l);
        }
        string when = "";
        if (feed.written_at != "") {
            var written = new DateTime.from_iso8601 (feed.written_at, new TimeZone.utc ());
            if (written != null) {
                var age = now.difference (written) / TimeSpan.SECOND;
                when = age < 60 ? "updated just now" : age < 3600 ? "updated %lld min ago".printf (age / 60) : "updated %s".printf (written.to_local ().format ("%H:%M"));
            }
        }
        freshness.label = feed.present
            ? "%s · %s".printf (feed.connected ? feed.server : "not connected", when)
            : "no status feed yet";
    }
}

public Wingpanel.Indicator? get_indicator (Module module, Wingpanel.ServerType server_type) {
    if (server_type != Wingpanel.ServerType.SESSION) {
        return null;
    }
    return new ClaudePantheon.Indicator ();
}
