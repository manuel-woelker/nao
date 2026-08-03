# What problem does this plan solve?

Some long-running tasks, such as development servers, are more useful when their process output is shown directly while the task is running.

`nao` currently frames task output for logs and summaries, while the interactive CLI focuses on task status. That is good for build and CI-style tasks, but it hides important live feedback for server-like tasks such as:

- local dev servers printing URLs
- file watchers printing rebuild errors
- integration services printing readiness messages

The goal is to let selected tasks stream their output directly to the terminal while preserving the existing task lifecycle display, run artifacts, status capture, outcome capture, and failure summaries.

# What is the current status?

The relevant pieces are already in place:

- recipe parsing and task data live in `crates/recipe`
- task output is framed by `TaskOutputFramer` in `crates/engine/src/task_output_framer.rs`
- live log artifacts and `Task status: ` capture are handled by `LiveTaskArtifactSink` in `crates/engine/src/live_task_artifact_sink.rs`
- task execution is scheduled in `crates/engine/src/run_engine/execution.rs`
- CLI progress rendering is implemented through `RunObserver` implementations in `crates/cli/src/runner/live_display`
- CI mode already writes task lifecycle updates directly through `CiDisplay` in `crates/cli/src/runner/ci_display.rs`

The gap is that `RunObserver` currently receives lifecycle and status callbacks, but it does not receive task output callbacks.
Because of that, the CLI cannot selectively stream task output as it arrives.

# What implementation approach should be used?

Add a task-level opt-in for direct output, then propagate live output lines through the existing observer path.

The first implementation should:

1. add a recipe-level task property such as `direct-output=#true`
2. store the setting on `nao_recipe::Task`
3. extend the engine observer contract with an output-line callback
4. emit output-line callbacks from `LiveTaskArtifactSink` only after lines have been framed
5. let CLI live displays render opted-in task output immediately
6. keep persisted task logs and final summaries unchanged

This keeps direct output as a presentation feature rather than a second process-execution path.

# Why should direct output be task-scoped?

Direct output is useful for some tasks and noisy for others.

A global setting would make normal build/test runs harder to scan, especially with parallel execution.
A task-level flag lets recipe authors opt in only where live logs are part of the product experience.

For example:

```kdl
task "dev-server" direct-output=#true {
  run shell="pnpm dev"
}
```

# How should the direct output be formatted?

Every direct output line should be prefixed with the task name.
Prefixes should be padded to the same width across the planned run so messages align:

```text
dev-server | Vite ready at http://localhost:5173
api        | listening on http://localhost:3000
```

The width should be derived from the longest task name in the planned run, not just the currently running direct-output tasks.
That avoids output shifting when another task starts later.

The prefix should be plain, stable text in non-interactive output.
Interactive terminals may style the task name, but styling must not affect the computed visible width.

# How should stdout and stderr be distinguished?

The direct output line should prioritize the task name prefix.

For the first slice, stdout and stderr can share the same task prefix because many server tools already include severity in their own output.
If distinguishing streams proves necessary, add a small stream marker after the task name without breaking alignment:

```text
api        stderr | address already in use
```

Do not mix timestamp-prefixed artifact formatting into live direct output.
Timestamps remain in persisted logs and structured artifacts where they are more useful.

# How should this interact with live task displays?

Direct output should coexist with the current live displays without corrupting terminal rendering.

Recommended behavior:

- in non-interactive mode, print the run header and then stream direct output lines normally
- in `line-per-task` interactive mode, temporarily move below the live display, print direct output, then redraw the display
- in `single-line` interactive mode, clear the spinner line, print direct output, then redraw the spinner
- in CI mode, keep existing CI lifecycle rendering unless a separate `--ci` direct-output policy is intentionally added

If terminal redraw handling becomes messy, prefer a conservative first slice where direct output is enabled only for non-interactive display and `line-per-task`, then document the limitation. The implementation should not make spinner output visually broken.

# How should concurrent task output be handled?

The engine already frames output into complete lines before appending task logs.
Direct output callbacks should use those framed lines rather than raw byte chunks.

This gives the CLI complete logical lines and avoids interleaving partial chunks from parallel tasks.
The CLI should still treat each callback as one atomic write so multiple tasks cannot interleave within a single rendered line.

# What data model changes are needed?

Likely recipe changes:

- add `direct_output: bool` to `nao_recipe::Task`
- parse an optional task property named `direct-output`
- reject non-boolean values with a clear recipe error
- default to `false`

Likely engine changes:

- extend `RunObserver` with `on_task_output_line(task_name, stream, line)`
- call the observer from scheduler-owned code when `task.direct_output` is enabled
- include `ProcessOutputStream` in the observer callback so future renderers can distinguish streams

Likely CLI changes:

- teach live display implementations how to render direct output lines
- compute one shared task-name prefix width from the planned run
- keep writes synchronized with existing live display updates

The persisted artifact format should not need to change unless implementation discovers that the task setting should be written into `nao-plan.json` for history inspection.

# What implementation order is recommended?

The recommended order is:

1. Extend `Task` with a `direct_output` boolean and update parser tests.
2. Document `direct-output` in `docs/RECIPES.md` and the CLI help text.
3. Extend `RunObserver` with an output-line callback that includes task name, stream, and line.
4. Emit direct-output callbacks from the execution path after framed log lines are available.
5. Update CLI live displays to render aligned task-prefixed output lines.
6. Add tests for parser behavior, observer behavior, and CLI rendering.
7. Run repository-wide verification.

# What concrete work items were completed?

- [x] Add `direct_output: bool` to `nao_recipe::Task` using the existing task model style.
- [x] Parse optional `direct-output=#true|#false` task properties.
- [x] Add recipe parser tests for default behavior, enabled behavior, and invalid values.
- [x] Update recipe documentation and CLI help text with the new task property.
- [x] Extend `RunObserver` with a direct output callback carrying task name, stream, and line payload.
- [x] Emit direct output callbacks only for tasks where `direct_output` is enabled.
- [x] Keep `Task status: ` and `Task outcome: ` capture working for direct-output tasks.
- [x] Render direct output lines in the CLI with aligned fixed-width task prefixes.
- [x] Ensure direct output writes do not corrupt interactive live display redraws.
- [x] Add engine tests proving direct-output callbacks are emitted only for opted-in tasks.
- [x] Add runner or live display tests for aligned prefixes with different task-name lengths.
- [x] Add regression tests for direct-output tasks producing complete, non-interleaved lines.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated parser tests in `crates/recipe/src/parse_recipe.rs`
- engine tests using `PalMock` process events in `crates/engine/src/run_engine/tests.rs`
- CLI runner or live display tests proving aligned prefix rendering
- at least one test where two direct-output task names have different lengths
- at least one test proving a normal task does not stream direct output
- `./scripts/check-code.sh`

The implementation was verified with focused recipe, engine, and CLI tests, then with `./scripts/check-code.sh`.
The full check passed formatting, build, clippy, and 173 tests.

# What was implemented?

The completed implementation adds task-scoped direct output through `direct-output=#true`.
Opted-in task output is framed into complete lines by the engine, streamed through `RunObserver`, and rendered by the CLI with fixed-width task-name prefixes computed from the planned run.

The implementation keeps persisted task logs, `Task status: ` capture, `Task outcome: ` capture, CI summaries, and final run summaries on the existing paths.

# What assumptions and risks matter?

This plan assumes:

- `direct-output=#true` is the right first recipe spelling
- direct output should be opt-in per task, not global
- line-level callbacks are sufficient for server-style output
- raw byte streaming is not needed for the first implementation
- persisted timestamped task logs remain the source of truth for historical output

The main risk is terminal redraw complexity when direct output is mixed with animated live displays.
If that becomes fragile, the implementation should reduce animation around direct output rather than introduce a separate execution path.

# What should be considered later?

Possible follow-up directions:

- a recipe-level default for direct output
- stream markers when stderr visibility matters
- direct output support in the TUI active-run view
- a `nao dev` style mode optimized for long-running tasks
- support for grouping or filtering direct output when many services run together
