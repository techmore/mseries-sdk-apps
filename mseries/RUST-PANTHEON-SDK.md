# Rust Pantheon application SDK and migration inventory

## Direction

Use one shared Rust SDK for our first-party GTK 4 + Granite application
surfaces. T3 (`rust/claude-pantheon-rs`) is the template: it is the most
complete Rust desktop application in this repository and already uses the GTK
4.20 / Granite 7.8 bindings that match the current M-Series image.

Regular Pantheon/elementary defaults and the elementary stylesheet are the
visual source of truth for this SDK. Pantheon-R is performance-focused; its
theme has not been fully audited, so its current colors, shapes and spacing are
not evidence of Pantheon parity. Follow user-selected GTK theme, dark mode and
accent settings where the platform exposes them, with verified elementary
defaults as fallbacks. Record visual parity only after checking the relevant
Pantheon and Pantheon-R screens in light/dark modes, scale settings and window
sizes.

The SDK starts at `rust/mseries-pantheon-kit`. Its layout contract uses the
current Pango text line height as a spacing and breakpoint unit. This tracks the
effective text metrics without pretending Pango line height is CSS `em`. GTK
maps logical coordinates to device pixels, so apps must not multiply lengths
by a display scale factor. Default window dimensions remain app-chosen logical
sizes; enlarged text changes spacing and responsive layout, not the requested
window size. Apps must honor desktop font and text-scale settings rather than
guessing a font from pixel resolution or viewing distance.

The Mac hardware target needs rounded content surfaces and window frames. App
cards use a shared 12 logical-pixel radius; the GTK client-side window style,
Gala window clip, and Rust compositor's built-in display mask share a
provisional 20 logical-pixel radius independent of font/text scaling. The Rust
compositor masks all four corners only on recognized internal-panel connectors
(eDP, LVDS or DSI); `PANTHEON_SCREEN_CORNER_RADIUS` selects a logical-pixel
radius at compositor startup and `0` disables the mask. Cached, supersampled
textures add no polling or per-frame rasterization. This is not yet visually
qualified against the panel's exact physical curve; the recorded previous
display-mask setting was 14 logical pixels. Visual qualification must check
normal, maximized, fullscreen, dialog, popover and external-monitor surfaces.
Top-level window clipping remains a separate system/compositor concern; an app
class alone cannot guarantee that a window follows the built-in display
outline.

Apple's [guidance for layout](https://developer.apple.com/design/human-interface-guidelines/layout)
and [designing for macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos)
are complementary usability references for clear hierarchy, legible text,
direct feedback, keyboard and assistive-technology access, and layouts that
adapt to available space. Translate those principles through Pantheon's native
GTK and Granite controls, following the [elementary OS HIG](https://docs.elementary.io/hig)
for native interaction and desktop integration. Do not copy macOS controls
where they conflict with the host desktop's interaction patterns. Geometry and
scale decisions must preserve user agency: resizable windows, larger text,
system theme, and visible focus remain supported.

The first shared API covers text-line-relative spacing and margins,
compact/regular width observation, accessible names, and shared geometry
classes. `Polling` adds an explicit enable/disable control, interval changes,
manual refresh, and a binding to GTK mapped state; pollers start disabled so
apps do not acquire background timers by accident. `SystemSampler` provides
on-demand Linux CPU and memory snapshots, and `poll_system_metrics` combines
it with that controllable timer. These are small host metrics, not a process
manager or a replacement for the compositor's own counters. `DockBadge`
provides a best-effort session-bus client for Pantheon's LauncherEntry count
badge and optional progress bar. It connects asynchronously, coalesces updates
while connecting, hides a zero count, and has no polling timer. Messages uses
its unread-message total; T3 uses the shared API for its running-thread count
and progress indicator.

`has_focus_within` handles composite controls such as GTK SearchEntry, whose
keyboard focus belongs to an internal child. Use it for entry-specific shortcuts.
`attach_context_menu` gives an accessible, focusable anchor the same right-click,
long-press, Menu-key and Shift+F10 behavior. Supply the popover actions and name
its anchor with `set_accessible_label`; the helper owns parenting and removes the
popover when the anchor is destroyed. Messages uses this shared path for both
conversation and message actions, replacing duplicated pointer-only handlers.
These helpers add event handlers only when attached; they start no polling.

`ResumeWatcher` provides a best-effort, asynchronous logind resume signal for
apps that need to refresh after sleep without a permanent wake-up timer. Rust
Messages now combines it with a user-controlled 30-second `Polling` fallback;
the poller is disabled while the window is unmapped, and duplicate refresh
requests coalesce into one follow-up so delayed D-Bus replies cannot pile up or
replace newer history with an older snapshot. Its refresh switch is restored
from an owner-only `messages-rust.json` preference, separate from the shared
send-key preference, so opting out of background refresh survives restarts.

Disk Space now uses the SDK's text-relative spacing and margin helpers for
persistent layout groups, GTK font-relative CSS padding for repeated rows, and
SDK accessible names for icon-only actions and its main content lists. Its
three-pane navigation still needs a compact-window layout pass; fixed pane
minimums remain until that interaction is redesigned and visually qualified.

Current Rust GTK consumers are T3, Messages, Disk Space, and Software Updates;
Torrential consumes the SDK from its private fork. The System Monitor preview
uses the SDK's sampler and polling control. Software Updates is part of this
SDK rollout too: its native Rust window uses shared spacing and adaptive
layout helpers, while the SDK owns the unprivileged CLI adapter for the typed
`mseries-update-core` contract. Messages
is the first responsive-layout adopter: its list and conversation switch to a
back-navigable single-column view when the window becomes compact.

### OS-to-app API boundary

The SDK is an app-level Rust library, shipped and versioned with the first-
party app layer. It is not an OS daemon, kernel component, privileged service,
or a shortcut for applications to link against OS implementation code. The OS
owns privileged policy and services; applications use stable, versioned
contracts rather than linking to service internals. The SDK provides common
GTK/Granite presentation, accessibility, scaling, polling and typed client
adapters for those OS contracts. An app owns its domain-specific user flow and
asks for confirmation before consequential actions.

For each integration, the OS side publishes the provider, permissions,
compatibility/version range and lifecycle guarantees. The app SDK owns the
typed client and maps that contract into stable Rust types. Apps call the SDK
adapter; they do not call a private daemon implementation or parse its logs,
database, filesystem state or command output. Both sides can evolve
independently while CI and release metadata check that their API versions
overlap. An app-level feature that needs no OS privilege can remain entirely in
the app and SDK; a feature that needs OS state or authority gets an explicit
OS contract first.

Software Updates is the working example: the OS updater and root helper own
transaction policy, package staging and rollback. The app reads status schema
1 and can request only the closed set of typed operations in
`mseries-update-core`; the app does not decide packages or execute privileged
commands directly. The SDK owns the typed adapter and its supported
`mseries-update-core` contract; the service provider and privileged helper stay
OS-owned. Keep that contract versioned and publish its supported range with the
SDK integration.
Messages follows the same boundary for BlueFerry's session D-Bus service: the
SDK owns the API v2 client and typed status, thread and contact models, while
BlueFerry remains the only owner of MAP access, message history, recipient
validation and photo policy. The SDK folds quoted, text-form Tapbacks in the
typed message model and leaves unmatched events visible. The app does not open BlueFerry's database or
manage its Bluetooth session. Contact-photo decoding is also shared SDK code;
it accepts bounded JPEG/PNG headers, scales in memory and leaves the user's
existing opt-in unchanged. The SDK formats BlueFerry ISO timestamps in the
local calendar for message timelines and direct-conversation rows. Its shared
timeline helper groups nearby messages by direction and group sender and emits
date/time headings after long gaps. BlueFerry PR [#277](https://github.com/erikwb/blueferry/pull/277)
merged at `ff418298e3e9b71b650e0d72b4b1ba61576cf29f`; it adds an optional stable
UUID for the configured phone to the existing status contract. The SDK accepts
that field and scopes imported pin metadata in Secret Service with the ID. The
current Python client reads the same field and keyring item; when BlueFerry
reports a valid ID, it can migrate the old Python-only item locally. The latest
public release remains v0.8.1 from 2026-09-15, before the merge, so released
packages omit the ID and import actions ask the user to update BlueFerry. Neither
client uses the Bluetooth address as its new preference scope. See BlueFerry's
[pinned D-Bus API contract](https://github.com/erikwb/blueferry/blob/60fd7ef1abbd25947981f0cf27a521d82784a75f/docs/en/developer/dbus-api.md#compatibility).
The same released API v2 also exposes `Messages1.ListRecent`; the SDK now
provides a bounded typed client for that method. Rust Messages refreshes this
phone-exposed inbox snapshot on startup, manual refresh, reconnect and resume,
then every five minutes while connected when auto-refresh is enabled. It merges by validated address and
message handle, avoids ambiguous and group routing, and retains the last good
snapshot when the phone read fails. This adds no polling timer of its own: the
existing switch-controlled status poll schedules the five-minute check, while manual and
resume events request it directly.
The Rust Messages preview now includes bounded public HTTPS image, GIF, and
YouTube previews. Incoming links require an explicit load; sent links can load
automatically. Downloads are DNS-pinned to validated public addresses, limited
in bytes, dimensions, frames and in-memory cache, and never write to disk or
fetch MAP attachments.
Likewise, the compositor owns frame and client counters; its future read-only
metrics protocol belongs to the OS, with a typed SDK adapter for System
Monitor. This keeps CPU/memory polling and compositor telemetry on one
consistent app-facing interface without making apps parse logs or compositor
internals.

The custom Rust Pantheon compositor already exports shell and display
integration over its own Wayland and D-Bus interfaces. Its frame timing and
client counters currently go to structured journal events (`pantheon_metrics`),
not to an app-facing metrics API. The SDK must not scrape the journal. A
versioned, read-only compositor metrics interface is a separate OS-side task;
the System Monitor should consume it through a typed SDK adapter once that
contract exists. The existing host CPU/memory sampler remains useful on hosts
that do not run the custom compositor.

Usability and performance are SDK acceptance requirements. Shared patterns
must support keyboard navigation, accessible names, long text, increased text
scale, narrow windows, empty/loading/error states, and theme changes. App work
must preserve service/model boundaries, update existing rows instead of
rebuilding whole screens for each poll, stop hidden-window polling, and measure
idle CPU/RSS before claiming a performance win. The SDK must not add a global
animation, timer, renderer, or font-scale override.

First release-build smoke baseline (2026-10-22): one Messages process on the
MacBookPro18,3 host after a 10-second warm-up, sampled for 20 seconds in a
1280×900 Xvfb display with Cairo rendering and an empty private D-Bus service
directory, reported 0.00% process CPU at `/proc` tick resolution and 146.9 MiB
median RSS. This is an offline, single-sample baseline only; it does not qualify
the owner Wayland session, BlueFerry-connected use, other text scales, or compare
polling on versus off. Repeat those cases before claiming a performance win.

## Migration inventory

### Rust Pantheon applications and first-party GTK app candidates

| Surface | Current implementation | SDK path |
| --- | --- | --- |
| T3 Code client | Rust, GTK 4, Granite | Reference app and first SDK consumer; retain its T3 server boundary. |
| Disk Space | Rust, GTK 4, Granite | Existing Rust app; initial SDK adoption covers text-relative spacing, row padding, and accessible names. Add and qualify compact-window pane navigation before calling its responsive pass complete. |
| Software Updates | Rust, GTK 4, Granite; `mseries-update-core` contract and OS helper | Included in the SDK rollout: shared responsive/scaling/accessibility helpers and a typed OS update contract; keep privileged transaction logic in the OS. |
| Messages | Python GTK/Granite is qualified; Rust preview is in `rust/mseries-messages` | Continue the Rust app to feature parity through BlueFerry's supported D-Bus API; do not create a second MAP or history owner. |
| Torrential | Private Rust/Relm4 fork using Transmission | Private `mseries-native` consumes the pinned SDK and shared stylesheet. The current layout/accessibility pass adds text-relative margins and spacing, compact-width search sizing, localized accessible names, and cached torrent-row accessible labels. Keep source and packages private until upstream licensing is resolved. |
| System Monitor | Rust GTK 4/Granite preview at `rust/mseries-system-monitor`; btop is installed as a TUI command | Embed btop through VTE inside the shared app shell; use SDK host CPU/memory snapshots and controllable polling. Add a typed adapter to the custom compositor metrics API after that API is defined. |
| Battery | Python GTK 4/Granite read-only history and health dashboard | Port the presentation to Rust; keep its logger/history format and read-only contract. |
| Display Settings | Python GTK 3 display arranger and diagnosis UI | Port to GTK 4/Rust; preserve the Mutter/Gala display-configuration contract and rollback behavior. The profile watcher stays a service/CLI. |
| AirPlay Outputs | Python GTK 3 selector backed by PipeWire commands | Port the selector UI; retain the PipeWire group service and selection format. |
| Phone Setup | Python GTK 4/Granite onboarding over BlueFerry and BlueZ | Port the user flow; BlueFerry/BlueZ remain the pairing and message-access owners. |
| Performance Troubleshooter | Python GTK 4/Granite UI over the local diagnostic command | Port the UI and preserve the bounded diagnostic protocol and privacy rules. |
| Apple Passwords (experimental) | Python GTK 4/Granite sign-in and vault UI | Candidate after the security boundary is re-reviewed; preserve private-pipe credential handling, Secret Service storage, explicit recovery consent, and opt-in installation. |
| PolyBagger Print Manager | Python GTK 4/Granite operator app plus CLI | Port the window while preserving CLI/printer checks and operator confirmation; physical printer acceptance remains required. |
| Screenshot Shelf and Markup | GTK 4 screenshot helper; external Swappy/Papers editors | Port the shelf UI only with its XWayland placement, document-portal drag, and screenshot-retention contracts intact. Keep existing editor engines. |
| Window Recovery | Python launcher plus a Zenity chooser over Gala's `GetWindows` and `FocusWindow` D-Bus methods | Replace the chooser with a small SDK-backed GTK/Granite window; retain the read-only listing path, explicit selection and fresh-session capability check. Add a typed SDK adapter after Gala publishes a versioned app API contract. |
| Screen Share | Vala GTK 4/Granite standalone window plus a Wingpanel plugin | Port the standalone app after documenting the screen-capture portal contract; the panel plugin remains a separate extension target. |
| Orchard | Unified Vala GTK 4/Granite app for machines, Incus and Podman | Large staged Rust port; preserve VM, Incus, Podman, storage and privilege boundaries. Keep current source until each replacement surface qualifies. |
| Orchard for Incus (older client) | Python GTK 4/Granite app, with a separate Vala implementation | Inventory as predecessor/alternate; port only if it still owns a unique shipped workflow after unified Orchard parity review. |
| Drawer Shortcuts | Python GTK 4 settings window over Gala shortcut integration | Port the settings UI; Gala shortcut registration remains an OS/shell contract. |
| M-series Launcher | Python GTK 4 XWayland-centered launcher | Port only after the custom compositor provides a supported placement/activation path; do not reproduce global scale overrides in the SDK. |
| Horizon RDP | Upstream Rust/FreeRDP client behind a zenity launcher | Keep the upstream transport; replace the small connection prompt with a shared native app if packaging and license review permit. |
| Native Claude Pantheon | Vala/GTK 4/Granite predecessor | T3 is its Rust successor. Keep the Vala app until T3 workflow parity and migration are accepted, then retire the duplicate. |
| Signal | Official signed Signal Desktop plus downstream launcher/theme/tray integration | Keep the official client. The Rust SDK can standardize launcher/tray integration, but no Rust replacement may handle messages until Signal offers a supported client API and security boundary. |

### Platform surfaces and code that should not be treated as ordinary app ports

| Surface | Current implementation | Migration boundary |
| --- | --- | --- |
| Touch ID Settings | Vala/libadwaita Switchboard plug and helpers | Separate Switchboard/GObject ABI workstream; port the settings UI after the fingerprint matching backend has a supported boundary. A Rust UI alone cannot clear the authentication blocker. |
| Work Mode Settings | Vala/libadwaita Switchboard plug | Separate Switchboard plug ABI; share interaction/accessibility rules, then evaluate a Rust GObject bridge. |
| CodexBar, RepoBar, Share Eyes and desktop notification indicators | Wingpanel Vala plugins with some standalone settings or companion apps | Separate Wingpanel/GObject extension ABI. The GTK app SDK does not replace shell plugin contracts; prototype and qualify a Rust/GObject bridge before migrating plugin code. |
| Quick Settings | Vala GTK shell modules | Separate shell extension ABI and lifecycle workstream. |
| Custom Pantheon compositor | Rust Wayland compositor, custom shell protocol, and D-Bus compatibility services | OS component. Expose app-facing capabilities through versioned protocols/D-Bus contracts; do not link compositor internals into app crates. |
| Display profiles, battery logger, power tools, BlueFerry and service helpers | CLI programs, user services and backend modules | Preserve stable service/CLI contracts. Porting a frontend does not imply rewriting these owners. |
| Teams, Outlook, YouTube and other web wrappers | Browser/PWA clients or launcher scripts | Keep browser engines and web service boundaries; only port a custom native wrapper when it adds a real native workflow. |
| Rust compositor session, kernel tools and system policies | OS/session and kernel integration | Belong to the OS or Kernel workstream, not the first-party app SDK. |

## Migration order

1. Stabilize the SDK with the existing Rust app set, private Torrential, the
   explicit polling/system metrics API, and a btop-backed System Monitor.
   Measure idle work in consumers; keep pollers off while pages are hidden.
2. Finish the Messages Rust client and retain the Python client until feature,
   phone, accessibility, and owner acceptance.
3. Port the smaller standalone GTK apps in contract-sized slices, then port
   Orchard's large surfaces while preserving every existing service boundary.
4. Move settings and shell integrations only after their Switchboard,
   Wingpanel, Quick Settings, and compositor-facing Rust/GObject APIs are
   versioned and qualified.
5. Keep Signal's signed upstream client until its supported integration and
   security requirements change.
6. Stage source-repository separation only after each satellite repository
   builds, packages and passes the hub's pinned integration checks. Keep this
   repository complete during that transition; see
   [`REPOSITORY-TOPOLOGY.md`](REPOSITORY-TOPOLOGY.md).

Each app gets a source/license review, a package and launcher migration plan,
and its own usability/performance receipt. GTK build success alone does not
qualify an app for replacement or installation.

DockBadge wire-contract qualification runs with the isolated Messages GUI runner.
It uses its own session bus and checks URI/count/progress/visibility, preconnection
coalescing, clamping and clearing; owner dock visual acceptance remains separate.

ImportedPinState reads v1/v2 shared Python records. Version 2 retains bounded
local_pins and hidden_imports and orders visible imported/local pins before the
remaining history. Clients must preserve those fields when reviewing a new import;
Rust Messages does so. favorite_thread_keys resolves visible imported/local favorites against the complete inbox;
toggled_favorite returns a validated v2 state without mutating server stars or
the imported snapshot. Rust Messages persists that state after phone checks.
