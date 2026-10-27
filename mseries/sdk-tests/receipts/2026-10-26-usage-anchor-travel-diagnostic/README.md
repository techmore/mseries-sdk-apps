# Candidate usage anchor: pointer travel diagnostic

Hosted ARM SDK run 40413041565 at commit c7e6cdb opened the synthetic popup and read once. X11 reported the popup at 291,335 with size 278x123; the pointer was moved to 430,396, inside that surface. The popup nevertheless closed before the probe completed its travel check. Pinning, outside dismissal and keyboard checks were not reached. A subsequent candidate change checks pointer containment when the close timer fires; that change still needs native validation. Synthetic data only.
