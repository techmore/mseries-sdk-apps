# s04 Usage dashboard (medium)

Golden reference: the CodexBar-style usage gauges in `rust/claude-pantheon-rs/src/usage.rs` and `mseries/codexbar-wingpanel/`.

A window of provider cards (Claude, Codex, OpenCode). Each card: name, plan, two gauges (session and
weekly, percent used with reset time), and a sparkline of the last 24 h. Data comes from the JSON that
`codexbar-linux --snapshot` prints, refreshed every 60 s and on demand. A card turns amber at 80 % and
red at 95 %, and also says so in text.

Capabilities: subprocess (`codexbar-linux`), timers.

Acceptance (fixture JSON supplied):
1. One card per provider in the JSON, in a stable order; an unknown provider still renders.
2. Gauges show the number and a text state ("OK", "High", "Critical"), not colour alone.
3. Refresh updates in place without resetting scroll or focus.
4. A failing command leaves the last good data, marks the card "Stale · 3 min" and shows a retry.
5. Resizes from 480 px to wide: cards reflow from one column to a grid; no clipping at 1.5x text zoom.
