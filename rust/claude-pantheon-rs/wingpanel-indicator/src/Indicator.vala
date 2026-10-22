/*
 * Claude for elementary OS: Wingpanel indicator.
 *
 * T3 Code has no desktop tray or menubar indicator (only a phone widget), so this is the one
 * at-a-glance view of the server: running threads, pending permission prompts and the plan windows
 * nearest exhaustion. It reads ~/.cache/claude-pantheon-rs/status.json, written by the client whenever
 * the shell or the provider snapshot changes (src/status.rs), and watches it with a FileMonitor; it
 * never polls and never talks to the server itself. Layout follows the CodexBar indicator in
 * mseries/codexbar-wingpanel: dense cards, a token-first bar row, a bottom bar with freshness.
 */

public class ClaudePantheon.Indicator : Wingpanel.Indicator {
    private const string APP_ID = "io.github.techmore.ClaudePantheonRs";
    private Gtk.Box? display_widget = null;
    private Gtk.Label badge;
    private Gtk.Image icon;
    private Gtk.Box? popover_widget = null;
    private Gtk.Box windows_box;
    private Gtk.Box running_box;
    private Gtk.Label next_label;
    private Gtk.Label freshness;
    private GLib.FileMonitor? monitor = null;
    private GLib.File status_file;

    public Indicator () {
        Object (code_name: "claude-pantheon");
        status_file = GLib.File.new_for_path (
            Path.build_filename (Environment.get_user_cache_dir (), "claude-pantheon-rs", "status.json"));
        visible = true;
        try {
            monitor = status_file.monitor_file (GLib.FileMonitorFlags.NONE);
            monitor.changed.connect (() => reload ());
        } catch (Error e) {
            warning ("status feed monitor: %s", e.message);
        }
    }

    public override Gtk.Widget get_display_widget () {
        if (display_widget == null) {
            icon = new Gtk.Image.from_icon_name ("mail-read-symbolic");
            badge = new Gtk.Label ("");
            badge.add_css_class ("caption");
            display_widget = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 4);
            display_widget.append (icon);
            display_widget.append (badge);
            reload ();
        }
        return display_widget;
    }

    public override Gtk.Widget? get_widget () {
        if (popover_widget == null) {
            popover_widget = new Gtk.Box (Gtk.Orientation.VERTICAL, 6) { width_request = 320 };
            var header = new Granite.HeaderLabel ("Plan windows");
            windows_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 4);
            next_label = new Gtk.Label ("") { xalign = 0, wrap = true };
            next_label.add_css_class ("caption");
            var running_header = new Granite.HeaderLabel ("Threads");
            running_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 2);
            freshness = new Gtk.Label ("") { xalign = 0 };
            freshness.add_css_class ("caption");
            freshness.add_css_class ("dim-label");
            var open = new Gtk.Button.with_label ("Open Claude") { halign = Gtk.Align.END };
            open.clicked.connect (() => {
                try {
                    var info = new GLib.DesktopAppInfo (APP_ID + ".desktop");
                    if (info != null) {
                        info.launch (null, null);
                    }
                } catch (Error e) {
                    warning ("launch: %s", e.message);
                }
                close ();
            });
            var bottom = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 6);
            freshness.hexpand = true;
            bottom.append (freshness);
            bottom.append (open);
            foreach (var w in new Gtk.Widget[] { header, windows_box, next_label, running_header, running_box, bottom }) {
                w.margin_start = 12;
                w.margin_end = 12;
                popover_widget.append (w);
            }
            bottom.margin_bottom = 8;
            reload ();
        }
        return popover_widget;
    }

    public override void opened () { reload (); }
    public override void closed () { }

    private static void clear (Gtk.Box box) {
        Gtk.Widget? child = box.get_first_child ();
        while (child != null) {
            var next = child.get_next_sibling ();
            box.remove (child);
            child = next;
        }
    }

    private void reload () {
        var parser = new Json.Parser ();
        try {
            parser.load_from_file (status_file.get_path ());
        } catch (Error e) {
            if (badge != null) badge.label = "";
            if (freshness != null) freshness.label = "Claude is not running";
            return;
        }
        var root = parser.get_root ().get_object ();
        var running = root.get_array_member ("running");
        var pending = root.get_array_member ("pendingPrompts");
        var windows = root.get_array_member ("windows");
        int prompts = 0;
        foreach (var p in pending.get_elements ()) {
            prompts += (int) p.get_object ().get_int_member ("count");
        }
        if (badge != null) {
            badge.label = prompts > 0 ? "%d".printf (prompts) : (running.get_length () > 0 ? "•" : "");
            icon.icon_name = prompts > 0 ? "dialog-question-symbolic" : "mail-read-symbolic";
        }
        if (popover_widget == null) return;
        clear (windows_box);
        foreach (var w in windows.get_elements ()) {
            var o = w.get_object ();
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            var name = new Gtk.Label ("%s %s".printf (o.get_string_member ("provider"), o.get_string_member ("label"))) {
                xalign = 0, width_chars = 16
            };
            var bar = new Gtk.LevelBar () { min_value = 0, max_value = 100, hexpand = true, valign = Gtk.Align.CENTER };
            bar.add_offset_value ("low", 60);
            bar.add_offset_value ("high", 90);
            bar.add_offset_value ("full", 100);
            bar.value = o.get_double_member ("usedPercent");
            var pct = new Gtk.Label ("%.0f%%".printf (o.get_double_member ("usedPercent"))) { xalign = 1, width_chars = 5 };
            row.append (name);
            row.append (bar);
            row.append (pct);
            windows_box.append (row);
        }
        var next = root.get_member ("nextTask");
        next_label.label = next.get_node_type () == Json.NodeType.OBJECT
            ? next.get_object ().get_string_member ("reason")
            : "every window is full; the runner idles until the earliest reset";
        clear (running_box);
        foreach (var t in running.get_elements ()) {
            var o = t.get_object ();
            var l = new Gtk.Label ("▶ %s · %s".printf (o.get_string_member ("title"), o.get_string_member ("provider"))) {
                xalign = 0, ellipsize = Pango.EllipsizeMode.END
            };
            running_box.append (l);
        }
        foreach (var t in pending.get_elements ()) {
            var o = t.get_object ();
            var l = new Gtk.Label ("? %s needs you (%d)".printf (o.get_string_member ("title"), (int) o.get_int_member ("count"))) {
                xalign = 0, ellipsize = Pango.EllipsizeMode.END
            };
            running_box.append (l);
        }
        if (running.get_length () == 0 && pending.get_length () == 0) {
            running_box.append (new Gtk.Label ("Nothing running") { xalign = 0 });
        }
        freshness.label = "%s · %s".printf (
            root.get_boolean_member ("connected") ? root.get_string_member ("server") : "not connected",
            root.get_string_member ("writtenAt"));
    }
}

public Wingpanel.Indicator? get_indicator (Module module, Wingpanel.IndicatorManager.ServerType server_type) {
    if (server_type != Wingpanel.IndicatorManager.ServerType.SESSION) {
        return null;
    }
    return new ClaudePantheon.Indicator ();
}
