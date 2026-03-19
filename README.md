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

## How can I use the devcontainer?

This repository includes a checked-in devcontainer under [`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json).
Open the repository in a devcontainer-compatible editor or CLI workflow to get a container with:

- pinned Rust via the devcontainer build definition
- Cargo, `rustfmt`, and `clippy`
- build-time Codex CLI installation

The container runs [`bash .devcontainer/post-create.sh`](.devcontainer/post-create.sh) after creation to print tool versions and prefetch Cargo dependencies.
Codex is installed in the image, but authenticated usage may still require logging in with your own credentials once the container starts.
