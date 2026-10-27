# Accessibility probe failure, 2026-10-26

SDK developer-path run 40392328090 attempt 1, commit dbf4eee, failed before
accessibility registration. The retained synthetic process stderr says GTK
failed to open the display. The reason the display was unavailable is unknown.
The same job's attempt 2 passed the complete workflow; this does not prove the
underlying display failure cannot recur. `controls.json` is the original
synthetic test artifact from attempt 1. No owner desktop was involved.
