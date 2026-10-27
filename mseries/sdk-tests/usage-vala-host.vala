/* Isolated Vala host: exercises the page used by desktop status hosts. */
int main (string[] args) {
    assert (args.length == 3);
    Gtk.init ();
    var unavailable = new MSeries.SdkUsagePage ("relative-program");
    unavailable.opened ();
    unavailable.closed ();
    unavailable.dispose ();
    MSeries.SdkUsagePage? page = new MSeries.SdkUsagePage (args[1]);
    var window = new Gtk.Window ();
    window.child = page.get_widget ();
    window.present ();
    assert (!GLib.FileUtils.test (args[2], GLib.FileTest.EXISTS));
    page.opened ();
    var loop = new GLib.MainLoop ();
    int attempts = 0;
    bool read = false;
    GLib.Timeout.add (20, () => {
        read = GLib.FileUtils.test (args[2], GLib.FileTest.EXISTS);
        if (read || ++attempts >= 250) {
            loop.quit ();
            return GLib.Source.REMOVE;
        }
        return GLib.Source.CONTINUE;
    });
    loop.run ();
    page.closed ();
    window.child = null;
    page = null;
    window.destroy ();
    assert (read);
    return 0;
}
