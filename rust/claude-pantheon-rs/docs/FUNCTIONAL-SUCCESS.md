# Functional success: Project → Issue → coding run (design, 2026-10-27)

Owner's ask: every coding run starts from an issue that says only what works when it is done,
in terms of what a person can do or see. The agent works backwards from that: success → checks it can
run → plan → code. This builds on [WORKFLOW.md](WORKFLOW.md) (GitHub Issues backlog, runner as T3
scheduled tasks, provider rotation). It adds no new backlog, server or store.

## 1. The issue template

Each pinned repo gets `.github/ISSUE_TEMPLATE/agent-task.yml`. It renders as an issue body with
fixed `###` headings, which the client parses.

```yaml
name: Agent task
description: Say what works when this is done. The agent works out how.
labels: ["agent:ready"]
body:
  - type: textarea
    id: success
    attributes:
      label: Success looks like
      description: |
        One line per outcome, written as what a person can do or see.
        "I can … and I see …". No file names, functions or libraries.
      placeholder: |
        - I open Repos, click an issue and see its success lines above the Start button.
        - An issue without this section shows "No success section" and Start is disabled.
    validations: { required: true }
  - type: textarea
    id: where
    attributes:
      label: Where to try it
      description: The app, page, command or URL where a person would check. Optional.
  - type: textarea
    id: out
    attributes:
      label: Not part of this
      description: Things a reader might expect that this issue does not cover. Optional.
  - type: dropdown
    id: lane
    attributes: { label: Lane, options: [build, feature, maintenance] }
```

Rules:
- **Success lines are the contract.** Each one is a single observable outcome. If it can't be
  observed (for example "the code is cleaner"), it belongs in `lane:maintenance` with a check the
  owner names, or it is not an agent task.
- **No how.** The issue doesn't name files, functions or approaches. The agent finds those.
- An issue without a non-empty **Success looks like** section is not `agent:ready`. The client
  won't start it, and the runner skips it and comments once asking for one.
- Issues written by hand work too, as long as they have a `### Success looks like` (or
  `## Success looks like`) heading followed by a bullet list.

## 2. From success lines to acceptance checks, before any code

A run has two phases in one T3 thread.

**Phase 1: work backwards (plan mode).** The thread launches with `interactionMode: "plan"`. The
first message gives the agent the issue and these instructions:

1. For each success line, write one **acceptance check**: the cheapest thing that runs in this
   repo and proves that line, such as a unit or integration test, a CLI invocation with an expected
   output, or a scripted UI drive that asserts on a screenshot or on-screen text (using the
   repo's own screenshot tooling where it has one).
2. Run each check now. **It must fail** (or not exist yet). A check that already passes means
   either the line is already true, so the agent says so, or the check doesn't test the line.
3. A line that no command can prove becomes a **manual check**: the exact steps a person takes and
   what they should see. These are allowed, but they are listed separately and the issue stays open
   for the owner to tick them.
4. Only then, write the implementation plan: the files and steps, ordered so each step turns at
   least one check from failing to passing.

The agent returns this as its proposed plan (T3's `proposed_plan` artifact) with a fixed table at
the top:

| # | Success line | Check (command or steps) | Kind | Now |
|---|---|---|---|---|
| 1 | I can … | `cargo test success_lines_parse` | auto | fails |
| 2 | I see … | Open Repos → … → expect "…" | manual | n/a |

The client posts the table to the issue as a comment starting `<!-- acceptance -->`. Each run
edits that one comment rather than adding new ones, so the issue always shows the current checks.

**Phase 2: build to green.** Implementing is T3's own "implement this plan" path:
`message.dispatch` with `sourcePlanRef {threadId, planId}` and interaction mode back to `default`.
The agent commits the failing checks first, then the code. The run is done when every `auto`
check passes plus the repo's normal checks. The PR description repeats the table with a final
**Now** column (`passes` / `manual: steps above`) and closes the issue only if there are no manual
lines (otherwise `Refs #n`).

**Unattended runs** (the runner and webhook tasks) do both phases in one go: no human approves the
plan. The acceptance comment is posted before any code commit, so the owner can see and veto what
"done" was taken to mean. If phase 1 can't produce at least one `auto` check, the run stops with
`agent:blocked` and a comment saying which lines couldn't be checked.

## 3. Where it lives

### In T3 (use what exists, add nothing to the server)

| Need | T3 already has | Use |
|---|---|---|
| Repo | `Project` (`workspaceRoot`) | unchanged, via `project_for_path` |
| One run per issue | `orchestration.launchThread` | as today, plus `interactionMode: "plan"` |
| Work backwards first | plan mode, `proposed_plan` artifact, `plan.updated` | phase 1 output |
| Go build it | `message.dispatch` + `sourcePlanRef` | phase 2 |
| Progress | `todo_list` artifact (steps with status) | shown as is |
| Repo's own checks | `Project.scripts[]` with `icon: "test"` | named in the prompt as the baseline check |
| Unattended | `ScheduledTask` (interval / webhook) | runner prompt gains the two phases |
| PR | `thread.pull-request.link` | as today |

What T3 does **not** have, and where it lives instead:
- **Success criteria / acceptance checks**: no field anywhere. They live in the GitHub issue (the
  success lines) and in the repo (the committed checks), and the acceptance comment ties them
  together. T3's `goal` is provider-owned and read-only (set only via `/goal`, judged by the
  provider), so we don't build on it. For Codex threads the client may also send
  `/goal <first success line>` as a bonus, but nothing depends on it.
- **Issue ↔ thread link**: no field. It stays in the client's `workflow.json` (`started`) and in the
  thread title `#n …`, as now. (`repos::issue_goal` is unused today and can go.)
- **Gating on check results**: T3 records exit codes only for setup scripts and terminals. The
  agent runs the checks itself and reports them in the table. The client doesn't re-run them.

### In the native client (Repos → Issues panel)

- **Issue row**: under the title, the success lines (first three, then "+n"). If the section is
  missing, a "No success section" pill appears and **Start on this** is disabled with a tooltip
  linking to the template.
- **Start on this**: launches in plan mode with the phase-1 prompt (replaces today's
  `issue_prompt`).
- **Thread opened from an issue**: a pinned card at the top (next to the existing comments row)
  lists the success lines as a checklist. A line's state comes from the latest acceptance table
  (`fails` → empty, `passes` → ticked, `manual` → hand icon). When a `proposed_plan` arrives, the
  card's button reads **Build to green** and dispatches with `sourcePlanRef`.
- **Repos page card**: the "next issue up" line shows its success-line count. Issues without
  success lines count toward a "needs success" figure instead of the lane counts.

## 4. Smallest first slice

One PR in `rust/claude-pantheon-rs` plus the template file. No server change, no runner change, no
thread card.

1. `.github/ISSUE_TEMPLATE/agent-task.yml` as above, in this repo first.
2. `workflow::success_lines(body) -> Vec<String>`: parses the `Success looks like` section (both
   heading levels, `-`/`*` bullets, stops at the next heading), with unit tests.
3. The Issues row shows the success lines or the "No success section" pill, and **Start on this**
   is disabled without them.
4. **Start on this** launches with `interactionMode: "plan"` and the phase-1 prompt (the agent
   writes the acceptance table and stops). The owner presses the composer's existing send to
   implement.

Success looks like (for the slice itself):
- I open an issue made from the template in Repos and see its success lines in the row.
- An issue with no success section shows "No success section" and I can't start it.
- I press Start on this and the thread's first reply is a table with one check per success line,
  each marked failing, and no code has changed.

Next slices, in order: the acceptance comment on the issue → **Build to green** button and
checklist card → runner and webhook prompts get both phases and the `agent:blocked` rule → the
template rolled out to every pinned repo.
