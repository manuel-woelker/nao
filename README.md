# `nao`

`nao` is a task runner for local development and CI. It runs tasks in parallel, respects dependencies between tasks, and keeps concurrent output separated so logs stay readable.

This project is a work in progress.

## Why use `nao`?

- Use the same task graph locally and in CI
- Run independent work in parallel
- Keep task output easy to inspect
- Get useful failure reporting and timing information
- Stay independent of a specific CI provider

## What can a task be?

A task can be a shell command, a Bash script, a container invocation, or another executable step.

## What should I read next?

See [`docs/OVERVIEW.md`](docs/OVERVIEW.md) for the higher-level project overview.
