# Corners: Mac-compliant rounded edges (SDK tokens and compositor contract, 2026-10-23)

Owner direction: the "curves" are rounded edges that match the MacBook Pro: the display's own rounded corners, and windows and
controls that follow them. This defines one corner system for the SDK and the compositor. Related: the open P2 item in
[BUG-FIX-ROADMAP.md](BUG-FIX-ROADMAP.md) ("app windows do not consistently follow the rounded shape of the built-in display"),
[DESKTOP-SETUP.md](DESKTOP-SETUP.md) (the Gala screen-corner patch), [SDK-INTROSPECTION-AND-STREAMING.md](SDK-INTROSPECTION-AND-STREAMING.md).

## 1. What exists today

| Piece | State |
|---|---|
| Screen-corner mask (black rounded masks over the built-in display's four corners, above everything, external monitors left square) | A **Gala patch** (`mseries/patches/gala-screen-corners.patch`), radius 14 logical px (`GALA_SCREEN_CORNER_RADIUS`). The live session runs the **Rust compositor, which has no mask and no corner code at all** |
| Window radius | A GTK stylesheet tweak (0.5rem to 0.8rem). Covers GTK apps only; the roadmap notes it does not cover every application and that the mask rounds the display, not individual windows |
| A numeric standard | Apple's Human Interface Guidelines do not publish one mandatory radius for all windows, so the numbers below are ours, tuned by eye on this display, not copied from macOS |

## 2. The corner tokens (built, in `msdk/data/tokens.json` under `corners`)

Logical pixels, shared by SDK apps and meant to be read by the compositor:

| Token | Value | Used for |
|---|---:|---|
| `screen` | 14 | The built-in display's physical corner (the mask) |
| `window` | 12 | Floating window outline and title band |
| `popover` | 10 | Menus, popovers |
| `card` | 12 | Cards, editors, the Transcript bubbles' parent shapes |
| `bubble` | 14 | Chat bubbles |
| `control` | 8 | Buttons, entries, segmented tabs |
| `min` | 4 | The smallest radius a concentric inner corner may shrink to |
| `style` | `continuous` | Reserved: corner shape (see section 4) |

**Rules (the "concentric" rule is Apple's):** a shape inset from a rounded corner takes `outer - inset` (helper
`concentric_px(outer, inset)`, never below `min`), so inner and outer curves share a centre and the gap stays even. A window never
has a larger radius than the display (`window <= screen`; tested). Maximized and fullscreen SDK windows have radius 0 in their own
stylesheet because an app cannot know whether it is on the built-in display or a square external one; the compositor decides (below).

## 3. Compositor contract (not built in the Rust compositor)

The compositor is the only layer that can make every app, including third-party and client-side-decorated windows, follow the curve.
It should read the same `corners` tokens and implement:

1. **Screen mask** on the built-in output only, radius `screen`, drawn above windows, the dock, the top bar and the lock screen.
   External outputs stay square. (Replaces the Gala patch for the Rust session.)
2. **Window clip:** every toplevel's rendered surface is clipped to a rounded rectangle of radius `window`; shadows and the focus
   outline follow the clipped shape.
3. **Hug the display:** a window whose outer corner touches a corner of the built-in display (maximized, tiled halves and quarters,
   fullscreen) uses `screen` at that corner, so it follows the display's curve; its other corners use `window` or 0 against a screen edge.
   On an external output those corners are square.
4. **Popups and dialogs:** popovers and menus clip to `popover`, dialogs to `window`.
5. **Layer surfaces** (dock, panels) round only the corners away from the screen edge, with `window`.
6. **Content stays intact:** clip only; never resize or crop client content. Surfaces that cannot be clipped cleanly (video overlays,
   some X11 windows) are listed, as the roadmap item requires.
7. **Per-output policy** from the compositor's display configuration (which output is the built-in panel), not from the apps.

## 4. Continuous corners

Apple's corners are continuous curves (a squircle: curvature eases in and out, no sudden change where the arc meets the edge), not
circular arcs. CSS `border-radius` can only draw circular arcs, so SDK stylesheets approximate with `corners.*`. The compositor's
clip shader can draw the real shape from a signed-distance function of a superellipse, which is also cheap on the GPU; `corners.style`
selects `circular` or `continuous`. This is the most visible "Mac-like" detail and costs nothing extra once a shader clips windows.

## 5. Calibration

The panel's real corner radius is not documented. Measure it once on this MacBook Pro: show a test pattern with a high-contrast border
and bring the mask radius up until the mask edge meets the glass edge with no visible gap or bite, in both light and dark; record the
number in `tokens.json` and this file. The current 14 came from eye-tuning (raised from 10 on 2026-10-1x).

## 6. Work items

1. Rust compositor: screen mask, window clip with a rounded-corner shader (Smithay GLES custom shader, as other Smithay compositors do),
   hug-the-display policy, popups, layer surfaces, per-output policy, tests for the geometry (which corners get which radius).
2. SDK: done for tokens, CSS and the concentric helper; next is an `msdk shot` check that compares a window's corner pixels to the token.
3. Calibration pass and a record of the result.
4. Qualify native GTK, libadwaita, Electron/Chromium and X11 windows, CSD and SSD, maximized, fullscreen, dialogs and popovers, and an
   external display, with screenshots.
