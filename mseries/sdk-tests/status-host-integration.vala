/* Exercises the shared T3 menu in either status host, with isolated neighbors. */
private Gtk.Stack? find_pages (Gtk.Widget widget) {
    if (widget is Gtk.Stack) {
        var stack = (Gtk.Stack) widget;
        if (stack.get_child_by_name ("sdk-usage") != null) return stack;
    }
    for (var child = widget.get_first_child (); child != null; child = child.get_next_sibling ()) {
        var found = find_pages (child);
        if (found != null) return found;
    }
    return null;
}
private bool has_label (Gtk.Widget widget, string expected) {
    if (widget is Gtk.Label && ((Gtk.Label) widget).label == expected) return true;
    for (var child = widget.get_first_child (); child != null; child = child.get_next_sibling ())
        if (has_label (child, expected)) return true;
    return false;
}
private bool wait_label (Gtk.Widget widget, string expected) {
    for (int i = 0; i < 500; i++) {
        while (GLib.MainContext.default ().iteration (false)) { }
        if (has_label (widget, expected)) return true;
        GLib.Thread.usleep (10000);
    }
    return false;
}
private bool wait_reads (string path, int expected) {
    for (int i = 0; i < 500; i++) {
        while (GLib.MainContext.default ().iteration (false)) { }
        try {
            string text;
            if (GLib.FileUtils.get_contents (path, out text) && text.length == expected) return true;
        } catch (GLib.Error error) { }
        GLib.Thread.usleep (10000);
    }
    return false;
}
int main (string[] args) {
    assert (args.length == 3);
    bool hosted = args[2] == "wingpanel";
    assert (hosted || args[2] == "dock");
    Gtk.init ();
    var window = new Gtk.Window ();
    var anchor = new Gtk.Button.with_label ("T3");
    var host = new Dock.TaskbarT3RPopover (anchor, 28, hosted);
    window.child = hosted ? host.get_host_widget () : anchor;
    window.present ();
    if (hosted) host.open_host ();
    else host.toggle_from_icon ();
    assert (!GLib.FileUtils.test (args[1], GLib.FileTest.EXISTS));
    var pages = find_pages (hosted ? (Gtk.Widget) window : (Gtk.Widget) anchor);
    assert (pages != null);
    pages.visible_child_name = "sdk-usage";
    assert (wait_reads (args[1], 1));
    assert (wait_label (pages, "7 day: 75% remaining"));
    pages.visible_child_name = "server";
    pages.visible_child_name = "sdk-usage";
    assert (wait_reads (args[1], 2));
    host.close_host ();
    window.child = null;
    for (int i = 0; i < 50; i++) {
        while (GLib.MainContext.default ().iteration (false)) { }
        GLib.Thread.usleep (10000);
    }
    string read_log;
    assert (GLib.FileUtils.get_contents (args[1], out read_log));
    assert (read_log.length == 2);
    host.dispose ();
    window.destroy ();
    return 0;
}
