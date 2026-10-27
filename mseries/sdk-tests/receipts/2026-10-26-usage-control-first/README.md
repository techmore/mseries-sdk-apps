# Usage control initial failure-state review — 2026-10-26

Hosted ARM SDK run 40402684685 passed on a5b0ff2. Native synthetic tests cover
one read per open, busy/disabled refresh, malformed output, stale retained
values, retry and no private stderr in labels. Four 480×500 light/dark Sans
10/15 synthetic failure screenshots are retained here; project icons were
staged only in a private fixture directory.

Review: the failure notice, stale value and refresh button are visible and
readable in both themes/text sizes. The same failure appears in both the panel
and footer, so the footer is being revised into a recovery prompt. The captures
are not owner visual acceptance, keyboard/AT-SPI interaction, real collector or
desktop status-host evidence.
