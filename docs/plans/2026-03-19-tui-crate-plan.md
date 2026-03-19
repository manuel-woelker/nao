# What problem does this plan solve?

`nao` currently exposes task execution through the CLI and persists run artifacts under `.nao/runs`, but it does not provide an interactive interface for browsing the task graph, starting runs, or inspecting live and historical logs from one place.

This plan describes how to add a new `crates/tui` crate built with `ratatui` that can:

- launch tasks interactively
- show live progress for an ongoing run
- browse historical runs from `.nao/runs`
- inspect per-task logs, event streams, and final summaries without leaving the TUI

# What is the current status?

The repository currently has:

- a CLI runner in `crates/cli` that can list tasks and execute planned runs
- an engine in `crates/engine` that can plan runs, execute them, emit observer callbacks, and persist `.nao/runs` artifacts
- a run artifact format documented in `docs/NAO-SPEC.md`, including `nao-plan.json`, `nao-events.jsonl`, per-task log files, and `nao-summary.json`
- live terminal display modes that are optimized for short-lived CLI feedback rather than full-screen interaction

The main missing pieces are:

- a dedicated TUI crate in the workspace
- a stable application state model for switching between launch, live run, and run history views
- a way to follow an active run while also reusing persisted artifacts for completed runs
- keyboard navigation and layout rules for moving between run lists, task lists, summaries, and logs

# What implementation approach should be used?

The implementation should add a separate `crates/tui` crate that owns presentation, input handling, and TUI-specific view state, while reusing `nao-engine` and the run artifact format as the source of truth for execution and run inspection.

The first version should prioritize:

- a clear, fast keyboard-driven workflow
- reuse of existing engine execution and `.nao/runs` artifacts
- one coherent UI model for both live and historical runs
- correctness and inspectability over visual polish or feature breadth

The first version does not need mouse support, remote run management, multiple simultaneous active runs in one session, or inline log filtering beyond basic scrolling.

# Why should the TUI live in its own crate?

The TUI has a different responsibility than the existing CLI runner. It needs:

- frame-based rendering with `ratatui`
- terminal event handling
- focus management and keyboard routing
- persistent screen state across multiple panels

Those concerns should not be mixed into `crates/cli`, which is currently optimized for command execution and simple terminal output. A separate `crates/tui` crate keeps the architectural boundary clear:

- `crates/engine` owns planning and execution
- `crates/cli` owns command-line invocation and concise terminal summaries
- `crates/tui` owns full-screen interaction and browsing

# How should the TUI integrate with existing crates?

The `crates/tui` crate should depend on:

- `nao-engine` for planning and launching runs
- `nao-pal` for terminal and filesystem interaction where needed
- `nao-base` for shared data types such as `SharedString` and file paths
- `nao-recipe` for task metadata used in launch screens

The TUI should avoid duplicating business logic that already exists in the engine. In particular:

- run planning should continue to come from `RunEngine`
- live task state should come from engine observer callbacks or an engine-provided event bridge
- completed run browsing should read `.nao/runs` artifacts rather than maintaining a separate run database

If the current observer interface is too narrow for the TUI, the first implementation should extend the engine-side observer contract in a reusable way instead of introducing TUI-specific engine forks.

# What high-level screen model should the first version use?

The first version should use a small set of top-level screens with explicit focus:

1. `TaskLauncherScreen`: browse available tasks, inspect basic task metadata, and start a run.
2. `RunDetailScreen`: inspect one run, whether active or completed.
3. `RunHistoryScreen`: browse previous runs discovered under `.nao/runs`.
4. `HelpOverlay`: show keybindings relevant to the current screen.

This should behave as one application with shared navigation state rather than as several separate mini-apps.
The simplest mental model is:

- launch from the task browser
- automatically transition into the active run detail view
- keep that same run detail layout for completed runs opened from history

# How should application state be modeled?

The TUI should keep an explicit application state tree so rendering and key handling stay predictable.

The first version should likely model:

- current screen
- focused pane within the current screen
- selected task in the launcher
- discovered run list and current selected run
- active run session state when a run is in progress
- parsed detail state for the currently opened run, including summary, events, and available task log files
- scroll offsets for each pane that needs independent scrolling
- transient status messages such as load errors or run-start failures

The active run state and historical run state should converge into the same `RunDetailState` shape wherever possible so the UI does not need two unrelated detail renderers.

# How should runs be launched from the TUI?

The first version should support launching one or more selected goal tasks from the task launcher screen.

The launch flow should be:

1. Open the task launcher screen.
2. Move through the task list.
3. Toggle one or more goal tasks for execution.
4. Start the run with a dedicated keybinding.
5. Transition immediately into the run detail screen for the newly created run.

During execution, the TUI should:

- show live task status counts
- update task rows as observer events arrive
- make newly written logs viewable without leaving the run detail screen
- preserve the final run state after completion so the user can continue browsing it as a historical run

If a second launch while a run is active is out of scope for the first version, the UI should make that explicit and disable the launch action until the active run completes or the user exits the TUI.

# How should live and historical runs share one detail view?

The run detail view should be artifact-centric. Whether the run is still active or already finished, the user should be looking at the same conceptual objects:

- run summary
- task list with current or final status
- event stream
- selected task log

For an active run, the TUI should prefer incremental reads from the run directory so the same browsing path works for both active and completed runs.
That implies the engine work should make task logs and the run event stream available while execution is still in progress.

For the first live implementation, the TUI should combine:

- tailing task log files and `nao-events.jsonl` as they grow, and
- lightweight observer-fed state updates only where the engine has not yet flushed an artifact update

For a historical run, the same panels should be hydrated entirely from files already present in `.nao/runs/<run-id>`.

This keeps the navigation model stable and reduces the amount of TUI-specific state translation.

# What UI layout should the first version use?

The first version should use a two-column run detail layout with a persistent header and footer.

A recommended desktop-oriented sketch is:

```text
+----------------------------------------------------------------------------------+
| nao TUI | recipe: nao.kdl | screen: Run Detail | run: 2026-03-19T12-00-00Z-test |
+----------------------------------------------------------------------------------+
| Summary: running 2 | completed 5 | failed 0 | skipped 0 | duration 00:01.42     |
+-------------------------------------------+--------------------------------------+
| Tasks                                     | Task Output                          |
|-------------------------------------------|--------------------------------------|
| > build            running      00:18     | build.log                            |
|   lint             completed    00:03     | [12:00:01] stdout: compiling         |
|   test             pending                | [12:00:02] stdout: linking           |
|   package          pending                | [12:00:05] stderr: warning: ...      |
|                                           |                                      |
|                                           |                                      |
+-------------------------------------------+--------------------------------------+
| Events                                    | Run Summary / Task Metadata          |
|-------------------------------------------|--------------------------------------|
| 12:00:00 run_started                      | requested tasks: test                |
| 12:00:00 task_started build               | run result: running                  |
| 12:00:03 task_finished lint completed     | selected task: build                 |
|                                           | exit code: -                         |
+----------------------------------------------------------------------------------+
| Tab move pane | j/k move | Enter follow task | g/G top/bottom | r runs | ? help |
+----------------------------------------------------------------------------------+
```

For narrower terminals, the TUI should collapse into a tabbed single-column detail layout:

- `Tasks`
- `Log`
- `Events`
- `Summary`

The first version should support both layouts from the same state model, with layout selection driven by terminal width.

# How should the task launcher screen be laid out?

The launcher should optimize for fast task selection before execution starts.

A recommended sketch is:

```text
+----------------------------------------------------------------------------------+
| nao TUI | screen: Launch Tasks                                                   |
+-------------------------------------------+--------------------------------------+
| Available Tasks                           | Task Details                         |
|-------------------------------------------|--------------------------------------|
| [x] test                                  | name: test                           |
| [ ] build                                 | deps: build                          |
| [ ] lint                                  | description: Run unit and integration|
| [ ] package                               | execution: shell command             |
|                                           |                                      |
+-------------------------------------------+--------------------------------------+
| Selected goals: test                                                             |
+----------------------------------------------------------------------------------+
| Space toggle | Enter start run | r history | / filter later | ? help            |
+----------------------------------------------------------------------------------+
```

Filtering can remain out of scope for the first version if needed, but the layout should reserve room for it so the screen can grow without a full redesign.

# How should the run history screen be laid out?

The history screen should list discovered runs in reverse chronological order and allow quick reopening of prior results.

A recommended sketch is:

```text
+----------------------------------------------------------------------------------+
| nao TUI | screen: Run History                                                    |
+-------------------------------------------+--------------------------------------+
| Runs                                      | Selected Run                         |
|-------------------------------------------|--------------------------------------|
| > 2026-03-19T12-00-00Z-test   running     | goals: test                          |
|   2026-03-19T11-42-10Z-lint    completed  | result: completed                    |
|   2026-03-19T11-10-02Z-build   failed     | tasks: 4                             |
|                                           | duration: 00:31                      |
|                                           | failure: build exit code 101         |
+-------------------------------------------+--------------------------------------+
| Enter open run | l launcher | R refresh history | ? help                         |
+----------------------------------------------------------------------------------+
```

The first version should read summary status from `nao-summary.json` when present and fall back to `nao-events.jsonl` or directory naming heuristics for in-progress runs.

# What navigation concept should the first version use?

The navigation model should be pane-focused and Vim-like by default, while still allowing a few familiar alternatives such as arrows and `Tab`.

Recommended global navigation:

- `q`: close help overlay or exit the application from a top-level screen
- `?`: open help
- `1`: go to task launcher
- `2`: go to active or selected run detail
- `3`: go to run history
- `Tab` and `Shift-Tab`: cycle pane focus on the current screen

Recommended task launcher bindings:

- `j` and `k` or arrow keys: move the task selection
- `Space`: toggle the selected task as a goal
- `Enter`: start a run from the current selection

Recommended run history bindings:

- `j` and `k` or arrow keys: move the selected run
- `Enter`: open the selected run in run detail
- `R`: rescan `.nao/runs`

Recommended run detail bindings:

- `j` and `k`: move selection in the focused list
- `Enter`: in the task pane, follow the selected task and open its log
- `h` and `l`: move between left and right panes when useful
- `PageUp`, `PageDown`, `g`, and `G`: scroll long logs or event streams
- `e`: focus events
- `t`: focus tasks
- `o`: focus task output
- `s`: focus summary metadata
- `r`: jump to run history
- `L`: toggle auto-follow for the selected task log during active runs

The keybindings should be rendered in a footer and in the help overlay so discoverability does not depend on external documentation.

# How should active-run updates reach the TUI?

The TUI should receive active-run state through a non-blocking bridge so the render loop stays responsive.

The first version should likely:

- run the TUI event loop on the main thread
- execute the run on a worker thread
- tail task log files and `nao-events.jsonl` incrementally as they are appended
- periodically poll terminal input and refresh the active run view from growing artifacts

If the engine still needs observer callbacks for low-latency state changes during the transition, those callbacks should remain a small bridge to the same run-detail state instead of becoming the primary live-log transport.

The TUI should cache the last file offset per tailed file to avoid rereading entire logs or event streams every frame.

# How should run artifacts be parsed for browsing?

The TUI should add focused artifact-reading code for:

- discovering run directories under `.nao/runs`
- reading `nao-summary.json` for final status and task metadata
- reading `nao-events.jsonl` incrementally for event lists and active-run updates
- listing and reading per-task log files on demand

The first version should be tolerant of partially written artifacts so a run can be opened while it is still in progress.
That means parsing should gracefully handle:

- missing `nao-summary.json`
- an events file that is still growing
- log files that exist but have not yet received content

This artifact-first approach should be treated as the preferred live-update model, not just as a fallback for completed runs.

# What implementation order is recommended?

The recommended order is:

1. Add `crates/tui` to the workspace and wire in `ratatui` plus the terminal backend dependencies.
2. Define core TUI application state types, screen enums, focus handling, and key dispatch.
3. Implement task discovery and the launcher screen using existing planning/listing APIs.
4. Add a run-start flow that launches execution on a worker thread and transitions to run detail.
5. Update run artifact writing so task logs and `nao-events.jsonl` are written incrementally during execution.
6. Unify engine execution around one parallel scheduler where `max_parallel_tasks=1` is the sequential case.
7. Introduce a reusable run detail state model that works for both active and completed runs.
8. Implement artifact discovery and the run history screen for `.nao/runs`.
9. Implement the run detail layout with task list, task log, event stream, and summary panels.
10. Add active-run updates by tailing growing artifacts.
11. Add terminal resize handling and narrow-screen layout fallback.
12. Document the TUI workflow and keybindings.

# What concrete work items should be tracked?

- [ ] Add `crates/tui` and register it in the workspace.
- [ ] Add `ratatui` and the required terminal backend dependencies.
- [ ] Define TUI application state types for screens, focus, selections, and scroll positions.
- [ ] Add a `TaskLauncherScreen` that lists tasks and supports selecting one or more goal tasks.
- [ ] Add a run-start action that uses `RunEngine` to plan and execute a run from the TUI.
- [ ] Change artifact writing so task log files and `nao-events.jsonl` are appended during execution instead of only at completion.
- [ ] Unify engine execution around one parallel scheduler where `max_parallel_tasks=1` is treated as the sequential case.
- [ ] Align single-task worker-limit behavior with the parallel scheduler rather than preserving the current separate sequential path.
- [ ] Add run directory discovery under `.nao/runs`.
- [ ] Add parsing for `nao-summary.json`, `nao-events.jsonl`, and task log files.
- [ ] Add a `RunHistoryScreen` for browsing current and historical runs.
- [ ] Add a `RunDetailScreen` with summary, tasks, events, and task output panes.
- [ ] Add log auto-follow support for active runs.
- [ ] Add keyboard navigation, focus routing, and a help overlay.
- [ ] Add responsive layout behavior for narrow terminals.
- [ ] Add tests for artifact discovery and tolerant parsing of in-progress runs.
- [ ] Add tests for key routing and screen-state transitions.
- [ ] Add tests for active-run observer event handling.
- [ ] Update user-facing documentation for launching and browsing runs in the TUI.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated tests for artifact discovery from `.nao/runs`
- tests for parsing completed and partially written `nao-events.jsonl`
- tests for parsing `nao-summary.json` and correlating task log files
- tests showing that task log files and `nao-events.jsonl` are updated while a run is still executing
- tests for keybinding dispatch and pane-focus transitions
- tests for screen transitions between launcher, live run detail, and history
- tests using `PalMock` for deterministic concurrent scheduling and live artifact growth
- a manual terminal smoke test covering:
  - launching a run from the task screen
  - following live task updates by tailing artifacts
  - opening task logs
  - reopening a completed run from history
- `./scripts/check-code.sh`

If full-frame snapshot testing is introduced, it should focus on stable structural rendering rather than fragile timing-dependent content.

# What assumptions should remain explicit?

This plan assumes:

- the TUI can rely on `.nao/runs` as the canonical source for completed run browsing
- the first version only needs to manage one active run per TUI session
- the engine can be adjusted to flush task logs and events incrementally during execution
- `ratatui` is the preferred rendering library for the first implementation
- mouse support, search, and advanced filtering are out of scope for the first version

# What risks or open questions matter most?

The main risks are:

- making the first version too ambitious and delaying a usable baseline
- creating duplicate run-state logic in the TUI instead of reusing engine or artifact models
- handling partially written artifact files incorrectly during active runs
- changing `max_parallel_tasks=1` semantics in ways that expose new ordering or failure behavior differences
- letting keybindings or pane focus become inconsistent across screens
- building a layout that works on a wide terminal but degrades poorly on narrower screens

The main open questions are:

- whether historical runs should support text search in logs in the first version or later
- whether the TUI should live behind a new `nao tui` CLI subcommand or a separate executable
- whether the first launcher screen should support multi-select goals immediately or start with one selected goal
