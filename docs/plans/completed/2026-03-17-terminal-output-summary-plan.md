# What problem does this plan solve?

Successful `nao` runs currently print per-task execution output to the terminal even though the run artifacts already persist detailed logs, events, and summaries under `.nao/runs`.

That is too verbose for the success path. A successful run should instead print one summary line that tells the user:

- which goal task or tasks were requested
- how many total tasks were executed for the run
- how long the run took, rendered in a human-friendly format

This plan describes how to change terminal output for successful runs without weakening failure diagnostics or the persisted run artifacts.

# What is the current status?

This plan is complete.

The repository currently has:

- a CLI runner that renders a one-line success summary
- a `RunExecutionResult` type that carries structured summary fields as well as detailed output for later failure handling
- engine-side task framing that is still used for persisted task logs
- persisted run artifacts under `.nao/runs` that already contain the detailed execution record

# What implementation approach should be used?

The success-path terminal output should become a thin summary that is derived from run metadata rather than from task log content.

The intended split is:

1. run artifacts remain the source of detailed execution information
2. failure paths continue to surface actionable diagnostics in the terminal
3. successful runs render one concise summary line from structured run result data

This means the engine should return structured summary fields for the CLI to render instead of treating the full per-task terminal log as the primary output contract.

# What should a successful summary line contain?

The success summary line should include:

- the requested goal task or tasks
- the total number of tasks included in the run plan
- the total execution time in a pretty-printed form

A representative implemented shape is:

```text
Suceeded test in 1.2s (2 tasks)
```

If multiple goal tasks were requested, the summary should still stay on one line.
The implemented separator is `,`, and the goal tasks are rendered in bold.

# What should happen on failed runs?

Failed runs should not be reduced to the same one-line summary.
The failure path still needs enough terminal output to make the problem obvious without immediately requiring users to open `.nao/runs`.

The failure behavior should therefore keep:

- the existing top-level error reporting
- the task failure message and exit code
- a pointer to the run directory when that helps the user find logs quickly

The implementation should explicitly avoid optimizing the failure path into silence.

# Where should the pretty execution time be computed?

The pretty-printed execution time should be computed above the PAL and below the CLI formatting boundary.
The PAL should continue to provide timestamps and wall-clock time, but it should not own human-readable formatting policy.

The best home for this logic is likely:

- a small engine-side helper that converts nanoseconds into a compact display string
- or a CLI rendering helper if the project wants terminal phrasing to stay out of the engine

Whichever location is chosen, the underlying run result should carry the raw timing data rather than only the final formatted string.

# What changes should be made to the run result model?

`RunExecutionResult` should evolve from “rendered output plus run directory” toward structured summary data.

The result model should likely include:

- the requested goal tasks
- the total task count
- the raw run duration
- the run directory path
- whether the run completed successfully

If detailed terminal output is still needed on failure, that detail can either remain as an optional field or be reconstructed from the error path separately.

# How should the task output framer be used after this change?

The task output framer is still useful for persisted per-task log files.
It should no longer be treated as the default source of terminal output for successful runs.

That means the implementation should:

- continue to collect framed task log lines for `.nao/runs/.../*.log`
- stop concatenating those lines into the success-path CLI output
- decide explicitly whether failure paths should reuse any of that framed output

This keeps terminal success output and artifact generation as separate concerns.

# How should the work be ordered?

The recommended implementation order is:

1. Extend `RunExecutionResult` with structured summary fields needed for terminal rendering.
2. Add a small pretty-duration formatter for run timing.
3. Update `RunEngine` so successful runs return summary data rather than full task-by-task terminal output.
4. Keep run artifact generation unchanged so detailed logs are still persisted.
5. Update the CLI runner to render a single success summary line from the structured result.
6. Decide whether failed runs should mention the run directory explicitly and implement that consistently.
7. Update or add tests for success and failure terminal output behavior.

# What concrete work items should be tracked?

- [x] Replace or supplement the success-path `output` field in `RunExecutionResult` with structured summary fields.
- [x] Add a helper for pretty-printing run durations.
- [x] Ensure the engine still records accurate total task count from the planned run.
- [x] Ensure the engine still reports the requested goal task or tasks in the run result.
- [x] Stop using per-task framed output as the terminal success contract.
- [x] Keep per-task framed output for persisted task log files.
- [x] Update `crates/cli/src/runner.rs` to render a one-line success summary.
- [ ] Decide and document whether failed runs should print the run directory path.
- [x] Add or update colocated tests for successful terminal output.
- [ ] Add or update colocated tests for failed terminal output.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- engine tests for the structured run result fields
- CLI tests showing that successful runs print exactly one summary line
- CLI tests showing that failure output still includes the relevant error information
- tests that cover one requested task and multiple requested tasks
- tests that cover short and longer durations for pretty-print formatting
- running `./scripts/check-code.sh`

# What assumptions should remain explicit?

This plan assumes:

- successful terminal output should prioritize brevity over replaying detailed task logs
- detailed execution information remains available in `.nao/runs`
- failure output still needs to be more informative than success output
- the requested goal tasks should be visible in the success summary line

# What risks or open questions matter most?

The main risks are:

- over-coupling terminal phrasing to engine internals
- choosing a pretty-duration format that becomes noisy or inconsistent
- accidentally hiding useful failure information while simplifying the success path

The main open questions are:

- whether the run directory path should always be printed on failure or only in selected cases
