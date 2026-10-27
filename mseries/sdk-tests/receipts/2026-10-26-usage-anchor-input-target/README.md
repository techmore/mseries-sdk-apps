# Candidate usage anchor: input target

Hosted ARM run 40415441480 at commit 4570a09 failed pointer travel. XQueryPointer reported the popup window itself as the root child under the pointer (0x20001e), while GTK motion events reported a popup leave and the controller contained-pointer state was false. The subsequent candidate change checks the GDK surface under the device when the hover-close timer fires. This change requires native validation. Synthetic data only.
