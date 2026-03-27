# Why is this plan needed?

`nao` already parses `run container="..."` task definitions and documents container execution as a supported workflow, but the engine still fails at runtime with a "not implemented yet" error.
That gap is now misleading enough that it should be treated as product debt rather than a nice-to-have.

This plan describes how to implement Docker-based task execution in a way that fits the repository's new `.docker/` convention and stays small enough for a first useful slice.

# What problem should the first implementation solve?

The first implementation should make documented container tasks actually runnable on a developer machine that has Docker available.
It should prefer a boring, inspectable execution model over a large abstraction layer.

The first slice should cover:

- executing `run container="..."` tasks with Docker
- passing positional `args` through to the container process
- mounting the repository workspace into the container
- running the container with the recipe directory as the working directory
- forwarding task environment variables into the container
- streaming logs and exit codes through the existing process execution pipeline
- documenting how repository-owned Docker assets under `.docker/` relate to task execution

The first slice should not try to solve every container workflow immediately.
In particular, it does not need to add Podman support, custom networks, port mappings, secrets management, or a full Compose-driven task model on day one.

# What execution model should `nao` use first?

The simplest correct implementation is to treat a container task as a generated `docker run` command.
That keeps the engine aligned with the existing process model instead of building a second execution subsystem.

For the current recipe format:

- `run container="ghcr.io/example/tool:latest"` maps to the container image
- `args ...` map to positional arguments passed after the image name
- task `env` entries become `docker run --env NAME=value`
- the repository workspace should be mounted into the container
- the container working directory should match the task working directory inside that mount
- the task should fail if `docker` is not installed or the image cannot be pulled or started

That means `build_process_command()` can keep returning a normal `ProcessCommand`, but for `RunSpec::Container` it should now synthesize a Docker CLI invocation instead of returning an error.

# How should the `.docker/` directory fit into task execution?

The `.docker/` directory should be the repository-owned home for Docker assets that support development and task execution.
The current `.docker/Dockerfile` and `.docker/docker-compose.yaml` already establish that pattern.

For task execution, the first implementation should treat `.docker/` as documentation and asset storage rather than as a required runtime protocol.
That means:

- container tasks may reference any image string the user provides
- repository-owned Dockerfiles used by tasks should live under `.docker/`
- documentation should show examples such as `.docker/tasks/<name>/Dockerfile`
- if a task depends on a locally built image, the build step should be explicit in the task graph rather than hidden inside `nao`

This is the right tradeoff for now because the current recipe format names an image, not a build context.
Trying to infer image builds from `.docker/` would be clever in the bad way.

# What concrete behavior should container tasks have?

Container tasks should behave as close as possible to shell and script tasks from the user's perspective.

Recommended first-pass behavior:

- mount the recipe workspace at a stable in-container path such as `/workspace`
- set `--workdir` to the recipe directory relative to that mount
- pass task environment variables with `--env`
- run with `--rm` so one-shot task containers do not accumulate
- attach stdout and stderr normally so the existing log framing keeps working
- return the container process exit code as the task exit code

Recommended path mapping rules:

- if the recipe file is `.nao/nao.kdl`, mount the repository root and use `/workspace` as the container working tree
- if the recipe file is elsewhere, mount that recipe parent directory and still expose it as `/workspace`
- script paths and artifact paths remain host-relative in the recipe model; container tasks only receive the mounted workspace and their CLI arguments

# What product and UX details matter most?

The biggest UX risk is making container tasks feel magical when they are actually just `docker run` with a workspace mount.
The docs should say that plainly.

The CLI and artifacts should make container runs inspectable:

- `nao-plan.json` should keep recording the structured container spec
- task logs should show the generated `docker run` failure output when startup fails
- user-facing docs should explain that the first version requires Docker on `PATH`
- if `docker` is missing, the error should say that container tasks require Docker and name the task that failed

This is also a good place to stop overselling support.
The docs should describe what is real now, not what might exist later.

# What code should change?

The initial implementation will likely touch these areas:

- [crates/engine/src/run_engine/process_command.rs](/data/projects/nao/crates/engine/src/run_engine/process_command.rs)
  Generate a `docker run` `ProcessCommand` for `RunSpec::Container`.
- [crates/recipe/src/run_spec.rs](/data/projects/nao/crates/recipe/src/run_spec.rs)
  Keep the current spec unless a small helper method improves path mapping or argument rendering.
- [crates/engine/src/run_artifact_writer.rs](/data/projects/nao/crates/engine/src/run_artifact_writer.rs)
  Ensure the serialized plan output remains useful for container tasks.
- [docs/RECIPES.md](/data/projects/nao/docs/RECIPES.md)
  Document the real execution model and show `.docker/`-based examples.
- [README.md](/data/projects/nao/README.md)
  Explain the difference between the devcontainer, the `.docker` helper image, and task containers if needed.

If path handling becomes noisy, introduce a small helper module near `process_command.rs` rather than bloating one function.

# What order should the implementation follow?

1. Define the exact `docker run` shape for container tasks, including mount path, workdir mapping, and environment forwarding.
2. Implement `RunSpec::Container` command generation in the engine.
3. Add colocated tests for command generation, especially for `.nao/nao.kdl` and custom `--config` paths.
4. Add execution tests that prove container tasks succeed, fail, and emit output correctly when Docker is available.
5. Update user-facing docs to describe the real behavior and the `.docker/` convention.
6. Run repository-wide verification and a manual container smoke test.

# What checklist should track the work?

- [ ] Define and document the first-pass Docker execution contract for container tasks.
- [ ] Implement `RunSpec::Container` as a generated `docker run` command instead of returning a not-implemented error.
- [ ] Mount the recipe workspace into the container at a stable path.
- [ ] Map the task working directory into the container correctly for both `.nao/nao.kdl` and custom recipe paths.
- [ ] Forward task environment variables into the container.
- [ ] Add colocated unit tests for generated Docker command arguments.
- [ ] Add execution tests that cover a successful container task.
- [ ] Add execution tests that cover a failing container task and preserved error output.
- [ ] Update `docs/RECIPES.md` to show a `.docker/`-based example that matches the implemented behavior.
- [ ] Update `README.md` if the current Docker sections need clearer separation between development containers and task containers.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include both command-generation tests and at least one real execution path.

Expected verification steps:

- colocated unit tests for generated `docker run` commands
- engine tests covering workspace mount and workdir mapping
- a manual smoke test with a tiny image such as `alpine` or `bash` that writes output and returns a known exit code
- a manual test using a repository-owned Dockerfile under `.docker/` to show the recommended project pattern
- `./scripts/check-code.sh`

If real Docker-based execution cannot run in CI yet, the repository should still have deterministic unit tests and clearly document the manual verification gap.

# What assumptions and follow-up questions should stay explicit?

This plan assumes:

- Docker is the first supported runtime for container tasks
- requiring Docker on `PATH` is acceptable for the first version
- the existing recipe format remains image-based rather than context-based
- one mounted workspace root is sufficient for the first slice

Follow-up work that should stay out of scope for this plan:

- supporting Podman or alternative runtimes
- adding container build steps to the recipe format
- Compose-native task execution
- custom bind mounts, ports, secrets, or networks
- artifact copying from anonymous container filesystems that are not under the mounted workspace

Those features may be useful later, but shipping the basic `docker run` path first is the highest-value move.
