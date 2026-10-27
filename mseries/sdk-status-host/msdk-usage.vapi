/* SPDX-License-Identifier: GPL-3.0-only */
[CCode (cprefix = "Msdk", lower_case_cprefix = "msdk_", cheader_filename = "msdk-usage.h")]
namespace Msdk {
    [Compact]
    [CCode (cname = "MsdkUsageControl", free_function = "msdk_usage_control_free")]
    public class UsageControl {
        [CCode (cname = "msdk_usage_control_new")]
        public UsageControl (string absolute_executable);
        [CCode (cname = "msdk_usage_control_widget")]
        public unowned Gtk.Widget widget ();
        [CCode (cname = "msdk_usage_control_open")]
        public bool open ();
        [CCode (cname = "msdk_usage_control_close")]
        public void close ();
    }
}
