# What is a `nao` recipe file?

A `nao` recipe file describes tasks, their dependencies, and the information needed to execute them. The format is intended to be KDL-based so it stays readable in source control, supports nesting naturally, and remains pleasant to edit by hand.

This document is a work in progress and describes the current intended direction of the format.

# Why use KDL for recipe files?

KDL is a good fit for task descriptions because it supports:

- Named nodes that map well to tasks and configuration blocks
- Properties for compact key-value metadata
- Nested structure for commands, environment, artifacts, and execution settings
- A syntax that stays readable without large amounts of punctuation

That makes it suitable for both simple local workflows and larger CI-oriented task graphs.

# What should a recipe file describe?

At a high level, a recipe file should describe:

- Which tasks exist
- How tasks are named
- Which tasks depend on other tasks
- How each task should run
- Which inputs, environment values, and artifacts matter for reproducible execution

The exact schema may evolve, but those concepts are the core of the format.

# How are tasks represented?

Tasks are represented as named KDL nodes. A task node should give `nao` enough information to understand what the task is called, how it runs, and when it is allowed to start.

In practice, that means a task definition will usually include:

- A task name
- One or more dependency names
- An execution mode such as shell, script, or container
- Optional environment variables
- Optional artifact declarations

# What naming rules apply to tasks?

Task names should be simple literal identifiers that are easy to type on the command line.

Today, task names must not contain `_`.
`nao` reserves `_` for wildcard task selectors such as `test_`, which can match multiple tasks without forcing shell quoting.

For example:

- `test_` can match tasks whose names start with `test`
- `lint` remains an exact task name

If you need multi-word task names, prefer `-` over `_`.

# How are dependencies expressed?

Dependencies are expressed by naming prerequisite tasks in the task definition. This keeps the graph explicit and easy to inspect in code review.

Because dependencies are name-based, `nao` can determine which tasks may start immediately and which tasks must wait for earlier work to complete.

# How is recipe-wide execution configured?

Recipe-wide execution settings live in an optional `config` node.

The current supported properties are:

- `live-display`, which chooses how interactive terminal progress is rendered
- `max-parallel-tasks`, which limits how many task processes may run at once

If `max-parallel-tasks` is omitted, it defaults to the platform-reported core count.

# What might a recipe file look like?

The following example shows the intended shape of a KDL recipe file:

```kdl
recipe "default" {
  config live-display="line-per-task" max-parallel-tasks=4

  task "build" {
    run shell="cargo build --workspace"
    artifact "workspace-target" path="target"
  }

  task "lint" {
    run shell="cargo clippy --workspace --all-targets --all-features -- -D warnings"
  }

  task "test" {
    depends-on "build"
    run shell="cargo nextest run --workspace --all-targets --all-features"
  }

  task "verify-docs" {
    run script="./scripts/check-docs.sh"
    env RUST_LOG="warn"
  }

  task "image" {
    depends-on "build"
    run container="ghcr.io/example/packager:latest" {
      args "--input" "target" "--output" "dist/image.tar"
    }
    artifact "container-image" path="dist/image.tar"
  }

  task "ci" {
    depends-on "lint"
    depends-on "test"
    depends-on "verify-docs"
    depends-on "image"
  }
}
```

# What does the example show?

The example shows several important ideas:

- `build`, `lint`, and `verify-docs` can start immediately because they have no prerequisites
- `test` and `image` wait for `build`
- `ci` acts as a coordination task that depends on multiple other tasks
- Execution can be expressed in different forms, including shell commands, scripts, and containers
- Artifacts can be declared explicitly so produced outputs become part of the task description

# How can a task report a short outcome summary?

Tasks may report a human-readable outcome by printing a line that begins with `Task outcome: `.
When multiple matching lines are printed, the last one wins.

For example:

```sh
printf 'Task outcome: 30 tests succeeded\n'
```

The outcome line remains in the task log and is also stored in structured run artifacts for the CLI and TUI.

# What parts of the format are still open?

The broad shape is clear, but several details are still work in progress, including:

- The exact top-level file structure
- The final names of nodes and properties
- How artifact metadata should be modeled
- How reproducibility settings should be expressed
- How task-level failure behavior and execution policies should be configured

This document should be treated as a design guide for the format, not as a frozen specification yet.
