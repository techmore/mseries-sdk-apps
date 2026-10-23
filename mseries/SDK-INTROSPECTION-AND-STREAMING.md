# Introspection, streaming and the standard shell (SDK, 2026-10-23)

Owner direction: the UI is a **human and agent first** experience. An agent must see what an app shows easily, through a native
API for every app and its settings, built into the SDK; the UI should also be streamable, with WebRTC the preferred transport. The
chat/support experience gets rebuilt on this SDK when it is done. This note defines the pieces and says what exists.

## 1. Introspection API: a built-in "look inside" for every SDK app

Like a browser's DevTools or an accessibility inspector, but part of the SDK: any running SDK app can be asked what it shows and
what state it holds, and can be told to do what a user could do. No screenshots are needed for an agent to understand a screen.

| Layer | What it answers | Status |
|---|---|---|
| **L1 Headless render** | `msdk render app.msdk [--state JSON]`: the visible tree for a description and a state, as an outline or JSON. Only visible controls, text and templates resolved, values shown, each control's action (`click: Launch(key)`), secrets masked. No window or display needed | **Built** (`msdk::inspect`, tests in `tests/inspect.rs`) |
| **L2 Live API** | The same tree for a running app, plus its state, its settings, a message log and a window screenshot; and input: `send` a message, `click` or `set` by JSON path. One Unix socket per app, JSON lines | Designed, not built |
| **L3 All apps and settings** | A registry (`$XDG_RUNTIME_DIR/msdk/<app-id>.sock`) so `msdk apps` lists running SDK apps and `msdk inspect <app> tree\|state\|settings\|shot` reads any of them; settings are the persisted keys plus the app's declared options, readable and writable | Designed, not built |

Live API calls (L2), all JSON: `tree`, `state`, `settings`, `get {key}`, `set {key, value}`, `send {message, payload}`,
`click {path}`, `type {path, text}`, `log {since}`, `shot` (PNG of the window; from the widget's paintable, so it works under
Wayland without the screencast portal).

Safety rules (they matter because an agent can drive the machine through this):

- The socket is mode 0600 in the user's runtime directory; nothing is exposed on the network by default.
- **Read-only by default.** `send`, `click`, `type` and `set` need the session to be started with input enabled, and every input is
  logged with who sent it.
- Password fields are masked in the tree, the log and screenshots; the API never returns a secret value.
- The `inspect` capability is declared in the app's description, and an app can refuse (a lock screen does).

Why this fits the rest of the SDK: gates, `complete`, `describe` and the benchmark already speak JSON paths; the tree uses the same
paths, so "click this" and "this diagnostic is here" are the same address. Gates can later assert on the rendered tree
(`{"shows": "Delete note"}`), which is a stronger test than state alone.

## 2. Streaming the UI: WebRTC, in two tiers

WebRTC is the right transport (low latency, NAT traversal, one connection for video and data). Because the UI is declarative, there
are two things worth streaming, and the first is far cheaper than pixels.

| Tier | Carries | Use | Cost |
|---|---|---|---|
| **S1 Semantic stream** | Tree deltas and state over a WebRTC data channel (a WebSocket works as the first stand-in) | A phone, a browser or another machine renders the app natively, with real text and accessibility; an agent watches live | Small; no capture, no encoder |
| **S2 Pixel stream** | A video track of the window, plus input events on the data channel mapped to `click`/`type` | Pixel-exact remote use, vision models, demos | A capture path and an encoder |

Plan: build S1 first on top of L2 (it is the same tree, pushed on change). S2 uses GStreamer's `webrtcbin` fed from the window
(PipeWire screencast portal, or the widget texture), with signalling over the introspection socket locally and over Tailscale for
remote use. Status of the dependency on this machine: `gst-inspect-1.0 webrtcbin` is not listed here (PipeWire elements are); on the
Arch base the WebRTC element ships in `gst-plugins-bad`, and `webrtc-rs` is the pure-Rust alternative if GStreamer proves heavy.
Neither tier is built.

## 3. The standard window, settings and shell layout (owner request)

Every app gets the same chrome without writing it, so a model never invents layout:

- **Window:** header bar with the title, the primary action at the start, and an app menu at the end (Settings, Keyboard Shortcuts,
  About). Sidebar and content for collections; welcome or empty state otherwise. Escape closes dialogs. Text zoom and dark mode follow
  the system. (The renderer already draws the header bar, kiosk card, dialogs and popovers; the app menu and About are not yet.)
- **Settings window:** a standard layout, not hand-built: sections with a heading, and rows with the label (and optional description) at
  the start and the control at the end; changes apply immediately and failures are shown; a "Reset to defaults" row. Proposed
  vocabulary: a `settings` surface kind plus `Section(title)` and `Row(label, description) { control }`. Today the `settings` template
  approximates it with a Card; this replaces that.
- **App metadata for the menu and About** lives in `app` (version, licence, website, issue link), checked by `msdk check`.
- **Tokens:** spacing, radius, type scale and window outline come from `msdk tokens`, shared with the compositor, so apps and window
  frames match (the "Mac curves" the owner mentioned belong here once the curves are defined: animation easing and pointer/scroll
  response would be tokens read by both the SDK and the compositor).

### Built so far (2026-10-23)

- **Standard window chrome:** a slightly toned silver title band, window controls at the top left (grey dots that colour on hover), an
  options menu beside them (Settings when the app has a `settings` dialog, About from `app.version/license/website`), the title or a
  `Tabs` switcher in the middle, and the app's own actions on the right. All from `msdk tokens`, so the compositor can match it.
- **Layout standards as tokens:** base font 112 % of the system size, type scale caption 0.92 / body 1 / heading 1.18 / title 1.7,
  spacing 6 / 10 / 16, control height 34, sidebar 224, pane padding 10. Panes run edge to edge, each with its own padding; the sidebar is
  tinted with a divider.
- **`Tabs` and `Stack`:** tabs in the title bar switch pages. **Optional auto-hide sidebar:** `Split(autohide: true)` slides the sidebar in
  after 0.5 s of hover on the left edge (or keyboard focus) and away after the pointer leaves; one file, one token switch, and the
  removal steps are written at the top of `autohide.rs`.
- **`msdk-run --screenshot out.png`:** the `shot` call of the live API, so a design can be looked at, not guessed.

### SDK shortfalls the T3 screenshot exposed (the work list)

Found by looking at the T3 port rendered through the SDK, which is poor today. These are SDK gaps, not app gaps:

1. ~~Layout control~~ **Fixed:** every component takes `align` (on the parent's cross axis), `grow` and `width`; multiline entries take `lines` and `maxLines`; buttons, tabs and dropdowns default to their natural size instead of stretching.
2. ~~Contrast was unchecked~~ **Fixed:** `msdk contrast` audits every colour pair the SDK styles create, in light and dark, for all ten elementary accents (WCAG 4.5:1); it is also a built-in gate for every app (`sdk-contrast`). It found a real failure on its first run (muted text, 4.19:1), fixed by raising the muted opacity token, and the badge text now uses the window background so it reads in dark mode too.
3. ~~Tabs do not look segmented~~ **Fixed:** one rounded group with the selected tab raised.
4. ~~Icons are colourful~~ **Fixed:** buttons and empty states use the `-symbolic` icon when the theme has it; app icons (dock) keep their colour.
5. **No Markdown or highlighted code in the Transcript**, no avatars, provider dots, timestamps or thread status badges.
6. **The sidebar is invisible at first sight** with auto-hide on; a first-run hint (or a pinned default) is needed if the feature stays.
7. **Titlebar band is too subtle** to read as "silver"; tune `chrome.band_strength` against a real screenshot in both themes.
8. **Pointer, scroll and animation curves** (the "Mac curves"): not defined anywhere yet. Belongs in tokens, shared with the compositor.
9. **The libraries still to build** for the Rust Pantheon: the panel host (indicators, popovers), real services (T3 client, keyring,
   notify, file chooser, HTTP/WebSocket), Markdown and code rendering, charts, a calendar grid, and the live introspection socket.

## 4. Building the SDK with the SDK

The tools around the SDK can be SDK apps, and should be: **Studio** (editor with live preview and completion), **Inspector** (L3),
the **gate runner** view, the **service manager** (plug-ins and their manifests), the **template picker**, the **benchmark
dashboard**. They are ordinary descriptions plus Rust services, and each exercises a different part of the vocabulary. The core
(checker, parser, renderer, services) stays Rust: the language describes UIs and simple logic, not compilers, so a literal
self-hosting compiler is not the aim. One honest step toward it is making the vocabulary table itself data (a file the Rust code and the
schema are generated from), so the language's own definition is inspectable by the same tools.

## 5. Order

1. L2 live API in `msdk-gtk` (tree, state, settings, send, click, shot) with the safety rules above; `msdk apps` and `msdk inspect`.
2. Gate assertions on the rendered tree.
3. S1 semantic stream; then decide S2 after checking `webrtcbin` on the Arch base.
4. Standard shell: `settings` surface, `Section` and `Row`, app menu and About.
5. Inspector and Studio as SDK apps.
