# What problem does this plan solve?

`nao` can currently plan dependency graphs but still executes the planned tasks strictly sequentially. That leaves independent tasks such as `fmt`, `clippy`, and unrelated sample tasks unable to run at the same time even when the dependency graph would allow it.

This plan describes how to add concurrent task execution while preserving deterministic planning, clear failure behavior, live terminal updates, and `.nao/runs` artifacts.

# What is the current status?

The repository currently has:

- a planner that expands task selectors, validates dependency order, and produces one topologically ordered task list
- a synchronous engine execution loop in `crates/engine/src/run_engine.rs`
- a PAL abstraction that can execute one process while streaming process events through a sink
- a run observer hook that reports task lifecycle events to the CLI
- run artifact writing that persists the final plan, task logs, event stream, and summary after execution completes
- a live display that can show either a single aggregate line or one line per task during execution

The main missing pieces for concurrency are:

- a scheduler that can start multiple ready tasks instead of only the next task in a single vector
- a way to run multiple PAL-backed task processes at the same time
- run-state tracking that can reconcile parallel task starts, completions, failures, and skipped tasks
- artifact/event ordering rules that stay understandable when tasks overlap in wall-clock time

# What implementation approach should be used?

Concurrent execution should be implemented as an engine-owned scheduler above the PAL.
The PAL should continue to own process execution details for one task, while the engine should decide:

- which tasks are ready to start
- how many tasks may run at once
- when the run should stop launching new work after a failure
- how final task state is recorded for completed, failed, running, ready, and skipped tasks

The first implementation should prefer correctness and explicit run-state modeling over maximum throughput.
It should support bounded concurrency with a configurable worker limit, but it does not need speculative scheduling, cancellation of already running tasks, or dynamic resource heuristics in the first slice.

# Why should the scheduler be added in the engine instead of the PAL?

The PAL knows how to execute one process and surface its process events.
It does not know task dependencies, recipe semantics, failure policy, or run artifact structure.

Those concerns already live in `nao-engine`, so the concurrency coordinator should also live there.
That keeps:

- dependency resolution in one place
- failure and skip policy consistent between sequential and concurrent runs
- terminal display updates driven by task lifecycle events instead of PAL internals
- the PAL API focused on process execution rather than task graph orchestration

# What execution model should the first concurrent version use?

The first concurrent version should use a ready-queue scheduler with a bounded worker pool.

The model should be:

1. Build a run graph from the planned tasks, including reverse dependency edges and a remaining-prerequisite count per task.
2. Seed a ready queue with tasks whose prerequisites are already satisfied.
3. Start up to `max_parallel_tasks` ready tasks.
4. When a task finishes successfully, decrement the remaining-prerequisite count of its dependents and enqueue any newly ready tasks.
5. When a task fails, stop launching new tasks, allow already running tasks to finish, and mark not-yet-started reachable tasks as skipped.
6. Persist the final state of every planned task and return the same high-level run result shape used today.

This execution model fits the current task semantics, keeps scheduling decisions easy to test, and avoids prematurely introducing cancellation or retry behavior.

# How should concurrency limits be configured?

The implementation should add recipe-level execution configuration for the maximum number of concurrent tasks.
The initial configuration should likely be a small integer property such as `config max-parallel-tasks=4`.

The first version should use these defaults:

- default to `1` when the property is omitted so existing behavior is preserved until concurrency is explicitly enabled
- reject values lower than `1`
- treat the configured value as a limit on concurrently running task processes, not on planned tasks or goal selectors

This keeps the rollout safe and makes it easy to compare sequential and concurrent behavior during testing.

# How should run state be represented?

The current engine records task state only as it iterates one task at a time.
Concurrent execution needs an explicit task-run state model.

The scheduler should track, at minimum:

- `Pending`: planned but not yet ready because prerequisites are unfinished
- `Ready`: eligible to start once worker capacity is available
- `Running`: process started and not yet finished
- `Completed`: exited successfully
- `Failed`: exited with a failure or execution error
- `Skipped`: not started because an earlier failure prevented further scheduling

This state model should drive:

- terminal display rendering
- artifact status fields
- event emission
- final failure-summary calculations

# How should task processes be executed concurrently?

The engine should execute concurrent tasks by starting one worker per running task above the PAL boundary.
Each worker should call the existing PAL process execution API for one task and report the result back to the scheduler over a channel.

The first version should avoid changing the public PAL interface unless implementation constraints force it.
That means the scheduler can likely use:

- one engine-owned thread per running task process
- a channel for task completion and lifecycle notifications
- the existing `TaskOutputFramer` logic inside each worker

This is a pragmatic first step because the PAL already hides Tokio and OS-specific process handling.
If thread-per-running-task overhead becomes a real issue later, that can be optimized as follow-up work after the scheduler semantics are stable.

# How should live terminal updates work during concurrent execution?

The current live display already understands task lifecycle events, but it assumes only one task can be running at a time.
Concurrent execution should extend the observer protocol so the CLI can render multiple running tasks correctly.

The first version should support:

- multiple tasks simultaneously showing a spinner/running state in `LinePerTask`
- completed, failed, and skipped statuses updating independently as workers report progress
- an aggregate `SingleLine` mode that summarizes counts such as running, completed, and remaining tasks instead of pretending there is only one active task

The display should remain engine-driven through task lifecycle notifications rather than by reading PAL process output directly in the CLI.

# How should artifacts and event ordering behave when tasks overlap?

Per-task log files should remain isolated and should still be written one file per task.
That part does not require a global output merge.

For run-wide artifacts:

- `nao-plan.json` should still describe the full planned run before execution starts
- `nao-events.jsonl` should include task start and finish events in actual observed order
- `nao-summary.json` should reflect each task's final status and timestamps regardless of overlap

The implementation should document that event ordering is by scheduler observation time, not by topological task order, once tasks overlap.
Tests should cover overlapping tasks to ensure event streams remain valid and understandable.

# How should failure behavior work?

The first concurrent version should use fail-fast scheduling without force-killing already running tasks.

That means:

- the scheduler stops launching new tasks after the first failure
- already running tasks are allowed to complete naturally
- tasks that never started and still depend on unfinished or failed work become `Skipped`
- the overall run fails with the first observed task failure that triggered scheduling shutdown

This avoids introducing process cancellation semantics in the same change as the initial scheduler.
If explicit cancellation is desirable later, it should be planned separately because it changes PAL requirements and artifact expectations.

# What implementation order is recommended?

The recommended order is:

1. Add a plan document for concurrent execution.
2. Introduce engine-side run-state types for per-task scheduler status.
3. Refactor the current sequential executor into a smaller execution component that can be reused by both sequential and concurrent scheduling paths.
4. Build a scheduler data model with prerequisite counts, dependents, ready queue handling, and bounded worker accounting.
5. Add a concurrent execution path that launches one worker per running task and receives completion messages over a channel.
6. Preserve the existing sequential mode as the behavior used when `max-parallel-tasks=1`.
7. Extend run observer events or observer state updates so the live display can show multiple running tasks.
8. Update run artifact writing and summary generation as needed for overlapping task timestamps and statuses.
9. Add recipe parsing and validation for the concurrency limit configuration.
10. Update docs to describe concurrent execution semantics, configuration, and failure behavior.

# What concrete work items should be tracked?

- [ ] Add a new recipe config property for maximum concurrent tasks.
- [ ] Add parser validation and defaults for the concurrency limit.
- [ ] Introduce focused engine types for scheduler task state and scheduler bookkeeping.
- [ ] Refactor task execution so one task run can be invoked independently from the outer run loop.
- [ ] Build dependency metadata for planned tasks, including reverse edges and remaining prerequisite counts.
- [ ] Add a ready queue and bounded worker scheduling loop to `nao-engine`.
- [ ] Execute multiple tasks concurrently using engine-owned worker threads above the PAL boundary.
- [ ] Record per-task started, finished, failed, and skipped state in a way that supports overlapping task lifetimes.
- [ ] Preserve final output framing and per-task log file generation for concurrent runs.
- [ ] Update `LinePerTask` live rendering so multiple tasks can be running at once.
- [ ] Update `SingleLine` live rendering to summarize concurrent progress instead of assuming one active task.
- [ ] Add engine tests for independent tasks running concurrently.
- [ ] Add engine tests for dependency-gated scheduling where dependents start only after prerequisites finish.
- [ ] Add engine tests for fail-fast scheduling with already running tasks allowed to complete.
- [ ] Add engine tests for skipped-task accounting after a concurrent failure.
- [ ] Add artifact tests for overlapping task start and finish events.
- [ ] Add parser tests for the new concurrency config property.
- [ ] Update recipe/config documentation.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated recipe parser tests for default and invalid concurrency-limit configuration
- engine tests using `PalMock` that simulate overlapping task durations and verify scheduler decisions
- engine tests for dependency unlocking, failure propagation, and skipped-task marking under concurrency
- tests for `.nao/runs/.../nao-events.jsonl` and `nao-summary.json` when tasks overlap
- CLI or runner tests for live display rendering with multiple simultaneously running tasks
- `./scripts/check-code.sh`

If exact wall-clock overlap is difficult to assert deterministically in tests, the tests should instead assert scheduler-observed event order and final task state.

# What assumptions should remain explicit?

This plan assumes:

- task dependencies remain the only scheduling constraint in the first version
- the initial concurrent version does not need process cancellation
- thread-per-running-task is acceptable for the first scheduler implementation
- run artifact ordering may reflect observed completion order instead of topological order once tasks overlap
- preserving existing sequential behavior for `max-parallel-tasks=1` is a hard compatibility requirement

# What risks or open questions matter most?

The main risks are:

- introducing race conditions in run-state bookkeeping or artifact generation
- making failure handling ambiguous when several tasks finish close together
- letting live display updates flicker or become inconsistent when several tasks change state at once
- over-coupling scheduler logic to CLI display concerns

The main open questions are:

- whether the concurrency limit should live only in recipe config or also gain a CLI override later
- whether already running tasks should eventually be cancellable after the first failure
- whether run observers need richer payloads than simple lifecycle callbacks once several tasks are running simultaneously
