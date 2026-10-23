# Building Pantheon apps: product feedback and build loop

Read this before building or porting an app with the Rust Pantheon SDK. This
workflow incorporates the owner's Signal/Messages feedback from 2026-10-22.
A launcher with tabs was delivered without a convincing integrated experience;
substantial supporting work and repeated release builds delayed useful feedback.
The owner rejected the result. Both apps are incomplete and deferred.

## First establish what the person will experience

Write a short outcome contract before implementation:

- What opens, which window owns navigation, and what each primary action does.
- Whether integration means one conversation window, switching existing windows,
  or a launcher. Show the actual behavior; these are different deliverables.
- The first useful user journey and the visible conditions for completion.
- Existing client/service boundaries and any limitation that changes that journey.

Use the user's existing decisions. Clarify only material unresolved choices;
continue independent work while waiting. Do not substitute infrastructure or a
passing fixture for the requested experience.

## Show a running example early

Target a real, runnable screen within about 30 minutes, using a fixture backend
when needed. This is a feedback target, not a promise that every project fits it.
If setup prevents it, report the exact blocker and elapsed time at that point.
Show the screen and primary interaction, with larger text and compact sizing,
before investing heavily in supporting systems. Identify whether the evidence
comes from the target desktop or an isolated display.

At roughly 45-minute work checkpoints, report the running result, elapsed time,
remaining product work and blockers. Explain any proposed scope expansion and
its effect on the requested outcome. Keep unrelated SDK migrations, localization
expansion, and other app ports outside the current delivery unless requested or
necessary. Do not add recurring notifications or timers just to enforce this rule.

## Reuse a template; extract the SDK from demonstrated needs

Inspect the closest existing app and its actual visual quality. T3 is a reference
implementation, not proof that every surface is approved or an app generator
exists. Prefer an already working app shell and fixture backend. Establish the
requested experience, then extract repeated behavior into shared SDK helpers.

The desired maintained starter should provide:

- Native window chrome, rounding ownership, typography, spacing and scaling.
- Navigation, visible focus, accessible controls and loading/error/empty states.
- Logging, actionable errors, settings, optional polling and dock badges.
- A fixture backend and verified create, run and package instructions.

A future generator must produce an immediately runnable, useful app with these
patterns demonstrated. Record missing generator/API support as a concrete gap;
do not invent commands or claim a scaffold exists. Avoid a new description
language until a measured repeated need justifies its maintenance cost.

## Keep development builds fast

- Use incremental debug builds and run the affected app from its build output.
- Use focused checks for the changed crate and affected behavior. Run/add tests
  when requested or required by the applicable task instructions.
- Preserve a shared Cargo cache and stable compiler, target and feature settings.
  Do not clean caches, rebuild dependencies, or launch competing redundant builds
  as a routine troubleshooting step.
- Build an optimized package at a release/checkpoint boundary when needed for
  deployment or performance measurement, not after every small UI/docs change.
- Separate timings for dependency acquisition, compilation, linking, testing,
  packaging, installation and CI queue time. Reuse valid receipts when inputs
  are unchanged and state which revision each receipt covers.
- If an incremental build unexpectedly takes minutes, inspect its rebuild cause
  before repeating it. Report timing evidence and a remedy; do not call all
  elapsed task time compilation time.

The owner reported roughly seven hours spent compiling during this effort. That
number has not been established by a retained timing breakdown. Repeated optimized
rebuilds were wasteful; diagnose measured costs before proposing remedies.

## Completion is a product claim

A build proves compilation. A fixture proves only the behavior it exercises.
Neither proves visual quality, desktop integration, live delivery or owner
acceptance. Report separately: implemented, built, installed, loaded, visually
reviewed, and accepted. Give explicit remaining gates instead of unsupported
completion percentages. Distinguish the new app from the qualified fallback.
Honor stop/defer instructions; preserve work and leave the app marked incomplete.

## Reusable app-building prompt

> Read AGENTS.md, mseries/RUST-PANTHEON-SDK.md and this guide. Build [app/user
> journey] using [existing reference]. State the visible outcome and integration
> behavior first. Show a runnable primary flow within about 30 minutes, or report
> the concrete blocker at that checkpoint. Use a fixture backend if necessary.
> Review the actual UI early; keep progress updates centered on the user journey.
> Use incremental debug builds, a shared cache and focused checks; avoid repeated
> optimized packaging. At roughly 45-minute checkpoints report elapsed time,
> running evidence, remaining work and any scope change. Implement the requested
> app, extract demonstrated shared patterns, and qualify a release afterward.
> Do not claim completion from compilation or fixtures alone. Preserve existing
> clients/services and clearly distinguish installed code from loaded behavior.
