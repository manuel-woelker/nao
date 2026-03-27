# Why is this plan needed?

`nao` now supports one-shot container tasks through generated `docker run` commands.
That covers simple isolated commands well, but it is the wrong abstraction for multi-service workflows that need named volumes, networks, or supporting services such as databases and queues.

This plan describes how to add a separate Compose-backed task type using a recipe shape like:

```kdl
task "integration-test" {
  run compose=".docker/integration" service="test-runner"
}
```

The goal is to support Compose-based workflows without polluting the simpler `run container="..."` model.

# What problem should the first implementation solve?

The first implementation should let a task run one declared Compose service from a checked-in Compose project directory.
It should be good enough for integration tests and packaging steps that depend on helper services, but it should not try to expose the entire Compose feature surface through KDL.

The first slice should cover:

- parsing `run compose="<directory>" service="<service>"`
- resolving the Compose directory relative to the recipe workspace
- running the named service through `docker compose`
- forwarding task `args` after the service name
- forwarding task `env` into the `docker compose run` invocation
- keeping logs and exit codes inside the existing process execution pipeline
- documenting how Compose project files should live under `.docker/`
- adding a real `nao` task that uses the checked-in Compose project under `.docker/docker-compose.yaml`

The first slice should not try to add per-task KDL syntax for volumes, networks, ports, or secrets.
Those belong in the Compose project itself.

# What should the recipe model look like?

The new execution form should be explicit and separate from `shell`, `script`, and `container`.
That keeps the mental model clean.

Recommended syntax:

```kdl
task "integration-test" {
  env RUST_LOG="warn"
  run compose=".docker/integration" service="test-runner" {
    args "--filter" "api"
  }
}
```

Recommended first-pass parsing rules:

- `compose` is a required string property naming the Compose project directory
- `service` is a required string property naming the Compose service to run
- `args` child nodes work the same way they do for container tasks
- `run` must still define exactly one execution mode

This should become a new `RunSpec::Compose` variant with a dedicated spec struct.

# How should Compose execution work?

The simplest useful model is to treat the Compose task as a generated `docker compose run --rm` command.
That keeps the engine aligned with the current process runner and avoids building a daemon client or a bespoke orchestration layer.

Recommended first-pass command shape:

```text
docker compose --project-directory <resolved-dir> run --rm [--env NAME=value ...] <service> [args...]
```

Key points:

- use `--project-directory` so Docker Compose resolves files and relative paths from the declared Compose directory
- run the service with `run --rm` so task containers do not accumulate
- pass task `env` entries as CLI `--env` flags rather than mutating the process environment
- append task `args` after the service name
- use the existing process pipeline for stdout, stderr, and exit code handling

This first slice should not automatically run `compose up` or `compose down`.
If the Compose project needs dependent services, Compose itself should start them as part of `run`, or the user should model startup explicitly in the service definition.

# How should the `.docker/` directory fit into this feature?

The Compose directory named in the recipe should usually live under `.docker/`.
That matches the repository's existing convention for Docker-owned assets and keeps container orchestration files out of the repository root.

Recommended repository pattern:

- `.docker/integration/docker-compose.yaml`
- `.docker/integration/.env` when needed
- optional Dockerfiles under that same subtree

The task should reference the directory, not the YAML file path.
That keeps the KDL small and lets the Compose project own its internal file layout.

# What boundaries should stay firm?

The main design risk is letting Compose-specific concerns leak into the general recipe format.
That would create a messy hybrid model quickly.

The first implementation should keep these boundaries:

- `run container="..."` stays the one-shot single-container mode
- `run compose="..." service="..."` is the multi-service mode
- volumes, networks, ports, health checks, and build settings stay in Compose YAML
- KDL only names the Compose project directory, service, environment variables, and positional args

If the feature needs more power later, that should happen by extending the Compose spec carefully, not by duplicating Compose YAML in KDL.

# What code should change?

The implementation will likely touch these areas:

- [crates/recipe/src/run_spec.rs](/data/projects/nao/crates/recipe/src/run_spec.rs)
  Add a Compose run spec and `RunSpec::Compose`.
- [crates/recipe/src/parse_recipe.rs](/data/projects/nao/crates/recipe/src/parse_recipe.rs)
  Parse `compose` and `service` properties and reuse `args` parsing.
- [crates/engine/src/run_engine/process_command.rs](/data/projects/nao/crates/engine/src/run_engine/process_command.rs)
  Generate the `docker compose --project-directory ... run --rm ...` command.
- [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
  Persist Compose run metadata in `nao-plan.json`.
- [crates/cli/src/help_text.rs](/data/projects/nao/crates/cli/src/help_text.rs)
  Document the new task type.
- [docs/RECIPES.md](/data/projects/nao/docs/RECIPES.md)
  Show the supported syntax and clarify that volumes and related settings belong in the Compose project.
- [.nao/nao.kdl](/data/projects/nao/.nao/nao.kdl)
  Add a checked-in task that runs the repository Compose service.

# What order should the implementation follow?

1. Define the Compose run spec in the recipe model.
2. Extend parsing validation so `run` still accepts exactly one execution mode.
3. Generate the Docker Compose CLI command in the engine.
4. Serialize the Compose execution details into run artifacts.
5. Add unit tests for parsing and command generation.
6. Add execution tests for successful and failing Compose-backed tasks.
7. Add a checked-in example task that runs the repository Compose service.
8. Update help text and recipe documentation.
9. Run repository-wide verification.

# What checklist should track the work?

- [ ] Add a `RunSpec::Compose` variant and a dedicated spec struct.
- [ ] Parse `run compose="..." service="..."` with optional `args`.
- [ ] Reject `run` nodes that mix `compose` with other execution modes.
- [ ] Resolve Compose directories relative to the recipe workspace.
- [ ] Generate `docker compose --project-directory ... run --rm ...` commands.
- [ ] Forward task environment variables as Compose CLI `--env` flags.
- [ ] Persist Compose run metadata in `nao-plan.json`.
- [ ] Add parser tests for valid and invalid Compose task definitions.
- [ ] Add engine tests for generated Compose commands.
- [ ] Add execution tests for successful and failing Compose-backed tasks.
- [ ] Add a `nao` task that runs the checked-in Compose service from `.docker/`.
- [ ] Update CLI help text to document the new execution mode.
- [ ] Update `docs/RECIPES.md` with a `.docker/`-based example.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include both parser coverage and engine coverage.

Expected verification steps:

- parser tests for valid Compose tasks
- parser tests for conflicting `run` mode definitions
- engine tests for generated `docker compose` arguments
- execution tests showing that stdout, stderr, exit codes, and task outcomes still flow through the existing pipeline
- `./scripts/check-code.sh`

If live Compose execution is not available in automated checks, deterministic mocked execution tests are still required, and any manual daemon-level verification gap should be called out explicitly.

# What assumptions and follow-up questions should stay explicit?

This plan assumes:

- Docker Compose is available through `docker compose`
- the Compose directory contains a standard Compose project layout that Docker can resolve from `--project-directory`
- naming a directory is sufficient for the first version
- one service per task is the right initial granularity

Follow-up questions that should remain out of scope for this plan:

- whether tasks should support `docker compose exec` in addition to `run`
- whether teardown helpers or project lifecycle orchestration deserve first-class recipe support
- whether multiple Compose files should be supported in one task
- whether Podman Compose compatibility matters
- whether Compose project names should be configurable from KDL

Those may be worth exploring later, but the first version should stay narrow and predictable.
