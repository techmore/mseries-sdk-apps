/* Synthetic neighboring indicators for an isolated shared-menu integration test. */
namespace MSeries {
    public class CodexBarIndicator : GLib.Object {
        private Gtk.Box widget = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
        public CodexBarIndicator (bool on_demand = false) { }
        public Gtk.Widget get_widget () { return widget; }
        public void opened () { }
        public void closed () { }
    }
    public class RepoBarIndicator : GLib.Object {
        private Gtk.Box widget = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
        public RepoBarIndicator (bool on_demand = false) { }
        public Gtk.Widget get_widget () { return widget; }
        public void opened () { }
        public void closed () { }
    }
}
