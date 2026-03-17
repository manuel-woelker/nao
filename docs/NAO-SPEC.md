# What is this document?

This document defines the intended run output format for `nao`.
It focuses on the files and directories that `nao` should produce for each run so runs are inspectable, reproducible, and easy to analyze after completion.

This document is a work in progress and describes the current intended direction of the format.

# Where should run output be written?

Each `nao` run should create a new directory under `.nao/runs` in the same directory that contains the `nao.kdl` file used for the run.

If `nao` is executed from a repository root that contains `nao.kdl`, the output root should therefore be:

```text
.nao/runs
```

If a different `nao.kdl` file is selected, the `.nao/runs` directory should be resolved relative to that file's parent directory rather than the current working directory.

# How should a run directory be named?

Each run directory name should include:

- the current date and time in ISO-8601 format
- the task name given on the command line

The timestamp should identify when the run started.
The task portion should make it obvious which top-level task the user requested.
Run directory names must be filesystem-safe and must not contain `:`.
The timestamp should therefore use a filesystem-safe ISO-8601 variant.

An example directory name is:

```text
2026-03-17T18-42-11Z-test
```

This preserves the meaning and sort order of the timestamp while avoiding characters that are problematic on some filesystems.

# Which files must a run directory contain?

Each run directory should contain these files:

- `nao-plan.json`
- `nao-events.jsonl`
- one `<sanitized-task-name>.log` file per task
- `nao-summary.json` once the run completes

Together, these files should make it possible to reconstruct what `nao` intended to run, what happened during the run, and what the final result was.

# What should `nao-plan.json` contain?

`nao-plan.json` should describe the run plan determined before task execution begins.

At a minimum, it should include:

- the requested top-level task or tasks
- the full set of tasks selected for the run
- the dependency relationships between those tasks
- the relevant execution details for each task, such as execution kind and command or script path

The file should represent the planned graph, not the observed runtime outcome.
Its purpose is to show what `nao` decided to execute and why individual tasks were eligible to run.

# What should `nao-events.jsonl` contain?

`nao-events.jsonl` should be a JSON Lines file that records run events in chronological order.
Each line should be one JSON object representing a single event.

Events should include the significant lifecycle transitions of the run, such as:

- run start
- task start
- task finish
- task skip
- run end

Task finish events should record the final status and exit code when an exit code exists.
Additional event types may be added as needed, but the event stream should remain append-only and ordered so it can be consumed incrementally while a run is in progress.

# How should per-task log files work?

Each task should have its own log file containing the combined output written by that task's stdout and stderr streams.
If a task never starts or produces no output, the implementation may still create an empty log file so the file set remains predictable.

Every log line should be prefixed with a timestamp in ISO-8601 format so readers can understand when output was produced.
When stdout and stderr are combined into one task log, each rendered line should identify which stream produced it.
Keeping one log file per task preserves readability during parallel task execution and allows users to inspect task output independently after the run.

The task log filename should make the corresponding task easy to identify.
Task log filenames must also be filesystem-safe and must not contain `:`.
Using a sanitized task name with a `.log` suffix is the intended format.

# What should `nao-summary.json` contain?

`nao-summary.json` should be written once the run has completed.
It should summarize the final outcome of the run and the final state of each task.

At a minimum, it should include:

- the overall run result
- an optional failure message when the run fails for an engine or process-execution reason
- overall run timing information
- one entry per task

Each task entry should include:

- the task result
- the task status
- the task log filename
- the exit code when one exists
- timing information

Task status values should include:

- `completed`
- `failed`
- `skipped`

Task result values should distinguish successful completion from failure or skip.

This file is the stable end-of-run summary that users and other tools should read when they need the final outcome rather than the full event stream.
