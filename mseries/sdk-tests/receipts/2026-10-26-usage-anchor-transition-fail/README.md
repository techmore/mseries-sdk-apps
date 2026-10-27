# Candidate usage anchor: transition diagnostic

Hosted ARM SDK run 40414571505 attempt 2 at commit 8465596 failed pointer travel. The popup surface remained at 291,335 (278x123) before and after moving to 430,396. GTK logged popup enter, popup leave, and a close timer with both motion controllers reporting false. This did not establish which X11 window received the pointer. Pinning was not reached. Attempt 1 failed earlier because its private GTK display could not be opened. Synthetic data only.
