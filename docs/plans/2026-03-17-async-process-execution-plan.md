# What problem does this plan solve?

`nao` needs a cross-platform way to execute parallel tasks and stream their output without dedicating a thread per process just to write logs. Linux and Windows have different low-level pipe behavior, so the implementation should hide those details behind the platform abstraction instead of spreading Tokio or OS-specific process code through the engine.

This plan describes how to add async process execution while keeping the rest of `nao-engine` synchronous and runtime-agnostic.

# What is the current status?

This plan is active.

The repository currently has:

- a `nao-engine` crate that can load recipes and plan runs
- a `nao-pal` crate that abstracts filesystem access, directory walking, file watching, and clocks
- a synchronous PAL process execution API with raw output events
- a Tokio-backed `PalReal` implementation for process execution
- an engine-side output framer and sequential execution path
- no run artifact persistence yet beyond rendered CLI output

# What implementation approach should be used?

Tokio should be introduced as an implementation detail of `nao-pal`, not as a dependency that leaks into `nao-engine` or higher-level domain types.

The intended layering is:

1. `nao-engine` describes what should be executed and consumes a synchronous PAL-facing execution API.
2. `nao-pal` owns process spawning, stdout and stderr collection, and platform-specific async I/O.
3. `nao-pal::pal_real::PalReal` uses one shared Tokio runtime internally to drive child process I/O on Linux and Windows.
4. `nao-pal::pal_mock::PalMock` provides deterministic test doubles without requiring Tokio.

This keeps runtime choice and OS-specific behavior at the platform boundary while allowing the engine to stay focused on task graph orchestration and run state transitions.

# What PAL API should be exposed for process execution?

The PAL should expose process execution in terms of domain-relevant events and results rather than futures, streams, or Tokio handles.

The initial process API should likely support:

- spawning a task process from a fully prepared command description
- receiving stdout and stderr output as synchronous sink-delivered events during process execution
- waiting for process completion and retrieving exit status
- timestamping output and lifecycle events via `Pal::now()`

The PAL API should avoid exposing:

- Tokio types such as `Runtime`, `JoinHandle`, `AsyncRead`, or channels
- direct writable stdin handles for child processes in the first implementation
- engine-visible async traits or `async fn` requirements

One reasonable first shape is a synchronous method that blocks until process completion while invoking a callback or sink trait for output and lifecycle events. That gives the engine a simple call boundary while still allowing `PalReal` to implement the internals with async pipe readers.

# How should events be transported between the PAL and the engine?

The public PAL boundary should stay synchronous.

That means:

- the engine calls a blocking `Pal` process execution method
- the engine passes a mutable event sink or callback
- `PalReal` invokes that sink directly as process events arrive
- the process execution call returns only after output draining and process completion are finished

Async communication should exist only inside `PalReal`.
The real PAL implementation may use Tokio tasks and an internal async channel to fan in stdout, stderr, and exit notifications before forwarding them synchronously to the engine-visible sink.

This split keeps:

- Tokio out of the engine API
- process event transport explicit and easy to test in `PalMock`
- ordering policy centralized inside the real PAL implementation

# How should async process execution be handled inside `PalReal`?

`PalReal` should create and own one Tokio runtime for process execution work. The runtime should remain private to the PAL implementation and should be shared across all process executions owned by the PAL instance.

The runtime-backed implementation should:

- spawn child processes with piped stdout and stderr
- use Tokio readers for both streams
- forward raw stdout and stderr chunks rather than doing line framing in the PAL
- use an internal async channel or equivalent fan-in mechanism to combine stdout, stderr, and exit events
- emit process events to the calling sink in a deterministic order defined by arrival at the fan-in point
- wait for stream draining and child completion before returning

The log-writing path should not create one dedicated writer thread per process. Instead, the PAL execution path should fan in stdout and stderr events and let the higher-level run execution code perform line framing and write per-task log files and `nao-events.jsonl` in a controlled way.

This allows the system to preserve:

- per-task log isolation
- consistent timestamping
- a single event model for both real-time updates and persisted run artifacts

# Why should the PAL avoid line framing?

The PAL should be kept as small and policy-free as practical.
Line framing is part of log formatting and output persistence policy, not part of platform interaction.

The PAL should therefore emit raw chunk events such as stdout bytes and stderr bytes, each tagged with a timestamp.
An engine-side output collector or log framer should own:

- per-task stdout buffering
- per-task stderr buffering
- splitting buffered bytes into lines
- handling partial trailing lines when a stream closes
- rendering timestamp-prefixed log lines into run artifacts

This keeps the PAL focused on process I/O and time while allowing the run artifact layer to evolve independently.

# How should the engine interact with the PAL without depending on Tokio?

`nao-engine` should treat process execution as a blocking platform service call that reports structured events. The engine should not manage async runtimes or spawn Tokio tasks directly.

That means the engine can:

- decide when a task is eligible to start
- request that the PAL execute one task process
- update run state in response to emitted raw output and completion events
- persist run files using ordinary synchronous logic or small focused writer abstractions

If the engine later needs real parallel task execution, it can use ordinary Rust threads or a small coordinator to invoke multiple PAL executions concurrently. Tokio should still remain hidden inside each real PAL-backed process execution call.

# What command and event types should be introduced first?

The first implementation should add explicit types for process execution instead of passing loosely structured strings around.

The initial model should likely include:

- a process command spec with executable path, arguments, working directory, and environment
- a process event enum for process start, stdout chunk, stderr chunk, stream close, and process exit
- a process result type that includes exit code and timing information

These types should live in small focused files, consistent with the repository file-organization rules.

# How should logging be structured if process writers are not yet required?

The first version should treat child stdin as out of scope. Most build, test, and lint tasks can run unattended, and excluding process writers keeps the PAL API smaller and the cross-platform process model easier to validate.

That means the initial design should:

- support read-only observation of stdout and stderr
- omit stdin piping or interactive process control
- document stdin support as deferred follow-up work

Run logging should be driven by the engine or a dedicated execution component above the PAL, not by the PAL writing files directly. The PAL should report what happened; the engine layer should decide how raw chunk events become framed log lines and `.nao/runs/...` artifacts.

# How should the work be ordered?

The recommended implementation order is:

1. Add plan and process-domain types for PAL-facing execution requests, events, and results.
2. Extend the `Pal` trait with a synchronous process execution entrypoint.
3. Implement the new API in `PalMock` with deterministic scripted events and exit codes.
4. Add Tokio as a dependency of `nao-pal` and implement real process execution in `PalReal` with one shared runtime.
5. Add a small engine-side execution component that consumes PAL process events and updates run state.
6. Add an engine-side output framer that turns raw chunk events into timestamp-prefixed task log lines.
7. Add per-task log writing and run event persistence above the PAL.
8. Wire the CLI or engine entrypoint to execute planned tasks through the new engine path.
9. Reconcile [`docs/NAO-SPEC.md`](../../NAO-SPEC.md) with the implemented event and log format details.

# What concrete work items should be tracked?

- [x] Add focused process execution domain files to `crates/pal/src/`.
- [x] Extend `crates/pal/src/pal.rs` with a synchronous process execution API.
- [x] Define a sink trait or callback shape for PAL-to-engine process events.
- [ ] Document in code why Tokio remains a PAL-only detail if that is not obvious from the public API.
- [x] Add `PalMock` support for scripted process execution events and results.
- [x] Add Tokio dependencies to `crates/pal/Cargo.toml`.
- [x] Add one shared Tokio runtime to `PalReal` rather than constructing a runtime per process execution.
- [x] Implement `PalReal` child process spawning with piped stdout and stderr.
- [x] Implement concurrent stdout and stderr draining with Tokio readers.
- [x] Preserve non-UTF-8-safe output handling by reading raw bytes without line framing in the PAL.
- [x] Return structured process completion data including exit code and timing.
- [x] Add an engine-side executor component in `crates/engine/src/`.
- [x] Add an engine-side output framer that converts chunk events into timestamp-prefixed log lines.
- [x] Add engine tests for event handling, task completion, and failure propagation.
- [ ] Add PAL tests for process event sequencing in both real and mock implementations where practical.
- [ ] Add run-log and event persistence work that maps execution events into `.nao/runs`.
- [ ] Update [`docs/NAO-SPEC.md`](../../NAO-SPEC.md) if implementation details refine the file format.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated unit tests for new PAL process domain types
- `PalMock`-based engine tests for task success, failure, and output event handling
- tests for engine-side line framing from raw stdout and stderr chunks
- PAL-level tests for `PalReal` process execution behavior where deterministic fixtures are practical
- tests that cover partial final lines, interleaved stdout and stderr, and non-zero exit codes
- tests that verify the engine can consume process events without depending on Tokio types
- running `./scripts/check-code.sh`

If Windows-specific behavior cannot be covered reliably in the default test environment, that limitation should be stated explicitly in the implementation notes and CI strategy.

# What assumptions should remain explicit?

This plan assumes:

- Tokio is acceptable as a dependency of `nao-pal`
- the rest of the workspace should not depend on Tokio for ordinary engine logic
- the first process execution slice does not need interactive stdin support
- run artifact writing should stay outside the PAL
- line framing should stay outside the PAL
- a synchronous PAL API with internal async implementation is sufficient for the first milestone

# What risks or open questions matter most?

The main risks are:

- designing a PAL process API that accidentally leaks Tokio concepts upward
- making output framing too line-oriented and losing important raw-byte behavior
- over-coupling log persistence to process execution
- under-specifying ordering guarantees between stdout, stderr, and lifecycle events

The main open questions are:

- whether timing should be captured entirely through `Pal::now()` or partly from runtime events
- whether engine-level parallelism should initially use threads, a small worker pool, or another coordinator model above the PAL
