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

## Use the implemented SDK authoring path

Descriptions (`.msdk` or JSON), rules and declared services are the primary
authoring path. `msdk new` already generates a description, gates and README
from six templates; it does not generate a complete Cargo project, backend,
standalone executable or package. Use app-local Rust `Services`/`Logic` only
where the supported vocabulary is insufficient; record each escape hatch and
its reason instead of silently replacing the SDK with a hand-written app.

Inspect the closest existing app and its actual visual quality. Native T3 and
the SDK T3 executable are different consumers. A native T3 feature or screenshot
does not prove SDK-port parity. The SDK provides controls/rules/hosting; T3's
transport, account mappings and GitHub services remain app-specific adapters.

Use `cargo run -p msdk -- templates` from `rust/` to discover actual starters,
then the CLI's `describe`, `check` and `gate` commands. Consult
[RUST-PANTHEON-SDK.md](RUST-PANTHEON-SDK.md) for boundaries. Do not present
proposed `msdk gen`, `msdk pack`, `msdk parity` or live introspection commands
as available. Keep native chrome, scaling, accessibility, recoverable states,
diagnostics and install/remove behavior in the delivered outcome contract.

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

## T3 priority slice and SDK qualification gates

Owner priority (2026-10-25): **token monitoring and RepoBar-style repository/
GitHub integration first**, before broader T3 chat expansion. This defines the
first qualification slice, not a reduction of #523's full-client requirements.
Use #535 for SDK readiness and #593 for GUI/Dock usability. Signal/Messages
remain deferred. Record supported operating modes explicitly; the owner's
phrase “Windows and non-Windows mode” remains unresolved until clarified.

| Gate | Observable success condition |
| --- | --- |
| Scope and identity | Enumerate each promised monitor/GitHub action and its SDK control, rule, feed or app-local adapter. Selected provider/account and paired server are distinct; repository/checkout/branch scope remains visible. No invented API or functionality implied by a manifest alone. |
| Token semantics | Show units, source, account, time window, observation time and freshness. Keep local token totals, server token history and provider quota/reset windows distinct. Unknown, failed, stale and unavailable are not zero or “ready”; never attribute one account's usage to another. |
| Usage lifecycle | Exercise initial loading, empty history, successful refresh, stale cached data, unavailable/expired source, account/server switching and recovery. Late responses cannot overwrite the new selection. Disabled/closed optional monitoring adds no background polling. |
| Repository/GitHub flow | Open a pinned checkout, inspect branch/context, browse open/resolved issues with bounded paging and preserved selection. Inspect the RepoBar feature inventory before promising parity: PR/CI/status/actions need their own source and acceptance evidence, not an assumption that issue browsing supplies them. |
| Write authority | Read-only inspection never files an issue, starts implementation, sends a turn or mutates Git. Explicit write actions identify repository/target and surface cancellation/failure; retries cannot silently duplicate an issue/action. Provider/model choice is separate from issue intake. Credentials stay in supported credential storage and out of logs/screenshots. |
| Real adapters | Exercise the delivered SDK executable through its actual service/feed adapters, including denied access, missing `gh`, unavailable service and incompatible API versions. Rule gates that assert emitted effects or injected callbacks do not prove a service executed. Hide/disable explicitly unsupported actions with an explanation. |
| Visual and accessible | Review actual compact/regular, light/dark and default/larger-text screens; no clipped primary actions or unreadable rates/quotas. Complete the primary flow keyboard-only with visible focus and correct live accessible names/roles. A semantic tree or token-contrast check is supporting evidence only. |
| Dock and modes | Qualify launch/activation, popover ownership/dismissal, state freshness, full-window routing and app identity in the supported Dock layouts/modes; retain the detailed #523 interaction gates. Test transitions and relaunch, not launch alone. Resolve #593's mode terminology before closing that requirement. |
| Install and remove | Build/package/install the delivered executable, desktop entry/icons/assets and activation path; confirm source-to-installed-to-loaded identity. Uninstall retires owned launchers/services/modules/runtime state and prevents supported setup paths restoring obsolete components. State user-data retention and preserve unrelated clients/accounts. |
| Model-built proof | Give a named moderate model the pinned SDK/spec, a clean worktree and synthetic service fixtures. Record prompt/model/runtime, commands, first/final result, repairs and every human intervention. Independently review behavior, visuals and accessibility. Keep the 30-minute first-use coding target and separately report provisioning and checkout-to-running wall time. No full-app quality claim from a generated description or first frame. |

Keep evidence per scenario: source revision, executable identity, environment,
backend/account fixture, mode, expected/actual result and retained screenshot/
log. Report implemented, built, merged, installed, loaded, reviewed and accepted
separately. All applicable gates pass before closing the slice; unmet gates are
explicit blockers, not percentages. Broader chat/stream/approval/attachment
parity remains separately tracked and cannot be inferred from this slice.


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
