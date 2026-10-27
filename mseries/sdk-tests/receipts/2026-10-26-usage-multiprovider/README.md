# Multi-provider usage fixture — 2026-10-26

Hosted ARM Linux developer-path run 40396405090 passed on commit 2111eb1,
including the native multi-account/widget/scroll-range test and eight synthetic
480×500 light/dark, Sans 10/15 top/end screenshots. Existing CodexBar icon
assets were staged in an isolated XDG data directory. The fixture contains two
Codex accounts, two Claude accounts, one Grok and one OpenCode account; no
real accounts or live provider adapters were used.

Review: the captured top and bottom show the labels, distinct accounts,
provider marks, partial coverage and unknown/unavailable states. The bottom is
reachable by a programmatic scroll. At larger text sizes only two providers are
visible at opening, so provider coverage is not sufficiently obvious as the
status specification requires. The scroll boundary clips partial cards, as
expected for this ordinary viewport; no keyboard scrolling, interactive host,
owner acceptance or complete visual conformance is established.
