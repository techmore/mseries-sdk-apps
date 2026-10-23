# Baseline: what we know about current app-building cost

Written 2026-10-22. Short version: the **12 hours to reach 50 %** figure is the owner's experience
and is **not recoverable from git history**. This file records what history does and does not show,
so Phase 0 measures instead of guessing.

## What git history shows (origin/dev)

| App | Commits | First to last on `dev` | Note |
|---|---:|---|---|
| `rust/claude-pantheon-rs` | 40 | 10-21 20:51 to 10-22 01:46 (~5 h) | Preceded by a ~1 h spike in a standalone checkout (10-21 19:55 start, 20:00 backend decision). Feature-rich within ~6 h end to end, with the T3 server as a given backend |
| `rust/mseries-diskspace` | 1 (+1 housekeeping) | single commit 10-21 17:33 | Squashed in; the work time is not visible |
| `rust/mseries-updates` + `-core` | 1 (+1 housekeeping) | single commit 10-21 16:27 | Squashed in |
| `mseries/claude-pantheon-native` (Vala twin) | 1 | 10-21 20:36 | Squashed in |
| `mseries/quick-settings` | 16 | 10-12 to 10-21 | Plugin work spread over nine days alongside other tasks |

Findings:

1. A large Rust/Granite app can be built fast (the Claude client: ~6 h). So the cost is not
   uniform; it is high for some apps and some kinds of work. We don't yet know which.
2. Squashed commits and stacked WIP branches hide real agent time. Commit timestamps measure
   integration, not effort.
3. The standalone `claude-pantheon-rs` and `claude-pantheon` checkouts have only 10 and 4 commits.
   They also don't contain the working time.

## What would show it

Per-session facts live in the agent session logs (Claude Code transcripts under
`~/.claude/projects`, Codex and OpenCode logs), not in git. Phase 0 therefore does two things:

1. **Retro-measure two past apps** from session transcripts: wall-clock from the first prompt to
   the first window that opened, to the first state that passes half the acceptance lines, and to
   done; plus tokens, retries and human interventions. Candidates: Disk Space and Software Updates
   (both squashed, so history alone says nothing). Record the transcript ids used.
2. **Run specs s01 and s03 fresh** with the current process on a frontier agent, using the rubric,
   to get a controlled "before" that doesn't depend on memory.

Exit: a table here with real numbers and transcript or run ids. Until then, treat every "hours"
figure in [PLATFORM-PLAN.md](../PLATFORM-PLAN.md) as the owner's report, and every "minutes" figure
as a proposed target.

## Results table (fill in)

| Run | Spec | Mode | Model | t_first_window | t_done | Done-bar items met | Interventions | Notes |
|---|---|---|---|---|---|---|---|---|
| | | | | | | | | |
