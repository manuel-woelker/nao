# What problem does this plan solve?

The TUI refresh loop currently does more work than needed while a run is open.
Every refresh tick advances the spinner, polls for run completion, and then rereads run detail plus task logs from disk whenever the open run matches the active run.

That keeps the implementation simple, but it scales badly as run artifacts grow.
It also makes the refresh behavior harder to reason about because "refresh" currently mixes unrelated concerns:

- event-loop heartbeat
- active-run state transitions
- run-detail reload decisions
- task-log reload decisions

# What is the current status?

The current refresh loop in [`crates/tui/src/app.rs`](/data/projects/nao/crates/tui/src/app.rs) behaves like this:

- `run()` polls for terminal input every 150ms
- `refresh()` increments the spinner on every tick
- `refresh_active_run()` checks whether the background run worker has finished
- `should_refresh_open_run_detail()` returns true whenever the open run is also the active run
- `refresh_open_run_detail()` always reloads run detail, failed-task output, and selected task logs

This means the active run path rereads the same files repeatedly even when nothing user-visible changed between ticks.

# What implementation approach should be used?

The first pass should improve refresh behavior without redesigning the TUI architecture.
This should stay polling-based for now and avoid introducing file watching, async streams, or a general cache layer.

The implementation should separate three decisions that are currently coupled:

1. Should the app redraw because time passed?
2. Should run metadata be reloaded because the underlying run state changed?
3. Should task logs be reloaded because the selected log source changed or new bytes are likely available?

The cleanest way to do that is to make refresh produce explicit reload intents instead of calling broad reload helpers unconditionally.

# Why should this stay incremental?

The current behavior is inefficient, but it is not conceptually broken.
Trying to solve this with a large reactive redesign would create a lot of churn in a TUI file that is already too large.

The first improvement should remove obvious waste while preserving the current mental model:

- the TUI still ticks on a fixed cadence
- active runs still update live
- completed runs stay stable
- log-follow behavior stays intact

# How should refresh responsibilities be split?

`refresh()` should become a small coordinator that asks narrower questions and performs narrower updates.

Recommended split:

- keep spinner advancement independent from artifact reloads
- let active-run polling return a small summary of what changed
- reload run detail only when that summary indicates new persisted state is likely available
- reload selected task logs only when the open run, selected task, or known log progress changed

One reasonable shape is to introduce a small internal enum or struct such as `RefreshOutcome` or `RefreshChanges` that captures:

- whether the active run finished
- whether the open run likely changed on disk
- whether selected log content should be reloaded

# How should run-detail reloads be reduced?

Run detail should not be reloaded on every tick just because a run is active.
Instead, the app should reload detail only when one of these is true:

- the user opened a different run
- a new run was started and its preview directory became the open run
- the background worker reported completion or failure
- the active run is open and artifact progress likely advanced since the last observed state

To support the last case without reparsing everything blindly, the app should track a small amount of observed refresh state for the open run.
That state can be simple and local, for example:

- last loaded task count
- last loaded event count
- last known selected task log line count
- last refresh tick that observed active output progress

The goal is not perfect minimal I/O.
The goal is to stop rereading unchanged artifacts when no new data is likely present.

# How should task-log reloads be reduced?

Task logs are the hottest repeated read in the current code path.
They should be reloaded independently from run detail when possible.

The first pass should reload selected task logs only when:

- the selected run changes
- the selected task changes
- the failed task shown in the launcher changes
- an active run likely appended output for the relevant task
- the user has auto-follow enabled and the viewed run is still active

When the selected task and run are unchanged, the app should avoid reloading logs just because `refresh_open_run_detail()` happened.

If the current artifact-store helpers only support full reloads, the first pass may still use them, but the call sites should become conditional.
If that still leaves too much waste, a follow-up can add an incremental log-loading API.

# What small state should the app track?

The app likely needs one focused refresh-tracking struct rather than more ad hoc booleans.
For example, a struct owned by `App` could track:

- which run directory is currently open
- which task log is currently loaded
- whether the open run is active
- the last observed detail fingerprint for the open run
- the last observed fingerprint for the selected task log

The "fingerprint" can stay lightweight.
It does not need hashing.
Counts or persisted summary metadata are probably enough for the first pass.

# How should verification be designed?

Verification should stay behavior-based instead of time-based.
The important thing is to prove that unnecessary rereads stop happening while live updates still work.

Tests should focus on observable outcomes such as:

- completed runs do not trigger repeated artifact rereads during idle refreshes
- active runs reload only when new persisted state appears
- changing the selected task still reloads the correct log
- launcher failed-output view still follows the failed task log when a run is active
- run completion still reloads history and opens the completed run detail

If needed, `PalMock`-backed tests can count file reads or assert on which artifact files were requested.

# What implementation order is recommended?

1. Introduce explicit refresh-state tracking in `App`.
2. Refactor `refresh()` so active-run polling returns structured change information.
3. Make run-detail reloads conditional on explicit change signals instead of `should_refresh_open_run_detail()`.
4. Decouple selected task log reloads from full run-detail reloads.
5. Add or adapt artifact-store or TUI helpers needed to support lightweight change detection.
6. Add focused tests for idle refreshes, active-run updates, task switching, and run completion.
7. Run `./scripts/check-code.sh`.

# What concrete work items should be tracked?

- [x] Add a small refresh-tracking structure to `App` so reload decisions are based on explicit observed state.
- [ ] Refactor `refresh_active_run()` to report meaningful state changes instead of only mutating fields.
- [x] Remove `should_refresh_open_run_detail()` or reduce it to a narrow predicate that does not imply unconditional rereads.
- [x] Make open-run detail reloads conditional on real change signals.
- [x] Make selected task log reloads conditional on selection changes or likely log growth.
- [x] Preserve launcher failed-task output updates for active failed runs.
- [x] Add or update TUI tests that prove completed runs stay idle during repeated refresh ticks.
- [x] Add or update TUI tests that prove active runs still refresh when artifacts advance.
- [ ] Add or update TUI tests that prove task selection still reloads the correct log.
- [x] Run `./scripts/check-code.sh`.

# What assumptions should remain explicit?

This plan assumes:

- polling every 150ms remains acceptable for now
- the first pass should not introduce filesystem watching
- lightweight change detection is good enough even if it occasionally performs a conservative extra reload
- keeping the current screen model is preferable to a larger TUI state refactor in the same change

# What risks or follow-up questions matter most?

The main risks are:

- making reload conditions too strict and accidentally freezing live output
- spreading refresh state across too many fields and making `App` harder to maintain
- coupling the TUI too tightly to artifact-file implementation details

The main follow-up questions are:

- whether the artifact store should expose explicit "fingerprint" metadata for detail and log files
- whether the TUI should eventually switch from polling to file watching for active runs
- whether the larger `app.rs` file should be split before more refresh behavior is added
