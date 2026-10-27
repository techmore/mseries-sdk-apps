# Usage control keyboard and accessibility — 2026-10-26

Hosted ARM SDK developer-path run 40405964283 passed on 477234a. In a private
X11/D-Bus session, the synthetic app exposed `Refresh usage` with role `button`;
Tab focused it, Space triggered a failed refresh, stale data remained visible,
private stderr stayed out of the accessible tree, and Space retried successfully.
Ctrl+Q closed cleanly. The synthetic binary hash and checks are in
usage-controls.json. No real accounts or owner desktop were used.

Four 480×500 light/dark Sans 10/15 failure-state captures were reviewed. The
explicit button label, failure notice and retry prompt fit without clipping in
this fixture. Keyboard scrolling, focus/hover/dismissal in real status hosts,
long translated labels, real collector reads and owner visual acceptance remain
open. Earlier X-display and accessible-name failures are retained separately.
