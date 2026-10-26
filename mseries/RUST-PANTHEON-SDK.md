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

Use the implemented `.msdk`/JSON description, rules and declared-service path
for SDK apps, with app-local Rust `Services`/`Logic` for documented gaps.
Native Rust GTK/Granite helpers remain available in `mseries-pantheon-kit`.
Native T3 (`claude-pantheon-rs`) and the SDK-hosted T3 executable
(`claude-pantheon-sdk`) are separate consumers; native features do not imply
SDK parity. Source inventory: [SDK audit](SDK-AUDIT-2026-10-23.md#2026-10-25-t3-monitoring-and-sdk-qualification-addendum).

| Implemented path | Boundary |
| --- | --- |
| `msdk templates`, `msdk new` | Six starters generate description, gates and README; not a full Cargo app/backend/package. |
| `describe`, `schema`, `check`, `gate`, `render` | Discover/validate vocabulary and headless state/effect scenarios. Gates do not invoke real backend services; semantic rendering is not live GTK accessibility evidence. |
| `msdk-gtk::Host`, `Services`, `LogicFactory` | GTK hosting, declared controls/rules and explicit backend extension points. T3 transport/GitHub mappings are app-specific, not automatic SDK facilities. |
| `FixtureServices` | Isolated UI/data scenarios, not proof of live integration. |
| Native kit | Separately consumed GTK/Granite, scaling, accessibility, polling/badge helpers; lifecycle must still be qualified by the app. |

`msdk gen`, `msdk pack`, `msdk parity` and a live introspection registry remain
proposals, not commands to give an LLM as existing tools. The implemented
description generator is real; complete standalone app/package generation is
not established. Follow the [T3 priority-slice gates](APP-BUILDING-GUIDE.md#t3-priority-slice-and-sdk-qualification-gates):
token monitoring and RepoBar-derived GitHub overview first (#596), with
Shiplog (#595) building on that data. Whole-chat parity is a separate contract.

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

These guidelines do not declare a stable SDK v1 API or a complete shipped app
scaffolder. The description starter and GTK host exist. Signal integration,
Rust Messages and their combined launcher
remain incomplete and deferred by the owner. Their draft implementation inventory
and acceptance receipts remain in PR #526; this guidance does not merge those
apps or qualify them for replacement. Resume that app work only when requested.

The authoritative deferred app status is [MESSAGING-STATUS.md](MESSAGING-STATUS.md).
Read it before continuing Signal/Messages work or interpreting older receipts.
