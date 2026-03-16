# What is `now`?

`now` is a task runner for developers and operators who need the same workflow to behave consistently on a laptop and in CI. It runs tasks in parallel, accounts for dependencies between tasks, and keeps concurrent output separated so each task can be read in isolation instead of being mixed together in a single interleaved log stream.

This document is a work in progress and reflects the current direction of the project.

# Why does `now` exist?

Many task runners and CI systems force a tradeoff between efficient execution and readable logs. Parallel execution improves throughput, but concurrent output often becomes hard to follow when multiple tasks write to the terminal at the same time.

`now` is intended to remove that tradeoff. It preserves the performance benefits of parallel execution while making the result easier to understand. It also aims to bridge local and CI execution so teams do not need one mental model for development and another for automation.

# Who is `now` for?

`now` is aimed at:

- Developers running builds, tests, checks, and supporting automation locally
- Operators and CI maintainers who want reproducible automation that is not tied to a specific CI provider

The core idea is that the same task graph should be usable in both environments without being re-expressed in provider-specific pipeline logic.

# What is a task in `now`?

A task is a unit of work that `now` can execute as part of a larger graph. In practice, that can include:

- A shell command
- A Bash script
- A container invocation such as a Docker run
- Other executable steps with clear inputs, outputs, and dependencies

Tasks are named, and those names are used to express relationships between them.

# How are dependencies modeled?

Dependencies are modeled by naming prerequisite tasks. This lets `now` understand which tasks can start immediately, which tasks must wait, and which tasks become eligible once earlier work completes.

At a high level, this means `now` can execute independent tasks in parallel while still respecting the required ordering between related steps.

# What is the main feature of `now`?

The defining feature of `now` is its output model. Even when tasks run concurrently, their output remains clearly separated and easy to inspect per task. That is a significant usability improvement over traditional parallel execution where output from multiple tasks is interleaved into one stream.

This makes debugging faster, reduces cognitive overhead, and improves the ergonomics of running large task graphs locally and in CI.

# What else does `now` emphasize?

In addition to parallel execution and dependency awareness, `now` focuses on:

- Good error reporting, so failures can be located and understood quickly
- Timing information, so users can see where time is being spent
- Artifact handling, so produced outputs can be managed as part of task execution
- Reproducibility, so the same graph behaves consistently across environments

These features matter both for developer productivity and for reliable automation.

# How does failure handling work?

`now` supports two execution modes:

- Fail-early, where execution stops as soon as the first task fails
- Fail-late, where `now` continues to run as much of the graph as possible, but does not run tasks that depend on failed prerequisites

This allows teams to choose between fast feedback and maximum information gathering, depending on the situation.

# Why use `now` in CI?

`now` is designed to make task execution independent of the CI provider. Instead of encoding core workflow behavior directly in provider-specific configuration, teams can define the task graph once and run it both locally and in CI.

That helps reduce drift between environments, lowers the cost of changing CI providers, and makes local reproduction of CI behavior more straightforward.

# What should readers know about the implementation?

`now` is implemented in Rust. That choice supports building a fast, reliable command-line tool with strong control over execution, process management, and reporting.

From a user perspective, the implementation language is less important than the outcome: a portable task runner intended to provide predictable behavior, clear reporting, and efficient execution across environments.

# What does using `now` look like?

Consider a repository with these tasks:

- `lint`
- `unit-test`
- `build`
- `package`
- `publish`

The graph might look like this:

- `lint` has no dependencies
- `unit-test` depends on `build`
- `package` depends on `build`
- `publish` depends on `package`

In that setup, `now` can start `lint` and `build` in parallel. Once `build` completes, it can start `unit-test` and `package`. Once `package` completes, it can start `publish`.

If output from `lint`, `build`, and `unit-test` is produced at the same time, `now` keeps that output separated by task so each stream remains readable. If `build` fails in fail-late mode, `lint` may still complete, but `unit-test`, `package`, and `publish` would not run because they depend on `build`.

# What is the current status of this document?

This overview is intentionally high level. The project is still a work in progress, so the details of configuration, execution model, artifact handling, and reporting may evolve as the implementation matures.
