# Messages client parity audit

Source review: 2026-10-22. Baseline: `phone-setup/messages.py` and its model,
media and widget modules. Candidate: `rust/mseries-messages` and the shared SDK.
This is an implementation inventory, not proof of live service behavior.

| Behavior | Rust implementation/evidence | Remaining difference or acceptance |
| --- | --- | --- |
| Service unavailable, reconnect and retry | Typed public Messages1 API, isolated GTK recovery scenario | Actual owner service |
| Recent inbox and history | ListRecent merge, periodic/reconnect/resume refresh | Live phone history completeness |
| Search and contact picker | Unicode-folded conversation/body search; public FindContacts | Live contact matches and keyboard order |
| Direct/group send | Checked Send, participant confirmation, bounded optimistic outbox | Owner-approved live send and group acceptance |
| Drafts and phone changes | Per-phone in-memory draft/outbox states and generation guards; A/B/A regression | Live switch; unknown identity remains restricted |
| Read/favorites | Read API; shared local favorites, imported overrides, ordered strip and row/header actions | Sidebar refresh reselects the open conversation; automatic read checks selection and window activation, requires active window and coalesces per newest-message/read-count marker, with failed-request retry; live read state remains unqualified |
| Reply/copy | Quoted reply state, cancellation, clipboard and recipient copying | Keyboard/screen-reader interaction |
| Tapbacks | SDK folding and removal/change tests | Phone-specific examples |
| Media | Bounded public HTTPS image/GIF/WebP/YouTube previews; no private MAP attachment retrieval | Both clients depend on backend limits; live preview states |
| Photos | Opt-in API, bounded decode, revision and phone cache invalidation | Live photos |
| Pins | Stable UUID scope and shared Secret Service item; v1/v2 local-pin/hidden-import storage round-trip and order supported | Released BlueFerry lacks merged UUID field; Rust local-pin editing/strip implemented; private encrypted Rust/Python and GTK save/reopen/remove persistence verified; live acceptance remains |
| Navigation | Compact/regular resize; search shortcuts; Back draft retention | Refresh preserves focused inbox row by thread identity and reading position, follows when near bottom and offers Latest Messages; isolated sidebar/viewport regression passes |
| Scaling/theme | Elementary light/dark, long-text and large-font isolated GTK passes | Hardware scales and assistive technology |
| Localization | App gettext domain, 87 static source strings extracted, including accessible names/tooltips; English fallback and local calendar timestamps | Dynamic/plural strings and dynamic accessibility names and reviewed translated catalogs remain; gettext/template foundation is installed |
| Packaging | ARM64 package and user-level preview install | Full release CI and live acceptance; Python remains qualified |

Do not replace the qualified launcher or mark parity complete based on this table.
The current preview is available separately as Messages (Rust Preview).
