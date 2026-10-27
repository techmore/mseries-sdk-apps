# Candidate usage anchor: pointer travel failure

Hosted ARM SDK run 40412642984 at commit 98867b8 opened the synthetic popup using GTK-reported anchor bounds and recorded one synthetic provider read. The popup was no longer visible after moving the pointer toward the GTK-reported popup center. This does not establish whether the move targeted the actual X11 popup surface or whether the popup incorrectly closed during valid pointer travel. Pinning, outside dismissal, keyboard and package steps were not reached. The receipt contains synthetic data only.
