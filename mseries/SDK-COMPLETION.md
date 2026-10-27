# SDK completion and qualification ledger

The requested outcome is a complete, tested and validated Rust Pantheon app SDK.
The product contracts are issue #535, the current APP-BUILDING-GUIDE and
RUST-PANTHEON-SDK, the sdk-bench specs/rubric, and the Dock standard proposed in
PR #605. Existing Signal/Messages deferral remains in force: use synthetic
messaging data for SDK checks; do not resume or replace those clients.

Owner scope clarification (2026-10-26): a native experience is **not required
for every app**. Complete and validate the SDK's shipped capabilities and a
small set of representative flows. Existing web interfaces, wrappers, webviews
and hybrid apps can be retained where appropriate; do not turn SDK completion
into a project-wide native UI migration. A frontend integration must actually
exist and be tested before it is advertised; the current vocabulary does not
ship a generic WebView component. Full T3/product feature parity, consumer
redesigns and app-by-app native status ports are separate app work, not SDK
completion gates. Applicable backend/lifecycle/security contracts still apply
to interfaces the SDK actually ships or claims to support.

A green compiler/test run is scoped evidence. SDK completion requires every
applicable row below to have retained source-matched evidence; missing evidence
is an open gate. No completion percentage is assigned.

| Requirement | Authoritative evidence needed | Current state / next work |
| --- | --- | --- |
| Ownership/runtime/API support | Published graphics, shell, toolkit and app API versions/owners; stable/provisional/unsupported boundaries; compatibility and deprecation policy | SDK-COMPATIBILITY.md publishes candidate version/owner/migration boundaries and actual host evidence; manifest loading is versioned and atomic; exact real backend OS protocols and release/physical supported-host qualification remain open |
| Description authoring/discovery | Checker, schema, discovery and rules agree; all six starters pass; valid user strings survive generation | Starter escaping repaired; core suite includes current T3 identity/page assertions |
| Typed Rust and standalone app | `msdk gen` emits compiling records/State/Message plus a real GTK executable; original description/rules retained; custom backends explicit | All six model variants compile; actual generated settings app builds/tests/lints and runs on isolated Ubuntu 26.04 x86_64; source-matched receipts in sdk-tests/receipts/2026-10-26; hosted ARM developer-path run 40392328090 attempt 2 passes on dbf4eee, including scoped-stream core and native host checks; attempt 1 failed to open its display before accessibility registration; why the display failed remains unknown; physical target qualification remains open |
| Clean developer path | Clean checkout to useful app, recoverable service error, typed diagnostic/source context, optional capability, build/package/run guide; coding <=30 min and provisioning/wall time separately measured | Home Storage read-only sample and SDK-GOLDEN-PATH.md added; actual same-process keyboard/AT-SPI failure/retry, eight native reviewed captures, generated app build/lint and package lifecycle pass; measured novice/model trial still needed |
| Services and errors | Version mismatch/unavailable/denied/malformed data through actual adapters; bounded async/cancel/retry behavior; privacy-safe structured diagnostics | Candidate typed async replies, extension ABI checks, safe diagnostics, owned process groups and bounded Rust streaming/partial output implemented; scoped streaming and context pass core tests on macOS and hosted ARM Linux; native typed error/recovery/fixture/close checks pass on exact source; backend OS protocols and streaming/app-specific GTK adapters remain open |
| Optional lifecycle | Disabled/closed causes no extra reads, stale completions ignored, sources released; exact scope/account remains visible | Host/window watcher/process ownership and core scopes pass; explicit typed GTK reply bindings reject obsolete success/error including a second error-callback queue, with actual widget and release checks; scoped Rust streaming passes macOS and hosted ARM core tests; opt-in GTK feed context delivery passes a hosted ARM native lifecycle regression; legacy/built-in/GTK streaming/T3 context adapters and real account identity/teardown remain open |
| SDK native host/components | Source-matched light/dark, regular/compact, default/larger text and supported scales; keyboard-only primary journeys and live accessible names/roles in representative SDK consumers | Settings and Home Storage keyboard/AT-SPI and reviewed matrices qualify the SDK host on tested snapshots; target Pantheon/physical scales/owner acceptance remain open. No every-app native UI requirement |
| Shell/status module | Discoverable reusable CodexBar-derived provider/account module; Windows bottom-right and elementary status hosts; all semantics and exact interaction gates from #605 | Candidate `msdk::usage` data semantics pass macOS core tests; a candidate GTK row renderer passes a hosted ARM widget test; multi-provider previews, a scoped aggregate CodexBar read, GTK refresh/failure/retry control and reusable anchor hover, pin, dismissal and keyboard interaction pass synthetic hosted ARM checks (run 40416161840); an opt-in shared T3 host page passes local synthetic hosted/dock-mode checks with neighboring indicator stubs; live collector reads, account adapters, full presentation, actual desktop module builds and target qualification remain open; no parity claim |
| Installation/distribution | Correct executable/desktop ID/icon/metainfo; package validation, fresh install/reinstall/remove and source-installed-loaded identity | `msdk pack` implemented with actual ELF architecture/native-library dependency resolution and validated desktop/AppStream/icon/license assets; isolated x86_64 settings and current Home Storage package/install/reinstall/installed launch/binary equality/remove pass on source-matched snapshots; exact SDK/generated source fingerprints and binary/package hashes retained; arm64/target-distro/target-host qualification pending |
| Representative integrations | A small set of consumers or focused harnesses proves shipped SDK service, error, lifecycle and authority contracts; qualify any specifically advertised integration flow | Settings/Home Storage, real process work, private D-Bus and typed service/context cases supply evidence. Use existing frontends or focused adapters for any uncovered SDK contract; full T3/native product parity is separate. Messages remains deferred |
| Performance | Same hardware/workload startup/RSS/idle CPU/wakeups/render metrics; explicit build/provision/queue timings | No new performance result claimed |
| Model usability | Exact licensed model/runtime/weights/prompt/spec; clean isolated no-network trials, same rubric, failures/repairs/interventions retained; independent visual/behavior acceptance | Pinned discovery runner, four boundary tests and an offline non-root native/tool preflight are retained; initial s01 admission deferred before inference to preserve an existing resident model; independently scored cohorts, repeated runs and baseline comparisons remain open |
| Release/API readiness | All above gates audited against final immutable artifacts, docs match shipped behavior, unsupported scope explicitly recorded | SDK v1 completion remains unproven |

## Current verification environment

This chat's Mac has Rust but no GTK development package. Core checks use an
isolated copy of msdk and its external fixture files to avoid resolving the
unrelated pinned Smithay Git dependency in the whole-OS workspace. Generated
model compilation uses cached Cargo dependencies. This is not Linux GTK, target
Pantheon, installation, hardware or owner visual acceptance evidence.

`mseries/sdk-tests/prepare.py NEW_DIRECTORY` creates the reproducible SDK-only
workspace, keeping the checked-in dependency lock as its starting point. The
M-series SDK developer path workflow provisions Linux separately, tests/lints
core, builds/tests/lints a generated GTK app and captures its actual executable
in light/dark isolated desktops. Its resulting locks/source/hash/screenshots are
retained. Runtime and target-host gates remain open until those are exercised.
