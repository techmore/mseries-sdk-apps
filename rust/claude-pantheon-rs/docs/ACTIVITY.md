# Native daily activity

Activity opens inside the existing T3 window. It lists factual records from the
repositories pinned on the Repositories page and can copy a Markdown recap.
It does not create another app, indicator, background service or Omi integration.

This feature is inspired by [Shiplog](https://github.com/joshuaswarren/omarchy-shiplog)
at `9becc65740f6dbe23f296651df00f2538ba57ab4` (v0.1.1). Upstream MIT attribution
is retained in `licenses/shiplog-MIT.txt`. The Quattro QML shell is not used.

## First journey

1. Pin an explicit local checkout on Repositories.
2. Choose Activity from the app menu or command palette. Refresh reads local git
   history from pinned checkout paths only; no recursive discovery is performed.
3. Optionally choose Include GitHub. This reads the authenticated `gh` account
   and searches that account's activity within pinned GitHub repositories.
4. Inspect source status, open a source link, or choose Copy recap. The clipboard
   includes source availability and completeness. Review private details before
   sharing. Nothing is automatically sent to an agent, Omi or a remote service.

## What each category means

- Local commits: non-merge commits reachable from any ref, with exact author email
  matching the checkout's configured git identity. Author timestamps determine
  today's membership. A local commit is not evidence that it was pushed.
- GitHub commits: authored commits returned by GitHub search, whose index covers
  default branches. Author timestamps determine today's membership.
- Merged PRs: PRs authored by the signed-in account, using merge timestamps.
- Closed authored issues: the account's authored issues that are closed, using
  closure timestamps. This does not claim the account performed the closure.

Local and GitHub records can overlap, so they are counted separately and have no
combined “shipped” total. The day follows local midnight and next local midnight,
including 23/25-hour daylight-saving days. A refresh after midnight uses a new
window and does not retain stale rows from the previous day.

## Limits and failure states

Collection runs off the GTK main thread. Subprocesses receive structured argument
arrays, never shell-expanded settings. Every command has a 15-second deadline,
the refresh has a 90-second command budget, and stdout is capped at 2 MiB.
Only the first 20 pinned checkout entries are considered. Git scans at most 501
reachable non-merge commits per checkout; reaching the cap is marked Partial.
This scan cap can miss older reachable commits with today's author timestamp.

GitHub search fetches at most three pages of 100 rows per category. More results
or GitHub's `incomplete_results` mark the source Partial. Failed pages never
become a successful empty result. No authentication tokens are read or copied;
gh remains responsible for its own credentials. Authentication is rechecked per
GitHub refresh. Cached GitHub rows are not reused after an unverified or changed
account. The scope of pinned repositories forms part of the source identity.

Failed sources may retain last-known rows only within the same calendar window,
source and proven identity. Source labels show availability, completeness and
observation time. Unknown counts show unavailable, not zero; a partial/stale count
is explicitly the number of records shown. Disabling Include GitHub removes its
rows on the next refresh. No activity cache or recap is persisted to disk.

## Implementation and validation

- `src/activity.rs`: pure model, structural parsing, exact identity matching,
  stable deduplication, same-day/account retention, safe factual Markdown.
- `src/activity_collect.rs`: bounded read-only git/gh transport and collectors.
- `src/activity_view.rs`: GTK page, explicit refresh/clipboard, deterministic fixture.

The dedicated model/collector tests cover real temporary git fixture collection,
deadline and nonzero-exit handling, bounded pagination and partial responses,
failure retention across accounts/days, exact email matching and recap injection.
The isolated feature render probe uses `show_fixture(fixture_report())`; this
disables live collection. `CLAUDE_PANTHEON_OFFLINE` also disables collection.

No native Omi endpoint or service contract exists in the inspected `dev` tree.
A later adapter should consume a versioned read-only API and use explicit,
scoped export; do not infer or introduce a microphone/transcript collection flow.
Live account collection, target desktop operation and owner acceptance remain
separate from fixture/model/build evidence.
