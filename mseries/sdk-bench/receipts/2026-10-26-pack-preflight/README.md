# Packaging and quit preflight — 2026-10-26

Harness source commit: `019aee19d70841806f46b66da0a40c0b649bf073`.
The SDK-mode fixed tool surface ran list, new, gen, offline build, pack and
launch successfully as uid 2000 in the isolated loopback-only Incus sandbox.
The generated package metadata and SHA-256 are recorded; the binary package is
not included. No package installation or removal was performed in this test.
The positive capture reported a clean Ctrl+Q. A separate trusted GTK fixture
that ignores Ctrl+Q was captured and the probe returned nonzero, with
`ctrl_q_clean: false`. Frame content remained pending independent review.

The source and frames are synthetic. The project's owned scratch and state
were cleared after retaining these artifacts. This preflight is not a model
trial, a baseline comparison, or acceptance of the frozen specification.
