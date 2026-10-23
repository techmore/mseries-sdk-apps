# Scoring rubric

## Recorded for every run

| Field | Meaning |
|---|---|
| spec, model, runner, host | e.g. `s03`, `Qwen3.5-4B`, `llama.cpp`, `ser8`. Model runs go on SER8, not the laptop |
| mode | `baseline` (today's process) or `sdk` (with `msdk`) |
| t_first_window | minutes from start to a window that opens and shows real content |
| t_done | minutes to the "done" bar below, or `not reached` at the 120 min cap |
| attempts / retries | model turns, and how many were fixes after a tool error |
| tokens in / out | from the runner |
| human interventions | times a person had to step in; each counts as a failure of autonomy |
| result commit | the commit or tarball of what was produced |

## Done bar (same for every spec)

An app is "done" when all of these hold:

1. Builds with no warnings in the repo's CI settings (`cargo fmt --check`, `clippy -D warnings`).
2. Launches, shows the spec's main screen with real or fixture data, and quits cleanly.
3. Every acceptance line in the spec passes (each is written to be checkable by script or by looking).
4. Keyboard: Tab order reaches every control; the spec's shortcuts work.
5. Accessibility: every interactive element has a name and role (checked from the accessibility tree).
6. Works in light and dark, and at 1.0x and 1.5x text zoom without clipping.
7. Installs as a `.deb` with a correct `.desktop` file, icon and dock identity.

"50 %" in the owner's phrase is reported as the fraction of the spec's acceptance lines passing at a
given time, plus whether items 1 to 7 hold. Record both: the fraction and the time it was reached.

## Screenshot judging

Same rubric every time, scored 0 to 2 per line (0 wrong, 1 acceptable, 2 matches the elementary HIG):
layout and spacing, typography hierarchy, colour and contrast, dark-mode correctness, empty/loading/error
states, icon use, controls look native. Judge is a human or a vision model with the rubric and the
golden screenshot; record which. Never mix judges in one comparison.

## Reading the results

- Compare `baseline` and `sdk` on the same spec and the same model.
- A change is "better" only if t_done falls and the done bar still holds; faster but sloppier is a fail.
- Track "vocabulary gaps": every time an `sdk` run needed a `Custom` escape hatch, add a line to
  `GAPS.md` (created at the first run) naming the missing component.
