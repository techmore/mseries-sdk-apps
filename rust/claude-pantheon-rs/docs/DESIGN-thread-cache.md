# Design: settled-thread cache and `afterSequence` resume (2026-10-27)

Status: proposal, awaiting owner approval. Sized **Medium-Large** (about 2 days with tests); not started.

## Problem
Opening a thread, or reconnecting after a drop, re-downloads the whole projection (`orchestration.subscribeThread`
snapshot, bounded but up to several MB for long threads) and re-renders it. Long threads feel slow on a laptop
and on a relayed (Tailscale / T3 Connect) link, and a reconnect loses nothing but costs a full reload.

## Server contract (verified in contracts)
- The snapshot carries `snapshotSequence`; the contract says "resume events with `afterSequence = snapshotSequence`"
  (`orchestrationV2.ts`, subscribe input has optional `afterSequence`).
- `historyCursor` stays opaque; older history is separate (client PR "page in older thread history").

## Proposal
1. `thread_cache.rs`: per (environment id, thread id) one record `{schema, snapshotSequence, view (items, runs, checkpoints), historyCursor, hasMoreHistory}`
   in `~/.cache/claude-pantheon-rs/threads/<env>/<thread>.json.lz4`; projection and cursor written together via
   tmp-file + rename so they can never disagree.
2. Only **settled** threads (no active run, no pending request) are written, on leaving the thread and on run end.
3. Open: render the cached view at once (marked "Updating…"), subscribe with `afterSequence`; apply events;
   if the server answers with a fresh snapshot (sequence gap) replace via the existing diff path. Send, Stop and
   approvals stay disabled until the subscription's `synchronized` marker.
4. Limits: 8 MiB per record, 64 MiB / 32 entries per environment, LRU delete; unknown schema or parse error = cache miss.
5. Diagnostics page shows hits/misses (counters only, no content).

## Risks and decisions for the owner
- Needs a compression crate (`lz4_flex`, pure Rust): new dependency; alternative is plain JSON with a smaller cap.
- Cached message text sits on disk in the user cache dir (same sensitivity as `~/.claude` transcripts; mode 0600).
- Salvage source: OS draft #521 `thread_cache.rs` / `persistence.rs` (about 700 lines, 8 tests) written against the older Vala-era model; needs adapting to this tree.
- Value: medium (open/reconnect speed); it does not change correctness. Recommend doing it after live use shows slow opens.

## Also deferred (Large): full provider manager
#521's `provider_setup.rs` (1,450 lines): provider sign-in interactions and ACP registry search with generic
local-command setup. This tree ships the read-only AI Providers page only. Sign-in and third-party agent installs
act on accounts and on code the server will run, so they need the owner's go-ahead and live testing.
