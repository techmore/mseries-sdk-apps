# Rust Pantheon SDK: app-building guidelines

## Required workflow

Before scaffolding, porting, or generating an application, read
[APP-BUILDING-GUIDE.md](APP-BUILDING-GUIDE.md). It contains the reusable prompt,
early running-example and checkpoint policy, starter requirements, build-loop
rules, and completion criteria. AGENTS.md requires this reading for app work.

Start with the visible user journey. Show a running primary flow within about
30 minutes or report the concrete setup blocker. Review actual appearance and
interaction early; compilation and fixture checks do not establish acceptance.
At roughly 45-minute checkpoints report elapsed time, running evidence, remaining
product work and scope changes. Continue independent work while awaiting feedback.

## SDK and template direction

Use native Rust GTK/Granite conventions for first-party app surfaces. Inspect T3
at rust/claude-pantheon-rs as a reference implementation; its existence does not
make every screen an approved template. Reuse an app shell and fixture backend,
then extract demonstrated repeated behavior into shared SDK helpers.

A maintained starter should demonstrate window chrome and rounding ownership,
legible typography, adaptive spacing/scaling, navigation, focus/accessibility,
loading/error/empty states, logging, settings, optional polling and dock badges.
It must provide verified create, run and package instructions and an immediately
useful running app. A generator is a desired deliverable, not a claimed existing
command. Avoid introducing a description language without a measured need.

Elementary/Pantheon defaults and the user's theme, dark-mode and text settings
are the visual baseline. Translate Apple's usability principles through native
controls. Use logical coordinates and effective text metrics; do not guess font
size from screen resolution or viewing distance or apply display scaling twice.
Window/display clipping belongs to the compositor/windowing contract where
appropriate; an app stylesheet cannot establish physical-panel compliance.

Keep service authority behind explicit versioned APIs. Preserve existing clients,
data, account boundaries and recovery paths during migrations. Do not import
private service databases or replace transport implementations to simplify a UI.

## Fast build loop and honest delivery

Use incremental debug builds, focused checks and shared caches during iteration.
Avoid repeated optimized builds, packaging and installation for small edits.
Measure compilation, linking, testing, packaging and queue time separately; keep
receipts tied to source revisions. Preserve caches and investigate unexpected
rebuilds before repeating them. Run/add tests when requested or required by the
applicable instructions. Qualify the release at an appropriate checkpoint.

Report implemented, built, installed, loaded, reviewed and accepted separately.
Do not give unsupported completion percentages. Preserve stop/defer instructions.

## Current scope boundary

These are development guidelines, not a declaration of a stable SDK v1 API or a
shipped scaffolder. Signal integration, Rust Messages and their combined launcher
remain incomplete and deferred by the owner. Their draft implementation inventory
and acceptance receipts remain in PR #526; this guidance does not merge those
apps or qualify them for replacement. Resume that app work only when requested.

The authoritative deferred app status is [MESSAGING-STATUS.md](MESSAGING-STATUS.md).
Read it before continuing Signal/Messages work or interpreting older receipts.
