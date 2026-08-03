# What problem does this plan solve?

Long-running development tasks, such as servers and file watchers, often need a manual restart after code, configuration, or dependency changes.

Today the TUI can start a selected task graph with `Enter`, but once a run is active it only reports that another run is already active.
That blocks a natural workflow:

1. start a dev-server task graph
2. edit code or configuration
3. press one shortcut to stop the running graph and start it again

The goal is to add a keyboard shortcut that restarts the active run's requested goal tasks without requiring the user to quit the TUI or manually reselect tasks.

# What is the current status?

The TUI already has:

- launcher keyboard routing in `crates/tui/src/app/input.rs`
- run start logic in `crates/tui/src/app/lifecycle.rs`
- an `ActiveRunHandle` that tracks the active run directory and completion receiver in `crates/tui/src/app/mod.rs`
- run detail refresh logic that follows active run artifacts
- help and footer text in `crates/tui/src/app/render.rs`

The engine and PAL currently do not expose cancellation:

- `Pal::run_process` blocks until a child process exits
- `PalReal::run_process_async` waits on the child and output streams, but has no cancellation token
- the engine scheduler joins worker threads after process execution finishes
- the TUI active run handle has a receiver, but no way to request stop or join the worker thread

Because of that, the restart feature needs a real stop/cancel path before it can safely relaunch tasks.
Starting a second run while the old process still owns ports would be a bad user experience.

# What behavior should users get?

Pressing `Ctrl+R` while a run is active should:

1. request cancellation of the current active run
2. wait for running task processes to exit or be killed
3. start a fresh run using the same requested goal tasks
4. open the new run detail view
5. show a status message such as `run restarted`

If no run is active, `Ctrl+R` should restart the most recent in-session run when that requested task list is known.
If no restart target is known, it should show a status message such as `no run is available to restart`.

# Why use `Ctrl+R`?

Plain `r` already navigates to run history from launcher and detail screens.
Uppercase `R` refreshes history.

`Ctrl+R` keeps restart distinct from navigation and refresh while remaining familiar for reload-style behavior.
The implementation should match on `KeyCode::Char('r')` with `KeyModifiers::CONTROL`.

# What should be restarted?

The first implementation should restart the active run's requested goal tasks, not every task in the recipe.

That means if the user starts `counter-direct`, restart runs `counter-direct` again.
If the user starts a multi-goal selection, restart runs that same multi-goal selection again.
Dependencies are replanned from the current recipe, so recipe edits made between starts can affect the new graph.

This is more useful than literally starting every task in `.nao/nao.kdl`, which would run unrelated test, failure, and demo tasks.

# How should cancellation work?

Add a small cancellation primitive that flows from the TUI down to process execution.

Likely shape:

- add a `CancellationToken` or `RunCancellation` type in a shared crate or engine module
- store one token on `ActiveRunHandle`
- pass the token into `RunEngine::execute_planned_run_with_observer_started_at` or a new cancellable sibling method
- pass the token into scheduler worker threads
- pass the token into `Pal::run_process` or add `Pal::run_process_cancellable`
- teach `PalReal` to kill the child process when cancellation is requested
- teach `PalMock` to observe cancellation deterministically in tests

The cancel path should produce a clear run result rather than look like an ordinary task failure when possible.
If a fully distinct `cancelled` status is too broad for the first slice, the plan should at least ensure the TUI can tell "restart requested" from an unexpected process failure.

# How should process termination behave?

The first implementation should be pragmatic:

- request cancellation
- terminate running child processes
- drain already-received output events where practical
- write final run artifacts that make the interrupted run inspectable
- start the replacement run only after the old worker reports completion

On Unix, `PalReal` should prefer terminating the child process and then waiting for it.
If graceful termination is not already available through the chosen process API, killing the child is acceptable for the first implementation because this is an explicit restart command.

Follow-up work can add a graceful shutdown timeout or signal selection if dev-server tasks need it.

# How should TUI state remember restart targets?

The TUI should store the last launched requested goal task names separately from selection state.

Suggested field:

```rust
last_run_goal_tasks: Vec<SharedString>
```

Update it when a run starts successfully.
Use it for restart when no run is currently active.
For an active run, the active handle may also store its requested goals so restart is not affected by launcher selection changes made while the run is executing.

# How should keyboard routing work?

Add global handling before screen-specific routing:

- `Ctrl+R`: restart the active or last in-session run

The shortcut should work from launcher, run detail, and history screens.
It should be ignored while help is visible, matching the current help-modal behavior where keys close help instead of performing actions.

# How should rendering communicate the shortcut?

Update help and footer text to expose the shortcut:

- launcher footer should mention `Ctrl+R restart` when a run target exists
- detail footer should mention `Ctrl+R restart`
- help should include `Ctrl+R restart active run`

Keep text short so existing footer lines do not become noisy.

# What implementation order is recommended?

The recommended order is:

1. Add a cancellation token type and tests for its basic state transitions.
2. Extend PAL process execution with cancellable process running.
3. Implement real child termination in `PalReal`.
4. Add deterministic cancellation behavior to `PalMock`.
5. Thread cancellation through engine task execution and scheduler worker threads.
6. Decide how cancelled/interrupted runs are represented in run artifacts.
7. Extend `ActiveRunHandle` to keep cancellation, requested goals, and worker completion state.
8. Add `restart_run` lifecycle logic that cancels the current run and starts the same requested goals after it completes.
9. Add global `Ctrl+R` keyboard routing.
10. Update TUI help/footer copy.
11. Add focused tests for active restart, no-target restart, and shortcut routing.
12. Run repository-wide verification.

# What concrete work items are planned?

- [ ] Add a cancellable process execution API at the PAL boundary.
- [ ] Implement child-process termination in `PalReal`.
- [ ] Add `PalMock` support for deterministic cancellation tests.
- [ ] Thread cancellation through `RunEngine` without breaking existing non-cancellable callers.
- [ ] Persist interrupted run state clearly enough that history and detail views can inspect the old run.
- [ ] Store active and last-launched requested goal tasks in TUI state.
- [ ] Add `restart_run` lifecycle behavior that stops the active run before relaunching.
- [ ] Add global `Ctrl+R` input handling.
- [ ] Update help and footer text for the restart shortcut.
- [ ] Add engine or PAL tests for cancellation of a running task.
- [ ] Add TUI tests for restart while a run is active.
- [ ] Add TUI tests for `Ctrl+R` with no restart target.
- [ ] Add TUI tests proving restart uses the previous goal tasks instead of the current launcher selection.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- PAL unit tests proving cancellation terminates or interrupts a running process
- engine tests proving cancellation stops scheduling new tasks and records the interrupted run coherently
- TUI tests for `Ctrl+R` routing from launcher, detail, and history screens
- TUI tests proving restart relaunches the same requested goals
- TUI tests proving no active or previous run produces a helpful status message
- manual testing with `.nao/nao.kdl` task `counter-direct`
- `./scripts/check-code.sh`

# What assumptions and risks matter?

This plan assumes:

- the shortcut should be `Ctrl+R`
- "all tasks" means all tasks in the active run graph, not every task in the recipe
- restart should stop the old run before starting the new one
- cancellation should be implemented at the process boundary rather than by detaching old worker threads
- a hard kill is acceptable for the first restart implementation

The main risk is cancellation semantics across platforms.
Unix process termination and Windows process termination may need separate implementation details.
The plan should keep the public behavior stable while allowing platform-specific PAL code underneath.

# What open questions should be settled during implementation?

- Should cancelled runs get a first-class `cancelled` status in `nao-summary.json`, or should they remain failed with an explicit cancellation message?
- Should restart preserve the same run detail focus and selected task when the new run opens?
- Should `Ctrl+R` work when a completed historical run is selected, restarting that historical run's requested goals?
- Should long-running tasks later get a graceful shutdown timeout before force-kill?
