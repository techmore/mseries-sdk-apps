# SDK service and process contract

Status: implemented candidate contract for SDK 0.1.0; whole-SDK v1 qualification
remains tracked in SDK-COMPLETION.md. Inspect `msdk describe runtime` for the
implemented version, limits and failure codes. `msdk explain CODE` gives the
recovery message for runtime failures as well as checker diagnostics.

## Typed service extensions

An app supplies `msdk_gtk::host::Services` to `Host::open_with`. The primary entry
point is `invoke(ServiceCall) -> Result<bool, ServiceFailure>`. A request carries
the declared service name, arguments and an owned `ServiceReply`:

- Return `Ok(true)` once the request is owned, including asynchronous work. Keep
  its reply in that owned task and call `reply.success(payload)` or
  `reply.fail(failure)` on completion.
- Return `Ok(false)` only before doing any work. This permits another runtime
  adapter to handle the operation. `Err(failure)` is an owned failure and never
  falls through to a real system action.
- Registered service return payloads are validated before delivery. Wrong
  text/bool/none shapes become `service-malformed-data`, without logging the
  offending value. Declare all supported APIs through a service manifest.
- The first terminal reply wins, even across cloned replies and worker threads.
  Contradictory duplicate completion is ignored. Replies do not retain GTK.
- The Host assigns immutable service/call-source/operation identity to typed
  replies and errors. Rules calls use their `/update/Message` JSON pointer;
  custom Rust logic uses `/logic`. Operation IDs are host-local diagnostics,
  not account identity or a substitute for context/race checks.
- `Services::stop` must retire the provider's own subscriptions, timers, workers
  and pending requests. Its empty default is appropriate only when there is no
  ongoing work. Closing an accepted window stops host-owned work; a vetoed close
  keeps the Host usable. Late events cannot mutate a disposed Host.

`Services::api_version()` defaults to the existing extension interface version 1.
The Host checks it before call/feed dispatch and refuses incompatible versions.
This is the SDK extension contract, **not** a declaration that an OS daemon,
D-Bus interface, T3 server or provider API uses version 1. Each real backend must
publish and negotiate its own supported protocol/version and authority boundary.
Those backend and T3 qualifications remain open.

Existing `call`, `call_checked` and `start_feed` implementations remain compatibility
entry points. New service adapters should use `invoke`/`ServiceReply`. Legacy
raw event senders retain their own payload-validation and completion obligations;
compatibility does not qualify an old adapter automatically. Generic context
switch cancellation and app-specific adapter migration remain open gates.

### Typed GTK reply contexts

Before starting asynchronous work, bind its reply with
`let reply = call.reply.in_context(token)?;`. All clones share both the binding
and first-completion state, including the Host's retained clone used for a
synchronous `Err` from `invoke`. Binding twice, or after completion, returns a
typed invalid-input failure. A request cannot be rebound to a newer account to
make an old result current.

Binding also claims the request's context ownership. A provider that binds and
then returns `Ok(false)` receives an owned unavailable failure; the Host must not
fall through to a real unscoped built-in action. Decline unowned service names
before binding. Completing a reply and then declining likewise cannot execute
fallback work.

The Host retains the token across its typed delivery queue and checks it on the
GTK thread immediately before applying a success or reporting a failure. An
error callback retains that same token across its additional queue boundary.
This matters when an error is current when received, but the app switches
context before its callback runs. Such a callback must not overwrite the new
page with an obsolete error. The first-terminal-reply rule remains shared
across worker clones, and replies/queues do not retain the Host or GTK widgets.

Keep `RequestContext::advance`/`close` and UI state application serialized on
that state-owner thread. Retire provider-owned workers/subscriptions and close
the provider's contexts in `Services::stop`. Scoped reply delivery prevents
stale state mutation; it does not itself cancel a network operation or stop
polling. Use scoped process ownership or the backend's explicit cancellation.

Native private-session regression checks cover queued old success and failure,
shared binding on synchronous provider error, a context switch between failure
and callback delivery, valid current recovery, visible GTK label updates,
rebinding rejection, provider stop and Host release. These are explicit opt-in
reply bindings. They do not automatically scope legacy raw event senders,
legacy feeds, built-in service dispatch, extension negotiation before provider invoke,
streaming chunks, or T3/network adapters. The separate Rust scoped-stream API
below is opt-in; those GTK and app/backend gates remain open.

## Context-bound feeds

Providers can override `Services::start_feed_typed` and bind the `FeedReply`
to an app-owned `ContextToken` before starting a subscription. Deliver each
update with `push` on the GTK/state-owner thread; it checks freshness immediately
before applying the update. A bound endpoint cannot be rebound. Keep the
unbound endpoint only for creating explicitly new subscriptions, and retire
old subscriptions and reads when the context changes or the Host closes.
Binding filters stale updates; it does not cancel backend work. Legacy
`start_feed` providers continue to work without automatic context scoping.
The app remains responsible for displaying account/scope identity.

## Errors and privacy

`msdk::runtime::ServiceFailure` carries a `FailureKind`, service, source context,
operation ID and optional numeric version information. Categories distinguish
unavailable, incompatible, denied, invalid input, malformed data, I/O, failed
process, timeout, output limit, cancellation and busy conditions.

`diagnostic()` emits `msdk.runtime.diagnostic.v1`. Its fields contain source
identifiers, numeric identity and a static recovery message. Command arguments,
stdout/stderr, private backend payloads and credentials are excluded. Debug and
Display use the same safe metadata/message policy. `with_detail`/`detail()` are
an explicit private-UI channel; never copy that detail to public logs/receipts.
A registered error callback receives the recoverable UI message; without one,
the Host shows its error banner. The app must provide useful Retry/Cancel states.

The fixture provider uses this same typed route. A missing reply is an explicit
unavailable error and cannot execute a real fallback. A malformed fixture reply
is rejected. Fixture tracing records the service identifier, not its arguments.

## Owned process work

### App-owned request contexts

`msdk::context::RequestContext` represents one app-owned account/page/selection
lifetime. Create one for the domain whose pending results must expire together.
Keep its owner on the UI/state thread and give workers only a `ContextToken`.
The app still owns and displays the actual account/scope identity; the SDK token
contains no account information or credentials and grants no authority.

On an account/page/selection change, call `advance()` before issuing replacement
work. It invalidates all old tokens and returns a new token. Returning to a
previous account cannot revive its original token. `close()` permanently retires
the context; it cannot reopen. Dropping the last cloned owner also invalidates
tokens, while tokens and queued replies do not keep that owner alive.

Use `token.wrap(value)` for asynchronous replies. Forward the resulting
`Scoped<T>` through the app's queue intact, then call `into_current()` on the
state-owner thread immediately before applying. Both old successes and old
errors return None. Checking freshness only on the worker is insufficient: a
previously completed reply may wait in a queue until after the account changes.
Debug omits the private value, without requiring T to implement Debug.

Context changes and consuming/applying replies must be serialized on the same
state-owner thread. `into_current()` is a freshness snapshot, not a transaction
locking arbitrary app mutations on other threads. Never unwrap a reply on the
worker and enqueue the raw value as if it remained current indefinitely.

`msdk::process::spawn_scoped(token, command, limits, operation, complete)` binds
completed-output work to that token. It checks freshness before starting and
alongside the runner's normal cancellation/deadline checks. Advancing, closing
or dropping the owning context causes owned task cleanup and leader reaping.
Its callback receives `Scoped<Result<Output, ServiceFailure>>`; forward that
envelope to the state owner for final delivery. This cancels task work; it is
not rollback for effects already performed by a command.

`spawn_streaming_scoped` combines the same cancellation with a bounded
`ScopedOutputStream`. Its receive methods return `Scoped<Chunk>`; retain each
envelope through any UI queue and call `into_current()` on the state-owner
thread immediately before applying the bytes. Its completion is scoped too.
Dropping the subscription cancels owned work, and the usual queue/output limits
apply. This API does not automatically scope GTK feeds or external subscriptions.

Core tests cover queued/out-of-order success and error, independent contexts,
return-to-account invalidation, last-owner drop, permanent close, an already
closed request that never executes its command, and actual process cancellation/
reaping followed by a successful new request. GTK typed ServiceReply supports
the explicit binding described above. App-specific T3/network subscriptions are
not automatically scoped. Their adapters must carry
tokens through final delivery and retire their own reads/resources; full app
context-switch qualification remains open. Do not treat core or synthetic Host
results as that broader lifecycle acceptance.

The GUI-independent Rust API is `msdk::process::{spawn, Limits}`. Hold the returned
`Process` for the lifetime of the operation. `cancel()` or dropping an unfinished
handle requests cancellation of its dedicated process group. The worker reaps
its leader before completion. Group signals finish before the leader is reaped,
with cancellation serialized, so PID reuse cannot target a later process.

Implemented on Linux and macOS; unsupported platforms return a typed unavailable
failure. `/bin/sh -c` runs trusted app-authored task commands with null stdin and
nonblocking stdout/stderr pipes. Defaults are:

| Limit | Behavior |
| --- | --- |
| 120 seconds | Monotonic deadline; timeout stops the owned group |
| 1 MiB combined stdout/stderr | Exceeding the byte budget stops the group |
| 4 concurrent Host jobs | Further calls receive a recoverable busy error |
| Rust `Limits` override | Nonzero timeout/output; output cannot exceed 16 MiB |

`process.run` uses those defaults and delivers completed stdout to its `then`
message. Nonzero exit is a typed failure with stderr available as private detail.
Timeout/output-limit/cancellation are distinct from success. Inherited pipes do
not keep the worker waiting after the leader exits; same-group descendants are
retired. The Host clears owned handles on disposal.

This is task-command ownership, not a sandbox or supervisor for privileged,
daemonized, detached-session or OS-wide transactions. Use an explicit OS-owned
service adapter for those lifecycles; do not route a long-running update
transaction through a short task timeout. The declarative `process.run` service
still returns completed output; progress/update UI and the updater reference-app
contract remain unfinished.

### Rust streaming and partial output

`msdk::process::spawn_streaming` returns `(Process, OutputStream)`. Retain both
while the operation is wanted. Drain the stream on an app worker (never block a
GTK main loop), forwarding only needed updates to the UI. Dropping either owner
cancels silent or producing work. Cancellation and completion retain the same
process-group cleanup and leader-reaping guarantees as `spawn`.

Each `Chunk` identifies stdout/stderr and contains raw bytes. Byte order within
each stream is preserved; cross-stream order is unspecified. Chunks may split
lines or UTF-8 sequences: keep a separate decoder/buffer per stream, and choose
an explicit policy for invalid bytes. Chunk Debug shows stream and byte count,
never private content. Raw bytes and the final Output belong in private app UI,
not public diagnostics or receipts.

The queue holds at most 32 chunks of at most 8192 bytes each (256 KiB), in
addition to the normal bounded final-output buffers. A full queue stops the
command with `process-output-limit`; the worker never waits for a slow consumer.
Already received and queued bytes remain available after timeout, cancellation,
nonzero exit or output-limit failure. Output that exceeded either bound is not
silently accepted. The completion callback is authoritative for terminal status;
stream closure alone does not mean success. The sender closes before completion
is reported, so consumers can then drain queued partial data to disconnection.
The callback runs after process cleanup; keep it short and nonblocking.

Discovery keeps `process.streaming: false` for the completed-output description
service and reports the separate Rust streaming API/limits in
`process.rust_streaming`. This core capability does not qualify a GTK progress
adapter, stale-context handling or the updater app. Those remain open.

## Verification

Core real-process checks cover output/nonzero exit, timeout, combined byte limit,
flood output, inherited pipes, drop cancellation, leader reaping and same-group
child cleanup, invalid limits and invalid command arguments. Diagnostic tests
cover privacy, version mismatch, actual default-limit discovery and explanations.

The native private-session test uses the actual Host dispatch, worker threads
and GTK state: incompatible provider is not invoked; supported version recovers;
unavailable service, malformed async reply and permission denial enter recovery;
valid typed replies recover; duplicate completion is ignored; missing fixture
reply cannot create a marker through fallback; accepted close kills a real owned
command. Run with a fresh Xvfb/D-Bus session and synthetic XDG state:

```sh
MSDK_PRIVATE_TEST_BUS=1 XDG_DATA_HOME=/tmp/msdk-runtime-state GSK_RENDERER=cairo \
  xvfb-run -a dbus-run-session -- cargo test --manifest-path SDK/Cargo.toml \
  -p msdk-gtk --test lifecycle --test runtime -- --test-threads=1
```

Use the isolated workspace from `mseries/sdk-tests/prepare.py`. Retain source,
locks, commands, results and failures separately from owner/physical-host
acceptance. These tests do not close the remaining SDK completion gates.
