private class SignalNativePrototype : Gtk.Application {
    private Granite.Settings granite_settings;
    private Gtk.Settings? gtk_settings;

    public SignalNativePrototype () {
        Object (
            application_id: "io.github.sdolbec.SignalPantheonClient",
            flags: ApplicationFlags.DEFAULT_FLAGS
        );

        granite_settings = Granite.Settings.get_default ();
        granite_settings.notify["prefers-color-scheme"].connect (update_color_scheme);
    }

    private void update_color_scheme () {
        if (gtk_settings == null) {
            return;
        }
        gtk_settings.gtk_application_prefer_dark_theme =
            granite_settings.prefers_color_scheme == Granite.Settings.ColorScheme.DARK;
    }

    protected override void activate () {
        if (gtk_settings == null) {
            gtk_settings = Gtk.Settings.get_default ();
            update_color_scheme ();
        }
        SignalNative.Appearance.install ();
        foreach (Gtk.Window window in get_windows ()) {
            window.present ();
            return;
        }

        var window = new SignalNative.MainWindow (this);
        window.present ();
    }

    public static int main (string[] args) {
        return new SignalNativePrototype ().run (args);
    }
}
