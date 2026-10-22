# Rust Pantheon application SDK and migration inventory

## Direction

Use one shared Rust SDK for our first-party GTK 4 + Granite application
surfaces. T3 (`rust/claude-pantheon-rs`) is the template: it is the most
complete Rust desktop application in this repository and already uses the GTK
4.20 / Granite 7.8 bindings that match the current M-Series image.

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
Gala window clip, and built-in display mask share a provisional 20 logical-pixel
radius independent of font/text scaling. This is not yet verified against the
panel's exact physical curve; the recorded live display-mask setting was 14
logical pixels. Visual qualification must check normal, maximized, fullscreen,
dialog, popover and external-monitor surfaces. Top-level clipping remains a
system/compositor concern; an app class alone cannot guarantee that a window
follows the built-in display outline.

Apple's Human Interface Guidelines are the usability baseline for clear
hierarchy, legible text, direct feedback, keyboard and assistive-technology
access, and layouts that adapt to available space. Translate those principles
through Pantheon's native GTK and Granite controls. Do not copy macOS controls
where they conflict with the host desktop's interaction patterns. Geometry and
scale decisions must preserve user agency: resizable windows, larger text,
system theme, and visible focus remain supported.

The first shared API covers text-line-relative spacing and margins,
compact/regular width observation, accessible names, and shared geometry
classes. Current Rust GTK consumers are T3, Messages, Disk Space, and Software
Updates. Messages is the first responsive-layout adopter:
its list and conversation switch to a back-navigable single-column view when
the window becomes compact.

Usability and performance are SDK acceptance requirements. Shared patterns
must support keyboard navigation, accessible names, long text, increased text
scale, narrow windows, empty/loading/error states, and theme changes. App work
must preserve service/model boundaries, update existing rows instead of
rebuilding whole screens for each poll, stop hidden-window polling, and measure
idle CPU/RSS before claiming a performance win. The SDK must not add a global
animation, timer, renderer, or font-scale override.

## Migration inventory

| Surface | Current implementation | SDK path |
| --- | --- | --- |
| T3 Code client | Rust, GTK 4, Granite | Reference app and first SDK consumer; retain its T3 server boundary. |
| Disk Space | Rust, GTK 4, Granite | Existing Rust app; adopt common layout, accessibility, and responsive behavior. |
| Software Updates | Rust, GTK 4, Granite | Existing Rust app; adopt common layout and accessible controls. |
| Messages | Python GTK/Granite is qualified; Rust preview is in `rust/mseries-messages` | Continue the Rust app to feature parity through BlueFerry's supported D-Bus API; do not create a second MAP or history owner. |
| Torrential | Private Rust/Relm4 fork using Transmission | Already Rust. Adopt the SDK as a git dependency and converge on the same scale/accessibility patterns; keep source and packages private until upstream licensing is resolved. |
| Orchard / Machine Lab | Unified Vala/GTK 4/Granite app | Port in stages, preserving Incus, Podman, VM and action boundaries. Reuse the existing Rust/GTK apps as toolkit examples; do not rewrite the service clients without parity evidence. |
| Native Claude Pantheon | Vala/GTK 4/Granite predecessor | T3 is its Rust successor. Keep the Vala app until T3 workflow parity and migration are accepted, then retire the duplicate. |
| Touch ID Settings | Vala/Granite Switchboard plug and helper | Port the UI after the blocked fingerprint matching/backend work has a supported boundary. A Rust UI alone cannot clear the authentication blocker. |
| Phone setup and verification-code helpers | Python services and GTK UI around BlueFerry and browser integration | Port only user-facing app flows and stable contracts; retain platform/service helpers where they remain the supported bridge. |
| Signal | Official signed Signal Desktop plus downstream launcher/theme/tray integration | Keep the official client. The Rust SDK can standardize our launcher and tray integration, but no Rust replacement may handle messages until Signal offers a supported client API and security boundary. |
| Quick Settings and Wingpanel extensions | Vala plugins/widgets and shell-specific integrations | Separate extension ABI workstream. GTK app helpers do not replace Wingpanel's GObject plugin contract; prototype a Rust/GObject bridge before migrating individual extensions. |
| Display, power, and desktop utilities | Mix of Python, Vala, shell tools, and GTK settings surfaces | Inventory each surface's owner, backend and install path before porting. Command-line tools, policies, services and upstream launchers are not GUI ports. |

## Migration order

1. Stabilize SDK APIs and migrate the already-Rust app set plus private
   Torrential. Fix layout, scaling, keyboard access, text wrapping, and
   idle-work behavior in those consumers.
2. Finish the Messages Rust client and retain the Python launcher until the
   Rust client reaches feature, phone, accessibility, and owner acceptance.
3. Port the large native app surfaces (Orchard and Touch ID) in bounded slices
   with each existing backend contract and qualification gate preserved.
4. Migrate standalone settings/tools and evaluate a Rust/GObject extension
   surface for Quick Settings and Wingpanel.
5. Keep Signal's signed upstream client until its supported integration and
   security requirements change.

Each app gets a source/license review, a package and launcher migration plan,
and its own usability/performance receipt. GTK build success alone does not
qualify an app for replacement or installation.
