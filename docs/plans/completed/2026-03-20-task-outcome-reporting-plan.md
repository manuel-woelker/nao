# What problem does this plan solve?

`nao` currently records whether a task succeeded or failed, but it does not have a first-class way for a task to publish a short human-readable outcome summary such as `100 files formatted` or `30 tests succeeded`.

That leaves the CLI and TUI with only coarse task status plus raw logs.
Users must inspect full task output to find the useful one-line result.

# What is the current status?

The repository now has:

- strict Bash execution for Unix `run shell` tasks in [crates/engine/src/run_engine.rs](/data/projects/nao/crates/engine/src/run_engine.rs)
- failure reporting via an injected `ERR` trap for shell tasks
- persisted task lifecycle data through `TaskEventRecord` in [crates/engine/src/task_event_record.rs](/data/projects/nao/crates/engine/src/task_event_record.rs)
- persisted task summary data through `TaskArtifactRecord` and `nao-summary.json` in [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
- CLI and TUI surfaces that already display task result and failure information

The completed work added:

- direct `Task outcome: ...` capture from framed task output
- persisted `outcome_message` fields in run events and summaries
- CLI success-summary rendering for single-goal outcomes
- TUI task and detail rendering for persisted outcomes

# What implementation approach should be used?

The implementation should treat any task output line that starts with `Task outcome: ` as an explicit task outcome summary.

This design supports one clear producer path:

- tasks print `Task outcome: ...` themselves

The engine should not need any special shell-only helper behavior.

# Why use a human-readable log-line convention?

Using a readable line prefix is simpler than adding a hidden protocol that must later be stripped from logs.

This approach is attractive because:

- the task decides what the outcome means
- the engine stays generic across formatters, test runners, and custom scripts
- existing tools and scripts can opt in just by printing one readable line
- the raw logs remain useful even outside `nao`'s structured views

# How should the shell wrapper behave?

The Unix shell wrapper should keep the existing strict Bash and failure-reporting behavior.

The wrapper should:

- continue using `bash -o errexit -o nounset -o errtrace -o pipefail -c ...`
- preserve the existing `ERR` trap that prints the failing line and command
- avoid injecting wrapper-specific outcome plumbing

# How should marker formatting work?

The first implementation should use a single-line human-digestible marker:

```text
Task outcome: <message>
```

The engine should treat this prefix as the task outcome convention.
The implementation should treat the final framed line payload as the marker message.

The initial implementation should:

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
- if no matching line is printed, no outcome is recorded

This keeps the task author contract explicit and portable.

# What implementation order is recommended?

The recommended order is:

1. Extend output framing or post-processing in the engine to detect `Task outcome: ` lines and capture the last matching message.
2. Store the extracted outcome on task-finished events and persisted task summary records.
3. Keep the Unix shell wrapper focused on strict execution and failure reporting.
4. Update the CLI to display the task outcome where it improves readability.
5. Update the TUI artifact loading and task detail rendering to surface the outcome.
6. Document the direct-output contract with explicit examples.

# What concrete work items were completed?

- [x] Detect `Task outcome: ` lines in engine task output processing.
- [x] Decide whether failed tasks should ignore previously emitted matching log lines or preserve them as debug metadata.
- [x] Persist `outcome_message` on finished task events and task artifact records.
- [x] Write `outcome_message` into `nao-events.jsonl` and `nao-summary.json`.
- [x] Update TUI summary/event loading to deserialize and expose the new field.
- [x] Update CLI rendering to display the outcome on successful task completion when available.
- [x] Update TUI rendering to display the outcome in task detail views.
- [x] Keep the Unix shell wrapper focused on strict Bash execution and failure reporting.
- [x] Add engine tests for shell wrapper generation with the failure-reporting trap.
- [x] Add engine tests for extracting the last emitted outcome line and ignoring earlier values.
- [x] Add engine tests proving directly printed outcome lines are captured without shell-wrapper help.
- [x] Add engine tests proving human-readable outcome lines remain in persisted task logs.
- [x] Add artifact tests for `nao-events.jsonl` and `nao-summary.json` including `outcome_message`.
- [x] Add CLI or runner tests for successful task rendering with an outcome message.
- [x] Add TUI tests for loading and rendering persisted task outcomes.
- [x] Update user-facing docs and examples.
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

# What assumptions remained explicit?

This plan assumes:

- direct `Task outcome: ...` log lines are the primary outcome-reporting contract
- a one-line textual message is sufficient for the first slice
- leaving human-readable outcome lines in user-visible logs is preferable to hiding them
- later support for structured outcome payloads may be desirable but is not required now

# What decisions and follow-up notes matter most?

The implemented slice made these decisions:

- failed tasks preserve the last emitted outcome as debug metadata, but the CLI only promotes outcomes in successful single-goal summaries
- Unix `run shell` tasks use the same explicit output contract as every other task type
- the outcome marker remains plain text for now
- extraction happens in engine post-processing over framed task lines
- the CLI shows the outcome in the final success summary for a single requested goal task rather than trying to aggregate multiple outcomes

The remaining follow-up areas are:

- reconsidering whether richer task-list rendering should surface outcomes more prominently in the CLI or TUI
