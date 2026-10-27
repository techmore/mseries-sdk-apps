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
            @define-color accent_bg_color @theme_selected_bg_color;
            @define-color accent_fg_color @theme_selected_fg_color;
            .message-bubble { padding: 9px 14px; border-radius: 18px; font-size: 1.12em; }
            .message-bubble.incoming { background-color: alpha(@theme_fg_color, 0.10); border-bottom-left-radius: 5px; }
            .message-bubble.outgoing link, .message-bubble.outgoing link:visited { color: @accent_fg_color; text-decoration: underline; }
            .message-bubble.outgoing { background-color: @accent_bg_color; color: @accent_fg_color; border-bottom-right-radius: 5px; }
            .message-image { border-radius: 14px; }
            .sp-actions { opacity: 0; min-width: 22px; min-height: 22px; padding: 0; }
            .sp-avatar label { font-size: 1.05em; }
            .sp-avatar { border-radius: 99px; color: white; font-weight: 700; }
            .sp-caption { font-size: .92em; opacity: .75; margin: 0 4px; }
            .sp-composer { padding: 10px 16px 14px 16px; }
            .sp-empty { opacity: .8; }
            .sp-field textview, .sp-field textview text { background: transparent; font-size: 1.12em; }
            .sp-field { border-radius: 20px; border: 1px solid alpha(@theme_fg_color, .22); background: @theme_base_color; padding: 3px 5px 3px 4px; }
            .sp-field:focus-within { border-color: @accent_bg_color; box-shadow: 0 0 0 1px alpha(@accent_bg_color, .45); }
            .sp-heading { border-bottom: 1px solid alpha(@theme_fg_color, .10); }
            .sp-list row { border-radius: 10px; margin: 1px 8px; }
            .sp-message:hover .sp-actions, .sp-message:focus-within .sp-actions { opacity: 1; }
            .sp-name { font-weight: 600; font-size: 1.15em; }
            .sp-photo { background: alpha(@theme_fg_color, .08); }
            .sp-preview { font-size: 1.02em; opacity: .78; }
            .sp-reaction { background: alpha(@theme_fg_color, .10); border-radius: 99px; padding: 1px 8px; margin: 2px 4px 0 4px; font-size: 1.05em; }
            .sp-row { padding: 6px 4px; }
            .sp-search { margin: 10px 12px 6px 12px; }
            .sp-search-messages { margin: 0 16px 8px 16px; }
            .sp-send { min-width: 30px; min-height: 30px; padding: 0; border-radius: 99px; }
            .sp-stamp { font-size: .92em; opacity: .75; margin: 14px 0 6px 0; }
            .sp-status { font-size: .95em; opacity: .85; padding: 9px 14px; border-top: 1px solid alpha(@theme_fg_color, .10); }
            .sp-time { font-size: .92em; opacity: .75; }
            .sp-unread-dot { min-width: 9px; min-height: 9px; border-radius: 99px; background: @accent_bg_color; }
            .sp-unread-name { font-weight: 800; }
            .sp-tone-0 { background: #3689e6; } .sp-tone-1 { background: #28bca3; }
            .sp-tone-2 { background: #68b723; } .sp-tone-3 { background: #d48e15; }
            .sp-tone-4 { background: #f37329; } .sp-tone-5 { background: #c6262e; }
            .sp-tone-6 { background: #a56de2; } .sp-tone-7 { background: #715344; }
            """);
        Gtk.StyleContext.add_provider_for_display (
            display,
            provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
        );
        message_styles_installed = true;
    }

    // Up to two initials, from the first and last word of a display name.
    public static string initials (string name) {
        string first = "", last = "";
        uint count = 0;
        foreach (string word in name.strip ().split (" ")) {
            if (word == "") { continue; }
            unichar c = word.get_char ();
            if (!c.isalnum ()) { continue; }
            count++;
            if (first == "") { first = c.to_string ().up (); }
            last = c.to_string ().up ();
        }
        if (first == "") { return "#"; }
        return count > 1 ? first + last : first;
    }

    // A stable colour class per identity so a person keeps the same colour everywhere.
    public static uint tone (string identity) {
        return str_hash (identity) % 8;
    }

    // A round avatar: the photo when one exists, otherwise coloured initials.
    public static Gtk.Widget avatar (string identity, string name, int size, Gdk.Paintable? photo = null) {
        var holder = new Gtk.Overlay () {
            width_request = size, height_request = size, halign = Gtk.Align.CENTER, valign = Gtk.Align.CENTER
        };
        holder.set_data<string> ("avatar-holder", identity);
        holder.add_css_class ("sp-avatar");
        holder.overflow = Gtk.Overflow.HIDDEN;
        holder.add_css_class ("sp-tone-%u".printf (tone (identity)));
        var label = new Gtk.Label (initials (name)) { halign = Gtk.Align.CENTER, valign = Gtk.Align.CENTER };
        holder.set_child (label);
        if (photo != null) {
            var picture = new Gtk.Picture.for_paintable (photo) { content_fit = Gtk.ContentFit.COVER, can_shrink = true };
            picture.add_css_class ("sp-photo");
            holder.add_overlay (picture);
        }
        holder.update_property (Gtk.AccessibleProperty.LABEL, name, -1);
        return holder;
    }
}
}
