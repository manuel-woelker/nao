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
See [`docs/RELEASE.md`](docs/RELEASE.md) for the release process.

## How can I use the TUI?

`nao` includes a full-screen terminal UI behind the `--tui` flag.
Run it from a repository root that contains `nao.kdl` with:

```bash
cargo run -p nao -- --tui
```

Or point it at a different recipe file:

```bash
cargo run -p nao -- --tui --config path/to/nao.kdl
```

The first version provides:

- a task launcher for choosing one or more goal tasks
- a run detail screen for live and completed runs
- a run history screen backed by `.nao/runs`
- task log, event stream, and summary browsing without leaving the TUI

Key bindings:

- `1`, `2`, `3` switch between launcher, run detail, and history
- `Tab` and `Shift-Tab` cycle pane focus
- `j` and `k` move the current selection or scroll the focused pane
- `Space` toggles launcher task selection
- `Enter` starts a run from the launcher or opens a run from history
- `t`, `o`, `e`, and `s` focus tasks, output, events, and summary in run detail
- `L` toggles log auto-follow for active runs
- `?` opens help and `q` exits

## How can I use the devcontainer?

This repository includes a checked-in devcontainer under [`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json).
Open the repository in a devcontainer-compatible editor or CLI workflow to get a container with:

- pinned Rust via the devcontainer build definition
- Cargo, `rustfmt`, and `clippy`
- build-time Codex CLI installation

The container runs [`bash .devcontainer/post-create.sh`](.devcontainer/post-create.sh) after creation to print tool versions and prefetch Cargo dependencies.
Codex is installed in the image, but authenticated usage may still require logging in with your own credentials once the container starts.

## How can I use Flox for a reproducible Rust sandbox?

This repository includes a checked-in Flox environment under [`.flox/env/manifest.toml`](.flox/env/manifest.toml).
It pins the Rust toolchain and common development commands used by this project:

- `rustc`
- `cargo`
- `rustfmt`
- `clippy`
- `cargo-nextest`
- native build helpers such as `gcc` and `pkg-config`

Activate it with:

```bash
flox activate
./dev-shell.sh
```

The environment keeps Cargo state inside `.flox/cache`, which helps isolate builds from host machine state and keeps local sandboxing predictable.

Common workflows:

```bash
./dev-shell.sh ./scripts/check-code.sh
./dev-shell.sh cargo build --workspace
./dev-shell.sh cargo nextest run --workspace --all-targets --all-features
flox activate -- ./scripts/check-code.sh
flox activate -- cargo build --workspace
flox activate -- cargo nextest run --workspace --all-targets --all-features
```

Use Flox when you want a reproducible local toolchain without opening the devcontainer.
Use the devcontainer when you want a fully containerized editor or CLI environment.
Use [`./dev-shell.sh`](./dev-shell.sh) when you want the shortest path into the checked-in Flox environment.

## How can I run Codex with less host filesystem access?

Use [`scripts/run-codex-sandbox.sh`](scripts/run-codex-sandbox.sh) on Linux to start Codex inside a `bubblewrap` sandbox.
The wrapper mounts the repository read-write, mounts the Codex state directory under `~/.codex`, mounts the Cargo and Rustup homes so Cargo-installed tools such as `cargo-nextest` remain available, mounts the pnpm-installed Codex package read-only, and hides the rest of `/home`.

Run:

```bash
./scripts/run-codex-sandbox.sh
```

Pass normal Codex arguments after the script name.
If you need to expose additional host paths, set `NAO_CODEX_EXTRA_RO_BIND` or `NAO_CODEX_EXTRA_RW_BIND` to colon-separated absolute paths before launching the wrapper.
