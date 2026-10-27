# Usage control failure state — 2026-10-26

Hosted ARM SDK developer-path run 40403452026 passed on e1c91a8. The native
synthetic test covers one read per open, busy/disabled refresh, failure with
stale retained data, and successful retry. Four 480×500 light/dark Sans 10/15
failure-state captures are retained; no real provider was accessed.

Review: failure explanation, partial coverage, stale quota, recovery prompt
and Refresh button are visible without clipping in this fixture. The footer no
longer repeats the failure text. This is not measured contrast, keyboard or
AT-SPI interaction, a desktop status host, real collector validation, or owner
visual acceptance. Busy and successful states were tested in the widget tree,
not visually reviewed in these captures.
