# msdk (Phase 1 core)

The closed UI vocabulary of the M-series app SDK and `msdk check`. No GUI dependencies; builds in seconds.
Design and rationale: [mseries/PLATFORM-PLAN.md](../../mseries/PLATFORM-PLAN.md). Benchmark: [mseries/sdk-bench](../../mseries/sdk-bench/).

```bash
cargo run -p msdk -- check msdk/examples/quick-notes.ui.json   # one line per problem, with a JSON path, a code and a fix
cargo run -p msdk -- components                                 # the vocabulary as JSON (prompts, grammar generation)
```

Format v0 is JSON so a small model can be constrained to it with a grammar. An app description has `app`,
`capabilities`, `state` (text, bool, number, list), `messages` and a `window` built only from the components
listed by `msdk components`. `Custom` is the escape hatch and always raises a warning; each use is a vocabulary
gap to record in `mseries/sdk-bench/GAPS.md`.

Not here yet: the `logic.rs` side (State/Msg/update), the runtime that renders the tree (Relm4 spike or own
runtime on gtk4-rs + granite-rs), `msdk shot`, `msdk pack`.
