# What problem does this plan solve?

The repository now has a Flox-based development shell and a separate `bubblewrap` sandbox for Codex, but it does not yet have one repository-owned developer entrypoint that feels like its own little universe.
Today, reproducible tooling and process isolation are split across different workflows:

- Flox provides a pinned Rust toolchain and isolated Cargo cache paths
- `scripts/run-codex-sandbox.sh` provides Linux-only filesystem sandboxing for Codex
- interactive shell startup still inherits normal host shell behavior unless each tool path works around it

This plan describes how to add a Nix-based development shell plus a sandbox wrapper so contributors can enter a clean, reproducible, intentionally isolated environment for normal repository work.

# What is the current status?

The repository already has:

- a Rust workspace with repository-owned check scripts
- a checked-in Flox environment under `.flox/`
- a repo-root `dev-shell.sh` wrapper that activates Flox
- a Linux `bubblewrap` sandbox script for Codex under `scripts/run-codex-sandbox.sh`
- a documented devcontainer for a fully containerized workflow

The repository does not yet have:

- a checked-in Nix shell definition such as `flake.nix`
- a repository-owned Nix developer shell entrypoint
- a sandboxed developer shell that combines reproducible tooling with reduced host visibility
- one documented workflow that makes the shell startup behavior intentionally quiet and self-contained

The gap is not "we need another package manager because shiny."
The real gap is that the current dev shell is reproducible but still feels like the host machine wearing a different jacket.

# What should "its own little universe" mean in this repository?

For this work, "its own little universe" should mean:

- the Rust toolchain and supporting build tools come from a checked-in Nix definition
- entering the shell does not source the user's normal interactive shell config
- Rust and Cargo state is redirected into explicit persistent cache directories instead of leaking into default host-global paths
- the visible filesystem is intentionally limited, at least on Linux
- the workflow works both interactively and for one-shot commands such as `cargo build`

It should not claim stronger guarantees than it actually provides.
If networking remains available or some host paths remain mounted for practicality, the documentation should say so directly.

# Why use Nix plus a sandbox instead of Nix alone?

Nix solves reproducible dependencies well.
It does not, by itself, create a hermetic interactive session.

`nix develop` still launches a shell process in the host environment unless the repository wraps it in a cleaner shell invocation or an outer sandbox.
If the goal is a shell that behaves like a contained workspace instead of "your normal shell, but with different binaries on `PATH`", the implementation needs both:

- Nix for the toolchain and build inputs
- `bubblewrap` or a similar sandbox for process and filesystem isolation

That split should be explicit in both the implementation and the documentation.

# What implementation approach should be used?

The first version should stay narrow and practical:

1. add a checked-in `flake.nix` that defines the Rust development shell
2. add a repository-owned wrapper script that enters that shell without loading host shell config
3. add a Linux sandbox wrapper that runs the Nix shell inside `bubblewrap`
4. document the differences between the plain Nix shell, the sandboxed Nix shell, the devcontainer, and the existing Flox workflow

The first version does not need to replace Flox immediately.
Trying to delete the working Flox path in the same change would be a good way to create churn and break onboarding for no real gain.

# What should the Nix development shell include?

The Nix shell should provide the same baseline developer tooling currently expected by repository checks and normal Rust work:

- `rustc`
- `cargo`
- `rustfmt`
- `clippy`
- `cargo-nextest`
- `gcc`
- `pkg-config`
- any other native build tools proven necessary by `./scripts/check-code.sh`

The shell hook should set explicit persistent cache paths such as:

- `CARGO_HOME`
- `CARGO_INSTALL_ROOT`
- `CARGO_TARGET_DIR`
- `RUSTUP_HOME` if the chosen Nix workflow needs it for auxiliary tooling

Those paths should not point at the usual default Cargo directories under the user's home directory.
They should instead live in a dedicated persistent location such as `~/.cache/nao/nix-dev-shell/` or another clearly namespaced path chosen by the implementation.

That gives the shell three properties at once:

- persistent caches survive across shell sessions
- the cache location is explicit and reviewable instead of implicit host state
- the sandbox can mount only the exact cache paths it needs instead of exposing the whole home directory

# How should the integrated shell entrypoint behave?

The repository should add a dedicated wrapper such as `scripts/run-dev-shell.sh` or a repo-root `dev-shell.sh` replacement that:

- verifies `nix` is installed
- resolves the repository root
- starts the Nix dev shell from that root
- uses a quiet shell mode such as `bash --noprofile --norc` or `fish --no-config`
- forwards commands when arguments are provided
- opens an interactive shell when no command is provided

That wrapper should optimize for boring predictability.
It should not try to auto-detect every possible user shell and support them all equally in the first version.
The default interactive shell should likely be `bash` because it is the least surprising baseline for scripted behavior.

# How should the Linux sandbox wrapper behave?

The repository should add a second wrapper, likely under `scripts/`, that launches the Nix shell inside `bubblewrap`.
That wrapper should reuse the existing `scripts/run-codex-sandbox.sh` design where it makes sense instead of duplicating the same bind-mount logic in a slightly different flavor.

The sandboxed shell should:

- require Linux
- require `bwrap`
- mount the repository read-write
- mount `/nix/store` read-only
- mount the minimum host paths needed for Nix to work correctly
- mount the persistent Rust and Cargo cache directories read-write
- use tmpfs for transient locations such as `/tmp`
- set a deliberate working directory inside the repository
- avoid sourcing host shell startup files

The first version should be conservative about what host paths remain visible.
If exposing the user's full home directory is unnecessary, do not expose it just because it is convenient.
Mounting a narrow persistent cache root is the right compromise here.

# What relationship should this have to the existing Flox environment and Codex sandbox?

The repository should treat this as a new path, not as an instant replacement for everything else.

The plan should preserve:

- Flox as the current lightweight reproducible shell until the Nix path is proven
- `scripts/run-codex-sandbox.sh` as the Codex-specific sandbox entrypoint unless or until the Nix sandbox clearly supersedes it

Possible follow-up directions after the first version:

- keep both Flox and Nix if they serve different contributor preferences
- migrate `dev-shell.sh` from Flox to Nix once the new path is stable
- refactor shared sandbox assembly logic to avoid maintaining two independent `bubblewrap` scripts

The first implementation should not try to solve all three follow-up decisions at once.

# What files should likely be added or changed?

The implementation will likely add:

- `flake.nix`
- optionally `flake.lock`
- a repository-owned shell wrapper such as `dev-shell.sh` or `scripts/run-dev-shell.sh`
- a Linux sandbox wrapper such as `scripts/run-dev-shell-sandbox.sh`
- documentation updates in `README.md`

The implementation may also update:

- existing sandbox helper code if bind-mount logic is shared with the Codex sandbox
- documentation for any persistent cache root the shell owns

# What implementation order is recommended?

The recommended order is:

1. Define the target UX for plain `nix develop`, the clean dev shell wrapper, and the sandboxed wrapper.
2. Add `flake.nix` with the Rust toolchain and persistent Cargo cache configuration.
3. Add the non-sandboxed shell wrapper that starts a quiet interactive shell or forwards commands.
4. Verify the plain wrapper by running repository checks inside it.
5. Add the Linux `bubblewrap` wrapper around the Nix shell.
6. Verify the sandboxed wrapper with both interactive entry and forwarded commands.
7. Document when to use Flox, the Nix shell, the sandboxed Nix shell, and the devcontainer.

# What concrete work items should be tracked?

- [ ] Add `flake.nix` with a repository-owned Rust development shell.
- [ ] Pin the Rust and supporting tool versions through the Nix inputs used by the shell.
- [ ] Configure explicit persistent Rust and Cargo cache paths for the Nix shell.
- [ ] Add a developer shell wrapper that starts a quiet shell without loading host shell config.
- [ ] Make the shell wrapper forward one-shot commands correctly.
- [ ] Add a Linux-only sandbox wrapper for the Nix shell using `bubblewrap`.
- [ ] Reuse or refactor existing `bubblewrap` helper logic where that reduces duplication with `scripts/run-codex-sandbox.sh`.
- [ ] Ensure the sandboxed wrapper mounts only the minimum host paths required for the workflow.
- [ ] Ensure the sandboxed wrapper mounts the persistent cache paths needed for fast rebuilds.
- [ ] Document the new Nix-based workflows in `README.md`.
- [ ] Document the limits of the sandbox clearly, including any remaining network or host-path visibility.
- [ ] Document where the persistent caches live and how to clear them intentionally.
- [ ] Verify `cargo --version`, `rustc --version`, and `cargo nextest --version` inside the plain Nix shell.
- [ ] Verify `./scripts/check-code.sh` inside the plain Nix shell.
- [ ] Verify the sandboxed wrapper can run `cargo build --workspace`.
- [ ] Verify the sandboxed wrapper can run `./scripts/check-code.sh` on Linux.
- [ ] Verify that caches persist across separate shell sessions.

# How should the work be verified?

Verification should include:

- `nix develop --command bash --noprofile --norc -lc 'cargo --version && rustc --version'`
- the repository-owned dev shell wrapper running `cargo build --workspace`
- the repository-owned dev shell wrapper running `./scripts/check-code.sh`
- the sandboxed wrapper running at least one build command and one repository check command on Linux
- confirmation that `CARGO_HOME`, `CARGO_INSTALL_ROOT`, `CARGO_TARGET_DIR`, and any relevant Rust cache paths resolve into the dedicated persistent cache root
- confirmation that a second shell session reuses the same cache root instead of starting cold each time
- confirmation that interactive entry does not print the user's normal shell welcome output

If the sandboxed wrapper cannot reasonably run on non-Linux hosts, that should be documented instead of hidden behind vague language.

# What assumptions should remain explicit?

This plan assumes:

- using Nix flakes is acceptable for the repository
- Linux is the only platform where the stronger `bubblewrap` sandbox will be supported initially
- a plain Nix dev shell should still work on supported non-Linux hosts even if the sandbox wrapper does not
- `bash` is an acceptable default interactive shell for the first version
- a dedicated persistent cache root is preferable to both disposable repo-local caches and the default global Cargo paths
- networking does not need to be fully blocked in the first version unless implementation work shows that it is cheap and reliable to do so

# What risks and open questions matter most?

The main risks are:

- duplicating too much logic between the new sandbox wrapper and `scripts/run-codex-sandbox.sh`
- creating a shell that is technically isolated but too annoying for contributors to actually use
- mounting too much of the host filesystem and calling it a sandbox anyway
- breaking workflows on macOS by overfitting the implementation to Linux-only assumptions
- introducing slow first-run behavior from Nix without documenting that tradeoff

The main open questions are:

- whether the repo should keep both Flox and Nix long-term or migrate to one preferred path
- whether the sandbox wrapper should allow network access by default
- whether the Nix shell should become the default `dev-shell.sh` behavior or live behind a separate entrypoint first
- whether shared `bubblewrap` assembly should be extracted into a common helper script

# What is intentionally out of scope for the first slice?

The first slice should not try to do all of the following:

- replace the devcontainer
- replace the Codex sandbox immediately
- provide a cross-platform sandbox implementation outside Linux
- enforce a fully hermetic network sandbox
- redesign CI around Nix

Those may become sensible follow-up work.
They are not required to deliver a useful "self-contained developer shell" first version.
