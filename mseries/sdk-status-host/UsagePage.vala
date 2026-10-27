/* SPDX-License-Identifier: GPL-3.0-only
 * SDK usage page for a host-owned popup. The host owns visibility and calls
 * opened/closed as its page or popup is selected/dismissed. */
namespace MSeries {
    public class SdkUsagePage : GLib.Object {
        private Gtk.Box root;
        private Msdk.UsageControl? control;
        private Gtk.Label unavailable;

        public SdkUsagePage (string executable) {
            root = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
            unavailable = new Gtk.Label ("Usage is unavailable") { xalign = 0, wrap = true, visible = false };
            control = new Msdk.UsageControl (executable);
            if (control != null) root.append (control.widget ());
            else unavailable.visible = true;
            root.append (unavailable);
        }

        public unowned Gtk.Widget get_widget () { return root; }

        public void opened () {
            if (control != null) unavailable.visible = !control.open ();
        }

        public void closed () {
            if (control != null) control.close ();
        }

        public override void dispose () {
            closed ();
            if (control != null) {
                root.remove (control.widget ());
                control = null;
            }
            base.dispose ();
        }
    }
}
