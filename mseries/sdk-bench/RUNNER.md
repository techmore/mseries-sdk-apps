# Controlled SDK authoring discovery

`run-model.py` is an SDK-mode discovery runner for the existing frozen specs.
It is not a completed baseline comparison or a claim that every spec passes.
The first cohort uses s01 unchanged. This validates a representative authoring
flow, not a native port/replacement of an existing product.

The controller runs on SER8 against its existing local Ollama service. It pins
the installed qwen3.5:4b manifest digest, packaged license, runtime/options,
prompt/spec hashes and source revision. It refuses an occupied model service at
admission; it does not download a model, restart/switch a daemon or use a second
GPU worker. Requests, responses, token counts, timing and every tool result are
retained. Only synthetic/public SDK data enters the model. Lost inference/tool
observations stop dispatch, rather than treating a timeout as proof of stopped
upstream work and retrying blindly.

Generated Rust executes in a separate ephemeral clone of the SDK test container,
as uid2000 with zero effective/ambient capabilities and only loopback networking.
The configured limits are 4 CPUs, 6GiB RAM, 256 processes and a 16GiB root volume.
There are no owner-data mounts. The SDK and runner remain root-owned/read-only
to that user. Provision trusted dependencies/cache while networked, then remove
the inherited NIC before any model code runs. Preparation, downloads and cache
warmup are outside the model authoring timer and must be reported separately.

The model can write bounded project source files, read its project/public SDK,
and invoke a fixed command surface. No arbitrary host command tool is exposed.
Builds are offline. The SDK-mode `pack` command packages a built Cargo binary
into the owned evidence directory; it does not install or remove the package.
The `gate` command accepts an optional `gates` path for acceptance scenarios;
without it, only the description's built-in gates are run.
Installation, removal and the remaining acceptance criteria require independent
verification. `worker.py` rejects parent/absolute/hidden paths, writes to
runner evidence/build output, oversized files and unknown commands. Code-level
side effects are contained by the separate unprivileged offline instance, not
by claiming a Rust compiler is a sandbox. Discard the owned instance after the
cohort is retained; never reuse untrusted trial state for a comparison run.

`capture.py` is a shared external window/capture/Ctrl+Q probe. Handwritten code
does not need a private screenshot flag. The model receives the latest actual
frame for iterative feedback; independent review still judges the result. The
probe returns failure if Ctrl+Q does not close the captured application cleanly.
Mapped/captured timestamps are candidate first-window evidence. Do not report
them as a real-content first window until the image is reviewed. A `done`
action merely submits a candidate: `t_done` stays not reached until every frozen
spec acceptance and every rubric done-bar item is independently verified.

Keep all failures, repairs, escapes to app-local Rust, vocabulary gaps and
interventions. Baseline and repeated-cohort comparisons remain open; the
worker's baseline tool restriction is preparation, not a qualified baseline
runner. Never compare an SDK discovery run with a differently provisioned or
unmeasured native run and call the speed difference a result.

Before inference, exercise the fixed settings smoke through new/gen/build/
pack/launch, verify native suites at the benchmark UID, archive that preflight, then
reset only its owned project/state directories. `run-model.py` requires an empty
project and refuses to overwrite an output directory. Example on the controlling
host after provisioning and verification:

```sh
python3 run-model.py --container msdk-bench-20261026 \
  --spec specs/s01-quick-notes.md --source-commit VERIFIED_COMMIT \
  --output /tmp/msdk-authoring-s01-FIRST_UNIQUE_RUN
```

Retain the exact source/seed lock/environment with the run. The controller stops
at the 120-minute wall cap or its declared turn budget. A turn-budget stop is
not evidence the model failed the whole 120-minute task. Resume only after the
specific prior process/request is authoritatively terminal; this first runner
does not implement automatic interrupted-run resumption.
