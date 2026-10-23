# Frozen baselines and the parity loop (SDK, 2026-10-23)

Owner direction: take apps that are already good (Pantheon Signal, Image Studio, the original Claude for elementary OS), build them
out, unify their standards, and **freeze them as baselines**. The SDK versions are then improved, by passes of the SDK and an LLM
(a stronger model first), until a build made from a prompt reaches about **95 % of the custom app**. This note defines the method.
The measurement is the one the owner agreed to make at a decision point; it replaces the earlier "benchmark" idea with something
concrete: score a real app against a real reference.

## 1. Baselines (read-only references)

| Baseline | Source | Rule |
|---|---|---|
| **Claude for elementary OS (original)** | `rust/claude-pantheon-rs` (GTK, 13.8 k lines) with reference screenshots in its `docs/` | Frozen at a tagged commit; the SDK port (`sdk/t3.msdk`) is scored against it |
| **Pantheon Signal** | the Signal/Messages work (`mseries/signal-wingpanel`, PR #526) | **Deferred by the owner** ([MESSAGING-STATUS.md](MESSAGING-STATUS.md)). Freezing it is a snapshot, not a resumption: no build-out, no claim of completion |
| **Image Studio** | Codex's branch `codex/image-studio-20261022` (`mseries-image-studio-ui`) | Active work by another agent: snapshot a tag, never edit the branch |

A baseline is a directory `baselines/<app>/` holding, for each named **state** (for example "thread open with a permission prompt",
"settings", "empty"): the launch command and fixture data that reproduce it, a window size, theme (light and dark) and text
scale; screenshots; the measured layout (below); and the accessibility tree captured from the running app (AT-SPI), which is
comparable to `msdk render` output of the SDK version. Baselines never change; a new baseline is a new directory.

## 2. Unifying the standards

The baselines disagree in small ways (font size, sidebar width, paddings, header layout). Measure each from its screenshots and tree,
put the values side by side, and **the owner picks one value per standard**; that choice becomes the tokens (`msdk/data/tokens.json`).
This is how "font a little small" and "left column too wide" stop being opinions: base font, sidebar width, pane padding, control
height, title-bar height, corner radii and spacing each get a number with a source. The SDK defaults then come from the best of the
baselines instead of from guesses.

## 3. The parity score

`msdk parity <app> --baseline <name>` (to build) renders the SDK app in each baseline state with the same fixture data and compares:

| Part | How | Weight (initial) |
|---|---|---:|
| Structure | Roles, labels and order of the accessibility tree against `msdk render` output | 25 |
| Layout | Region sizes and positions (sidebar width, header height, paddings, row heights) within a tolerance, from the tree bounds | 25 |
| Typography | Font sizes and weights per role | 15 |
| Look | Structural similarity (SSIM) of downscaled screenshots, and colour distance, light and dark | 25 |
| Behaviour | The app's gates pass (`msdk gate`), including the built-in contrast gate | 10 |

Output is a number per state and a **diff report the model can act on**, for example "sidebar is 196 px, baseline 240",
"row title is 13 px, baseline 15", "missing: avatar in row", plus an annotated side-by-side image. The target is 95 or more in every
state and in both themes. Weights are a starting point to be tuned once there are scores.

## 4. The pass loop (prompt to compile)

1. Start from a template (`msdk new ...`) or a first generated description.
2. Run `msdk parity`; hand the model the baseline screenshot, the current screenshot, the diff report, and the discovery tools
   (`describe`, `complete`, `explain`). The model edits the `.msdk` file only.
3. `msdk check` and `msdk gate` must still pass; otherwise the pass is rejected.
4. Keep the pass if the score rose; stop at 95, or after three passes without gain.
5. Log every pass (prompt, edit, score, cost). Passes by a stronger model come first; the same loop run with a Qwen ~4B model later
   gives the real answer to "can the small model do it", measured against the same baselines.

Gaps the model cannot close (a missing component, service or layout prop) are recorded as SDK work, not hacked around in the
description; they are the work list for the SDK.

## 5. Status

| Piece | State |
|---|---|
| Render an SDK app to a PNG (`msdk-run --screenshot`, `CLAUDE_PANTHEON_SDK_SHOT`) | Built |
| Headless semantic tree (`msdk render`) | Built |
| SDK version of the T3 app on live server data (`claude-pantheon-sdk`) | Built (reads; sending and starting threads are coded but not exercised against the owner's accounts) |
| Baseline folders, AT-SPI capture, `msdk parity`, the pass loop | Not built |
| Owner decision of the unified values | Open |

## 6. Order

1. Freeze the T3 original as the first baseline (it has screenshots and runs against the real server), with a small set of states.
2. Capture its metrics and write the unified standards table; the owner picks values.
3. Build `msdk parity` (tree and layout first, SSIM after) and run the first pass by hand.
4. Freeze Image Studio and Signal as snapshots; add their states.
5. Automate the loop with a stronger model; then run it with the small model.
