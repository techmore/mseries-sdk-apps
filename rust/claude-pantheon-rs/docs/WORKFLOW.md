# The workflow: keep every agent window full, on work that matters (2026-10-21)

Goal stated by the owner: maximize token spend across all subscriptions, run continuously up to
each provider's 5-hour and weekly limits, and spend it in this order:
1. **Production-grade builds of target repos** (release pipelines, CI green, packaging, installers)
2. **Priority features** (the roadmap items the owner marks)
3. **Repo maintenance** (stale branches/worktrees, dependency bumps, hygiene docs, flaky tests)

## The backlog is GitHub Issues
One pinned set of repos. Every unit of work is an issue with two labels:
- a lane: `lane:build`, `lane:feature`, `lane:maintenance`
- a priority: `p0` … `p3`
plus `agent:ready` when it is specified well enough for an agent to take unattended, and
`agent:running` / `agent:done` set by the runner. An issue is the hand-off document: the agent's
thread links to it, the PR closes it, and the closing comment is the handoff summary
(see "Hand off to repo" in INTEGRATIONS.md).

## The runner lives in the T3 server
T3 already has the primitives (contracts in `~/claude-work/repos/t3code/packages/contracts/src`):
- **Usage windows** (`providerUsageLimits.ts`): per provider instance, `windows[]` with
  `kind: session | weekly | monthly`, `usedPercent`, `resetsAt`, `windowDurationMins`.
  `session` is the 5-hour window. Also `resetCredits` (Codex-style reset credits) with a
  consume RPC.
- **Scheduled tasks** (`scheduledTask.ts`): a stored `prompt`, a `provider`/runtime mode, and a
  trigger of `interval` (≥ 60 s), `fixed_time`, or `webhook` with signature verification and
  payload filters; prompts template `{{body.path}}`.
- **Automation webhooks** reach the environment through T3 Connect or Tailscale.

The runner is one scheduled task per pinned repo, interval 5 minutes, prompt:
"List open issues labeled agent:ready in <repo>, highest priority first, lanes in the order
build > feature > maintenance. Take the first one not labeled agent:running, label it
agent:running, work it to a PR that closes it, run the repo's checks, then label agent:done."
Plus a webhook task per repo on GitHub `issues.labeled` (filter `body.label.name == "agent:ready"`)
so a newly readied issue starts within seconds instead of at the next tick.

## Provider rotation by capacity
Before each task run the client (or a small scheduler thread in the server, if we add one)
picks the provider instance with the lowest `session.usedPercent` whose window is not exhausted,
tie-break by weekly. Order of preference when all are open: Claude Code (best on this codebase),
Codex, Grok, OpenCode. When a provider's session window passes ~90 %, new tasks go elsewhere and
its running threads finish. When every window is full, the runner idles until the earliest
`resetsAt`. This is what "endless usage to the 5-hour limits" means in practice: the windows are
staggered, so there is always one open.

## What the native client shows
- **Repos** page: pinned repos, each with lane counts and the next issue up.
- **Issues** panel per repo: live list (GitHub API through the server's token or `gh`),
  filters by lane/priority/agent label, "Start on this" which creates a thread with the chosen
  provider and labels the issue; issue comments stream into the thread view.
- **Usage** page (milestone 3): the windows above as gauges with reset countdowns, per account,
  and the rotation decision ("next task → Codex, Claude at 94 % until 21:40").
- **Wingpanel indicator**: running tasks, windows nearest exhaustion, pending permission prompts.

## Guardrails
- Unattended runs use the provider's supervised mode with the repo's `AGENTS.md`/`CLAUDE.md`;
  full-access only for lanes the owner marks. Every run ends in a PR, never a push to main.
- One running agent per repo at a time, unless the issue is labeled `parallel-ok`.
- Production builds (`lane:build`) must pass the repo's release workflow before `agent:done`.
- A task that fails twice gets `agent:blocked` and a comment with the error; the runner skips it.
