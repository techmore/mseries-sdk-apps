namespace SignalNative {
    public class MessageComposer : Gtk.TextView {
        public string placeholder_text { get; set; default = "Write a message"; }
        public string text {
            owned get { return buffer.text; }
            set { buffer.text = value; }
        }
        public signal void changed ();
        public signal void submitted ();

        public MessageComposer () {
            Object (wrap_mode: Gtk.WrapMode.WORD_CHAR, accepts_tab: false,
                top_margin: 8, bottom_margin: 8, left_margin: 8, right_margin: 8,
                tooltip_text: "Write a message. Enter sends; Shift+Enter inserts a new line.");
            buffer.changed.connect (() => changed ());
            var keys = new Gtk.EventControllerKey ();
            keys.key_pressed.connect ((key, code, state) => {
                if ((key == Gdk.Key.Return || key == Gdk.Key.KP_Enter) &&
                    (state & Gdk.ModifierType.SHIFT_MASK) == 0) {
                    submitted ();
                    return true;
                }
                return false;
            });
            add_controller (keys);
        }
    }
}
