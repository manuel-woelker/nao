# What problem does this plan solve?

Contributors currently need to install Rust, Cargo, and any supporting CLI tooling directly on the host machine before they can work on `nao`.
That creates avoidable setup differences between contributors, makes onboarding slower, and leaves no shared definition of a preferred development environment for local editor use or AI-assisted workflows such as Codex.

This plan describes how to add a repository-owned devcontainer configuration so contributors can open the project in a consistent containerized workspace with Rust, Cargo, and Codex available out of the box.

# What is the current status?

The repository currently has:

- Rust workspace configuration and repository checks
- project-specific agent guidance in `AGENTS.md`
- no `.devcontainer/` directory
- no checked-in container definition for editor-based development
- no documented container bootstrap flow for Rust or Codex usage

The main missing pieces are:

- a devcontainer definition file
- a Dockerfile or base-image choice that installs Rust and Cargo predictably
- container bootstrap steps for repository checks and common tools
- a way to make Codex available inside the container
- documentation that explains how contributors should use the container

# What implementation approach should be used?

The implementation should add a standard `.devcontainer/` directory with a checked-in `devcontainer.json` and a repository-owned Dockerfile.
The Dockerfile should be explicit about the installed toolchain so the environment remains inspectable and easy to update in code review.

The first version should prioritize reproducibility and low maintenance over aggressive optimization.
It should provide a working Rust development environment, support Cargo-based workflows, and make Codex usable in the container, but it does not need to solve every optional editor integration or CI image reuse question in the first slice.

# Why should the repository use a Dockerfile instead of depending only on a prebuilt image?

A repository-owned Dockerfile keeps the environment definition visible next to the code and makes tool additions reviewable in the same workflow as code changes.
It also gives the project a straightforward place to document why specific packages are installed and how Rust, Cargo, and Codex are provisioned.

A prebuilt image could still be introduced later if startup time or image reuse becomes important, but the initial implementation should optimize for clarity and maintainability.

# What should the devcontainer include?

The first devcontainer should include:

- a current stable Rust toolchain
- Cargo and standard Rust development components such as `rustfmt` and `clippy`
- system packages needed to build common Rust crates used by this workspace
- Git and shell tooling expected for normal repository work
- a non-root development user with the workspace mounted in the container
- Codex installation or bootstrap steps so the coding agent can run inside the container

If Codex installation depends on credentials or a user-specific login flow, the configuration should provide the necessary runtime dependencies and document the remaining user action instead of attempting to hardcode secrets into the container.

# How should Codex support be handled?

Codex support should be treated as part of the development environment rather than as an afterthought.
The container should install Codex during image build so the resulting environment is immediately usable after the container is created.

If authenticated use still requires user credentials at runtime, the image should include the CLI and its runtime dependencies while documentation covers the remaining login step.
The implementation should avoid deferring Codex installation to a post-create step.

# How should Rust and Cargo be provisioned?

The container should install Rust through a standard, well-supported mechanism such as `rustup`.
That keeps the toolchain update path familiar to Rust contributors and makes it easy to add components like `clippy` and `rustfmt`.

The initial configuration should:

- install a pinned Rust toolchain version
- install `clippy` and `rustfmt`
- make `cargo` and `rustc` available on the default `PATH`
- avoid relying on the host machine’s Rust installation

The pinned version should live in a checked-in repository file or the devcontainer build definition so updates happen explicitly in code review.

# What editor integration should be configured?

The devcontainer should include the minimum settings needed for a productive Rust workflow in compatible editors such as VS Code or other environments that support the devcontainer specification.

The first version should consider:

- Rust language support extensions
- a shell and terminal profile that exposes Cargo and Codex on `PATH`
- optional post-create commands such as `cargo fetch` when they materially improve first use

The configuration should avoid heavy editor-specific customization that is unrelated to building and working on the repository.

# What files should likely be added?

The implementation will likely add:

- `.devcontainer/devcontainer.json`
- `.devcontainer/Dockerfile`
- optional helper scripts under `.devcontainer/` if setup steps become too long for inline commands
- documentation updates in `README.md` or a dedicated developer setup document if needed

# What implementation order is recommended?

The recommended order is:

1. Define the target developer experience for opening the repository in a devcontainer.
2. Add a Dockerfile that installs Rust, Cargo, and supporting system packages.
3. Add `devcontainer.json` with workspace mount, user, and editor/tool settings.
4. Add Codex installation or bootstrap support inside the container.
5. Document how to open the container and verify the environment.
6. Verify the container by building the workspace and running the repository checks inside it.

# What concrete work items should be tracked?

- [x] Add `.devcontainer/devcontainer.json`.
- [x] Add a repository-owned `.devcontainer/Dockerfile`.
- [x] Install stable Rust via `rustup` in the container.
- [x] Pin the Rust toolchain version used by the devcontainer.
- [x] Install `cargo`, `rustfmt`, and `clippy` in the container environment.
- [x] Install the system packages needed for this Rust workspace to build successfully.
- [x] Configure a non-root development user.
- [x] Install Codex in the container image at build time.
- [x] Add devcontainer editor settings and extensions appropriate for Rust development.
- [x] Document how to use the devcontainer for normal development and Codex-assisted development.
- [ ] Verify that `cargo build` works inside the container.
- [ ] Verify that `cargo fmt --all` works inside the container.
- [ ] Verify that `cargo clippy` works inside the container.
- [ ] Verify that `./scripts/check-code.sh` works inside the container.

# How should the work be verified?

Verification should include:

- building the devcontainer successfully from a clean checkout
- confirming `rustc --version` and `cargo --version` inside the container
- confirming `cargo fmt --all` and `cargo clippy` run successfully inside the container
- confirming Codex is present in the built container and can start inside it
- running `./scripts/check-code.sh` inside the container

If Codex activation depends on user credentials, verification should explicitly distinguish between:

- environment verification, where the container has the required runtime dependencies, and
- authenticated usage verification, where a logged-in user confirms the tool runs correctly

# What assumptions should remain explicit?

This plan assumes:

- contributors will use a devcontainer-compatible editor or CLI workflow
- a Linux-based container image is acceptable for the first version
- a pinned Rust version is preferred over tracking the latest stable toolchain automatically
- Codex should be installed directly in the image build rather than post-create
- the initial devcontainer does not need to mirror CI perfectly as long as repository checks pass inside it
- extra debugging tools are out of scope for the first version

# What risks or open questions matter most?

The main risks are:

- adding a container image that is heavier or slower to rebuild than necessary
- introducing Codex setup steps that depend on undocumented credentials or local host assumptions
- overfitting the configuration to one editor instead of the devcontainer spec itself
- missing system packages required by some Rust dependencies in this workspace

The main open questions are:

- which pinned Rust version should be used initially
- which exact Codex installation path is most reliable for non-interactive image builds
