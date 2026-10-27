# SDK compatibility and ownership

Status: candidate policy for the current SDK 0.1 development line. This publishes
the implemented boundaries; it does not certify whole-SDK v1 readiness. Release
qualification and evidence remain in SDK-COMPLETION.md.

## Versions identify different contracts

| Contract | Implemented identifier | Owner and enforcement |
| --- | --- | --- |
| Rust packages | msdk / msdk-gtk 0.1.0, edition 2024 | SDK crates in this repository; generated apps currently depend on an explicit local checkout, not a published stable crate |
| App description | `sdk: 1` | `msdk::FORMAT_VERSION`, checker and generated schema; unsupported versions are rejected before code generation or Host startup |
| Service declaration manifest | `manifest_version: 1`; omitted means legacy v1 | `msdk::vocab::SERVICE_MANIFEST_VERSION` and `load_manifest`; describes names, capabilities, arguments, results and feed fields |
| Rust service extension | `Services::api_version() == 1` | msdk runtime and msdk-gtk Host; checked before calls/feeds; separate from manifest and backend versions |
| Runtime diagnostic | `msdk.runtime.diagnostic.v1` | msdk runtime; category, static recovery text and call/source identity; excludes private arguments/output/detail |
| GTK binding and API floor | gtk4 Rust crate 0.11, `v4_20` feature | msdk-gtk Cargo manifest and generator; native headers/runtime are host dependencies, not supplied by the SDK |
| Pantheon shell protocol | `io_elementary_pantheon_shell_v1` interface, maximum version 2 | Vendored compositor protocol XML; panel client binds `min(advertised, 2)` in panel.c. Surface placement still requires a compositor implementing it |
| Real OS/network backend | Provider-specific; no universal version assigned | Backend owner and app adapter must publish protocol, version, account/scope authority, cancellation and availability behavior |

The `v1` in a protocol's name does not override the XML interface version.
Neither a Rust service extension version nor a manifest version proves that a
Gala D-Bus service, T3 server, GitHub API or other backend is compatible.

## Manifest loading

The loader validates the complete manifest before publishing anything. Unknown
keys, malformed arrays/objects, invalid declared types, duplicate names within
one manifest, unsupported manifest versions and conflicting registered contracts
are errors. Identical reloads are idempotent; argument/field object order does
not change the contract. Conflict validation and publication share one registry
write lock, so concurrent loaders cannot partially publish a losing contract.
An error leaves earlier registered contracts intact and adds nothing from the
rejected manifest. CLI check/headless tools and native preview must stop on
manifest read/validation errors; they must not continue with an incomplete
contract. `check --json` emits a service-manifest diagnostic. Native preview
activation/screenshot failure is a nonzero exit, separate from owner acceptance. Successful registry declarations live for the process lifetime.

Direct Rust `register_service`/`register_feed` are legacy one-declaration entry
points; their silent duplicate behavior is not transactional manifest loading.
Use `load_manifest` for app/plugin declarations. Register manifests at startup,
not on each event. This registry is discovery/type metadata, not permission
granting, a sandbox, a network connection or an implemented backend.

## Compatibility and migration policy

Descriptions, manifests and declared service payloads must be checked before
generation/launch. Unsupported versions must fail explicitly; changing only a
version number is not a migration. Keep the original input, use a compatible SDK
or migrate content with explicit review, then check, regenerate typed models and
run the app's behavior/installation tests. No automatic migration is implemented.

Within description format 1, existing accepted component/action/property names
and payload meanings should be preserved. Additions must keep checker, schema,
discovery, text syntax, generator and Host aligned. Incompatible meaning changes
require a new format and an explicit migration path before release. Service
extensions and backend protocols evolve under their own versions. Changes to
argument types, required capabilities or result shapes are incompatible contract
changes; the loader refuses two different definitions under one registered name.

The Rust 0.1 API is provisional. Source-breaking work must be recorded in the
handoff and consumer guide with an actionable replacement; do not advertise
stable Rust ABI or crates.io availability. Existing raw-event `Services::call`
adapters remain compatibility entry points, with their own result-validation,
cancellation and stale-context obligations. New adapters should use typed replies.
No removal timeline is promised until a release policy and consumer migration
are qualified. No released deprecation is currently declared by this document.

## Host qualification boundary

| Environment | Evidence and limitation |
| --- | --- |
| macOS arm64 | Core checker/rules/generator/model and real process/streaming tests pass on retained revisions; this is not a Pantheon GUI or Debian install target |
| Isolated Ubuntu 26.04 x86_64, Rust 1.93.1, GTK 4.22.4 | Prior source-matched GTK/settings app, keyboard/accessibility, light/dark/text-size and package lifecycle receipts exist; runtime/reference snapshot cb77b66 passes source-matched core/native tests, lint, eight reference captures and package lifecycle; newer typed GTK context binding has separate native regression receipts; physical Pantheon qualification stays open |
| Linux arm64 CI | Developer-path run 40363235712 passed on ed9bc5b; this does not qualify newer source or physical M-series hardware |
| Physical elementary/Pantheon on M-series | Target theme, Wayland surfaces, physical scales, hardware lifecycle and owner acceptance remain open |
| Windows | Core Rust compilation and taskbar/status host parity are not qualified; process runner currently returns unavailable outside Linux/macOS |

A generated standalone window does not qualify panel/dock protocol behavior.
The separate native GTK/Granite kit and native T3 implementation are consumers
or adjacent libraries, not proof of SDK-hosted parity. Exact backend OS versions,
supported release distributions and the physical host matrix remain release
gates. Owners are the repository components above; this document does not invent
external maintainer commitments or support guarantees.
