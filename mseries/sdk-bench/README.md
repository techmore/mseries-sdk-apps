# SDK benchmark (Phase 0 of [PLATFORM-PLAN.md](../PLATFORM-PLAN.md))

Purpose: a fixed set of app specs and one scoring rubric, so we can say with numbers whether the SDK,
a different model, or the repo split made app building faster. Nothing here depends on the SDK
existing: the same specs are run today against the current process (gtk4-rs + granite-rs by hand or by
agent) to produce the "before" column.

- [RUBRIC.md](RUBRIC.md): what is recorded per run and how screenshots are judged.
- [BASELINE.md](BASELINE.md): what is known about current cost, what is not, and how to measure it.
- [specs/](specs/): six one-page app specs, ordered easy to hard. Each names the existing app that is its golden reference.

Rules: a spec is never edited after a run has been recorded against it (add `-v2` instead);
a run records the model, runner, date and commit; failed runs are kept.
