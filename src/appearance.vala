namespace SignalNative {
public class Appearance : Object {
    private static bool message_styles_installed = false;
    public static void install () {
        if (message_styles_installed) {
            return;
        }

        var display = Gdk.Display.get_default ();
        if (display == null) {
            return;
        }

        var provider = new Gtk.CssProvider ();
        provider.load_from_string ("""
            .message-bubble {
                padding: 8px 12px;
                border-radius: 12px;
            }
            .message-bubble.incoming {
                background-color: alpha(@theme_fg_color, 0.06);
            }
            .message-bubble.outgoing {
                background-color: alpha(@accent_color, 0.16);
            }
            """);
        Gtk.StyleContext.add_provider_for_display (
            display,
            provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        );
        message_styles_installed = true;
    }

}
}
