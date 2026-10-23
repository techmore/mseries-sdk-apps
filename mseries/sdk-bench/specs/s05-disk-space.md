# s05 Disk Space (hard: custom drawing, background scan)

Golden reference: `rust/mseries-diskspace`.

A disk usage analyzer. Pick a folder (portal file chooser); scan it in the background with progress;
show the largest children as cards and a circle (sunburst) chart. Clicking a card or segment drills
down; a breadcrumb goes back. "Open in Files" and "Move to Trash" per item (trash asks first).

Capabilities: filesystem (read user-chosen folders), portal file chooser, trash.

Acceptance (a fixture tree is supplied):
1. Scan shows incremental progress and never blocks the window.
2. Cards and chart agree on sizes and ordering; hover on one highlights the other.
3. Drill-down and breadcrumb work with mouse and keyboard; the chart has a text equivalent list.
4. Trash asks, then removes and rescans just that subtree.
5. Scanning 200 000 files keeps the UI responsive (frame time budget stated in the run notes).
6. Cancel stops the scan within 1 s.

Note: needs a custom-drawn chart. Counts as a `Custom` use unless the SDK ships a sunburst; record it in `GAPS.md`.
