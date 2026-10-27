# Usage observation semantics (candidate)

`msdk::usage` supplies initial Rust data semantics for a future reusable usage
monitor. It does not provide collectors, authentication, a serialization
protocol, a status-area host, or native presentation. Those are still required
before the full status module can be claimed complete.

`Scope` separates provider, source and optional account. A provider aggregate
has no account; a remote server has its own opaque source identity. Callers
must supply opaque keys rather than credentials or private paths. Debug output
omits those keys, but callers must still keep sensitive data out of their own
logs and UI.

`Quota` requires explicit used/remaining basis points in 0–10,000 and a source
observation timestamp. Its conversion preserves the distinction between used
and remaining; a reset at or before the current time marks a fresh window
expired. Unknown observations must remain absent rather than being constructed
as zero. `State` distinguishes loading, empty, unknown, fresh, stale,
unavailable, failed, unsupported and expired; the caller owns transitions and
must not present stale or expired data as current.

`Tokens::total` adds known input and output with overflow checking; missing
components stay unknown. Cached input and reasoning are subsets and are not
added again. `subsets_valid` detects contradictions where a known subset
exceeds its known parent. These helpers do not merge observations, establish
provider coverage, deduplicate overlapping local/server data, or infer source
semantics. Adapters must provide those guarantees from documented contracts.

The in-memory `Snapshot` includes explicit complete/partial/unknown coverage
and groups observations by distinct scope. `validate()` rejects empty identity
keys, duplicate scopes or window keys, a fresh observation without a source
timestamp, and contradictory known token subsets. A window has its own quota
observation time; it need not equal the containing observation's timestamp.
Validation is explicit: callers must invoke it before presenting a snapshot.
This versioned Rust structure is not a versioned wire protocol and cannot by
itself prove that the collector actually covered all configured accounts.

Each observation carries a human-readable provider label and, for account
scope, an account label separate from opaque identity keys. `selected` is
presentation context and does not filter other accounts. `Snapshot::groups()`
validates first, then preserves first-seen provider/source and account order;
provider aggregates remain separate from account observations. It does not
merge overlapping sources or implement native widgets. Provider labels must
agree within each provider/source group. Labels may contain private information;
consumers must not log them indiscriminately.

`msdk_gtk::usage::render` is an initial read-only GTK presentation of a
validated snapshot, grouping provider/source labels, separate account rows and
quota windows. It marks stale and expired observations and does not collect or
refresh data. Callers supply a human-readable source label separately from the
opaque source key. This initial widget has not been visually reviewed against
CodexBar and does not implement status-area placement, popup interactions,
refresh, provider marks, localization, or focus/scroll preservation. It is not
an accepted implementation of the complete status-area specification.

`msdk_gtk::usage::UsagePanel` is a candidate bounded read-only viewport. Its
`update` builds and validates replacement content before changing the displayed
snapshot, then restores the prior numeric vertical scroll offset after layout,
clamped for shorter content. It does not preserve a semantic row anchor, focus
in interactive children, or provide refresh, subscriptions or host placement.
The caller must apply context freshness and stop its own reads.

`msdk_gtk::codexbar::decode_snapshot` is a candidate read-only decoder for
CodexBar's version 1 `--snapshot` JSON. It accepts bounded bytes and an explicit
clock/freshness policy, produces provider aggregate observations, and marks
coverage partial because this format alone does not enumerate all accounts. It
requires source timestamps for live values, rejects malformed fields and unknown
schema versions, keeps null quotas unknown, and suppresses source payloads from
decode errors. It neither runs the command nor polls or authenticates. Fresh
read and actual account-specific adapters remain separate work; no real provider
or account qualification is implied by synthetic fixtures.

`msdk_gtk::codexbar::read_snapshot_scoped` performs one bounded `--snapshot`
request using a caller-supplied trusted absolute executable and literal argv;
it does not use a shell, schedule polling, start a daemon, or manage the existing
collector. Hold the returned process; context expiry cancels owned work. Forward
the scoped result to the state-owner thread and call `into_current` immediately
before applying it. The caller controls retries and must keep freshness current
as time passes. The result carries partial aggregate-only coverage. Diagnostic
errors omit executable paths, output and private payloads. Because the shared
process API decodes output lossily, this adapter conservatively rejects a Unicode
replacement character rather than accepting potentially corrupted JSON.

`UsagePanel::update_scoped` consumes a `read_snapshot_scoped` reply on the
GTK/state-owner thread. It ignores expired replies after any worker/UI queue.
On a current read failure it retains the last snapshot, marks previously fresh
observations stale, and shows a generic failure notice; a later successful
snapshot replaces that state. This is opt-in: the caller still owns the
process, context transitions, retry policy and lifetime. It does not install a
status host or schedule reads.

`msdk_gtk::codexbar_monitor::CodexBarMonitor` is an on-demand candidate
controller for a `UsagePanel`. The host opens and closes it explicitly; opening
alone does not read. `refresh` starts at most one bounded request and coalesces
duplicates until the reply reaches the GTK thread. Closing cancels the owned
process, invalidates queued results and marks retained fresh observations stale.
The callback reports idle, refreshing, updated and failed states for a host to
present. Source age is rechecked at final delivery; the controller does not
schedule polling, install status controls, authenticate or enumerate accounts.

`msdk_gtk::usage_control::UsageControl` wraps the candidate viewport and
on-demand monitor with a refresh button, spinner and status. The host calls
`open` when its popup is shown and `close` when dismissed; opening requests one
snapshot, repeated opens do not duplicate it, and unmapping or dropping the
control cancels owned work. It does not install or select a desktop host,
schedule background polling, manage accounts, or implement provider actions.

`msdk_gtk::usage_anchor::UsageAnchor` is a candidate GTK anchor for embedding
by a desktop status host. It opens on hover without an intentional focus grab,
pins on activation, and owns a UsageControl and its scoped read lifecycle.
The caller chooses popup position and must place the anchor in the appropriate
profile-owned status area. It does not install a Wingpanel or Windows taskbar
adapter, replace existing controls, or qualify either host. Hover-to-click,
outside dismissal, Escape, keyboard focus and teardown require native testing.
