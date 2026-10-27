# Candidate GTK usage preview — 2026-10-26

Source: edf7a4ee0aec0a682c1e016a69b91efcf2576d0f; SDK developer-path
run 40395051297 passed on hosted ARM Linux. Native widget-tree test passed,
and synthetic preview captures were made at 480×500 in light/dark with the
verified Sans 10 and Sans 15 profiles. No real accounts were used.

Visual review: text is readable in both themes and the sample is not clipped,
but the component is too flat for the CodexBar reference: no visible provider
card/mark or compact account hierarchy, repeated stale labels, and awkward
wrapping at larger text. The sample does not prove long-list scrolling, source
coverage, keyboard actions, accessibility interaction, host placement or owner
approval. Do not claim status module visual or functional acceptance from this.
