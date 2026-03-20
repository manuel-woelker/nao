# What problem does this plan solve?

`nao` currently records whether a task succeeded or failed, but it does not have a first-class way for a task to publish a short human-readable outcome summary such as `100 files formatted` or `30 tests succeeded`.

That leaves the CLI and TUI with only coarse task status plus raw logs.
Users must inspect full task output to find the useful one-line result.

# What is the current status?

The repository currently has:

- strict Bash execution for Unix `run shell` tasks in [crates/engine/src/run_engine.rs](/data/projects/nao/crates/engine/src/run_engine.rs)
- failure reporting via an injected `ERR` trap for shell tasks
- persisted task lifecycle data through `TaskEventRecord` in [crates/engine/src/task_event_record.rs](/data/projects/nao/crates/engine/src/task_event_record.rs)
- persisted task summary data through `TaskArtifactRecord` and `nao-summary.json` in [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
- CLI and TUI surfaces that already display task result and failure information

The main missing pieces are:

- a task-facing API for setting an outcome message
- engine logic that extracts and persists the final outcome
- CLI and TUI rendering that shows the outcome as part of task results

# What implementation approach should be used?

The implementation should treat any task output line that starts with `Task outcome: ` as an explicit task outcome summary.

For shell tasks on Unix, `nao` should also provide a convenience path through an environment variable plus injected wrapper code:

1. `nao` injects a reserved environment variable such as `NAO_TASK_OUTCOME`.
2. Task code may assign or update `NAO_TASK_OUTCOME` anywhere during execution.
3. `nao` prepends shell wrapper code that installs an `EXIT` trap.
4. The `EXIT` trap checks the final exit status and, on success only, prints a line such as `Task outcome: 100 files formatted`.
5. The engine extracts the last matching `Task outcome: ` line from framed task output and stores it as structured task outcome metadata.

This design supports two valid producer paths:

- tasks may simply print `Task outcome: ...` themselves
- shell tasks may rely on the injected `NAO_TASK_OUTCOME` convenience helper

The engine should not distinguish between those two sources.

# Why use a human-readable log-line convention?

Using a readable line prefix is simpler than adding a hidden protocol that must later be stripped from logs.

This approach is attractive because:

- the task decides what the outcome means
- the engine stays generic across formatters, test runners, and custom scripts
- existing tools and scripts can opt in just by printing one readable line
- shell tasks still get a convenience helper through `NAO_TASK_OUTCOME`
- the raw logs remain useful even outside `nao`'s structured views

# How should the shell wrapper behave?

The Unix shell wrapper should keep the existing strict Bash and failure-reporting behavior, and add a success-path `EXIT` trap for outcome emission.

The wrapper should:

- continue using `bash -o errexit -o nounset -o errtrace -o pipefail -c ...`
- preserve the existing `ERR` trap that prints the failing line and command
- add an `EXIT` trap that:
  - captures the final exit code
  - emits an outcome marker only when the exit code is `0`
  - emits an outcome marker only when `NAO_TASK_OUTCOME` is non-empty
  - preserves the original exit code

The task-facing shell usage should look like:

```sh
NAO_TASK_OUTCOME="discovering files"
count=$(find . -name '*.rs' | wc -l)
NAO_TASK_OUTCOME="$count files formatted"
cargo fmt
```

The wrapper should read the environment variable at exit time so the last value wins.

# How should marker formatting work?

The first implementation should use a single-line human-digestible marker:

```text
Task outcome: <message>
```

The engine should treat this prefix as the task outcome convention.
The message should be normalized to a single line before emission.

The initial implementation should likely:

- strip trailing newlines from the environment variable value before printing
- reject or normalize embedded newlines so the protocol stays one marker per line
- treat the last valid marker line in task output as the final outcome

Because the marker is human-readable, matching lines should remain in task logs as ordinary output.
If richer metadata becomes useful later, the marker format can evolve to structured JSON, but the first slice should keep the convention simple.

# How should outcome extraction work in the engine?

Outcome extraction should happen in the engine where task output is already framed into lines.
The engine should scan the framed task output lines for lines that begin with the `Task outcome: ` prefix, regardless of whether the line came from wrapper-injected shell code or ordinary task output.

The recommended behavior is:

1. Frame stdout and stderr as today.
2. Detect lines that begin with `Task outcome: `.
3. Extract the message payload from the marker.
4. Keep only the last extracted outcome for the task.
5. Leave marker lines in rendered task output, failure tails, and persisted task log files because they are intended to be understandable to humans.

This keeps the transport simple and makes the raw logs readable even outside `nao`'s structured views.

# What data model changes are needed?

The implementation should add an optional outcome field, likely `outcome_message: Option<SharedString>`, to the persisted task result structures.

The likely change points are:

- `TaskEventRecord::Finished` in [crates/engine/src/task_event_record.rs](/data/projects/nao/crates/engine/src/task_event_record.rs)
- `TaskArtifactRecord` in [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
- `nao-events.jsonl` task-finished events in [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
- task entries in `nao-summary.json` in [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
- any TUI summary-loading types that deserialize task summary data in [crates/tui/src/artifact_store.rs](/data/projects/nao/crates/tui/src/artifact_store.rs)

The field should remain optional so existing tasks continue to work without declaring outcomes.

# How should the CLI and TUI display the outcome?

The first UI slice should keep the display small and additive.

For the CLI:

- include the outcome in task-completion lines when available
- consider including the outcome in the final success summary for single-goal runs where it reads naturally

For the TUI:

- add an `outcome:` line in task detail views when available
- optionally show the outcome as muted trailing text in task lists if the layout has room

Failure rendering should continue to prioritize the failure state.
If a task printed a matching outcome line before later failing, the stored outcome may still be useful for debugging, but the UI should not present it as a success summary.

# What should the task author contract be?

The documented contract should be:

- any task output line beginning with `Task outcome: ` is eligible to become the task outcome summary
- if multiple matching lines are printed, the last one wins
- shell tasks on Unix may alternatively set or update `NAO_TASK_OUTCOME`, which `nao` will emit as `Task outcome: ...` on successful exit
- if no matching line is printed and the helper variable is unset or empty, no outcome is recorded

This makes direct log output the primary mechanism and the environment-variable wrapper a convenience path for shell tasks.

# What implementation order is recommended?

The recommended order is:

1. Extend output framing or post-processing in the engine to detect `Task outcome: ` lines and capture the last matching message.
2. Store the extracted outcome on task-finished events and persisted task summary records.
3. Add a focused helper that builds the Unix shell wrapper with both `ERR` and `EXIT` traps.
4. Inject `NAO_TASK_OUTCOME` into shell task environments if it is not already present.
5. Emit `Task outcome: ...` lines on successful shell task exit when the outcome variable is non-empty.
6. Update the CLI to display the task outcome where it improves readability.
7. Update the TUI artifact loading and task detail rendering to surface the outcome.
8. Document the direct-output contract and shell-task helper examples.

# What concrete work items should be tracked?

- [ ] Detect `Task outcome: ` lines in engine task output processing.
- [ ] Decide whether failed tasks should ignore previously emitted matching log lines or preserve them as debug metadata.
- [ ] Persist `outcome_message` on finished task events and task artifact records.
- [ ] Write `outcome_message` into `nao-events.jsonl` and `nao-summary.json`.
- [ ] Update TUI summary/event loading to deserialize and expose the new field.
- [ ] Update CLI rendering to display the outcome on successful task completion when available.
- [ ] Update TUI rendering to display the outcome in task detail views.
- [ ] Add a Unix shell-wrapper helper that composes strict Bash, the existing `ERR` trap, and a success-path `EXIT` trap for outcome emission.
- [ ] Decide whether `NAO_TASK_OUTCOME` should be injected as empty by default or only consumed when explicitly set by the task.
- [ ] Normalize or reject multiline outcome values before wrapper-driven marker emission.
- [ ] Add engine tests for shell wrapper generation with both failure and success traps.
- [ ] Add engine tests for extracting the last emitted outcome line and ignoring earlier values.
- [ ] Add engine tests proving directly printed outcome lines are captured without shell-wrapper help.
- [ ] Add engine tests proving human-readable outcome lines remain in persisted task logs.
- [ ] Add artifact tests for `nao-events.jsonl` and `nao-summary.json` including `outcome_message`.
- [ ] Add CLI or runner tests for successful task rendering with an outcome message.
- [ ] Add TUI tests for loading and rendering persisted task outcomes.
- [ ] Update user-facing docs and examples.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated engine tests for Bash wrapper construction and outcome extraction behavior
- tests covering multiple matching lines where the last outcome wins
- tests ensuring tasks without an outcome continue to behave unchanged
- tests proving directly printed outcome lines are captured for tasks without env-var helper usage
- tests ensuring human-readable outcome lines remain in normal logs
- artifact tests for persisted `outcome_message` in run events and summaries
- CLI and TUI tests that prove the outcome is rendered when present
- `./scripts/check-code.sh`

The tests should prefer `PalMock` and existing artifact assertions so the behavior remains deterministic.

# What assumptions should remain explicit?

This plan assumes:

- direct `Task outcome: ...` log lines are the primary outcome-reporting contract
- the shell env-var helper may be scoped to Unix `run shell` tasks in the first implementation
- a one-line textual message is sufficient for the first slice
- leaving human-readable outcome lines in user-visible logs is preferable to hiding them
- later support for structured outcome payloads may be desirable but is not required now

# What risks or open questions matter most?

The main open questions are:

- whether script tasks and Windows shell tasks should also receive a helper mechanism in the first slice or be left with direct-output support only
- whether the outcome marker should remain plain text or move to JSON immediately
- whether extraction belongs in the output framer, the task executor, or artifact writing
- whether the CLI should show only per-task outcomes or also aggregate them into the final run summary

The main risks are:

- shell-trap composition accidentally changing the existing failure semantics
- shell-trap emission and engine extraction disagreeing about newline normalization
