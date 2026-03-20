# What problem does this plan solve?

`nao` currently has no repository-owned release process.
Publishing the workspace crates and shipping a GitHub release would require manual, error-prone steps across Cargo manifests, tags, and GitHub Actions.

This plan describes how to add a minimal but sane release workflow that:

- publishes all crates to crates.io in the right order
- creates and pushes a release tag for the released version
- builds and uploads a Linux release artifact when that tag appears on GitHub

# What is the current status?

The repository already has:

- a Rust workspace with multiple publishable crates
- a CI workflow that builds and tests on GitHub Actions
- per-crate package versions currently set to `0.1.0`

The repository does not yet have:

- a release script
- a tag-triggered release workflow
- a documented release source of truth
- versioned internal dependency declarations suitable for publishing to crates.io

The last point is the real blocker.
Several crates currently depend on sibling crates using `path = "../..."` without a matching `version = "..."`.
That is fine for local workspace builds and wrong for publishing.
If this is not fixed first, `cargo publish` will fail and any shiny `release.sh` will just be a more efficient way to hit a wall.

# What implementation approach should be used?

The first version should stay intentionally small and boring:

- use one checked-in `scripts/release.sh`
- use one tag-triggered GitHub Actions workflow
- release Linux artifacts only
- avoid introducing extra release tooling unless the repository pain justifies it later

The workflow should be split by responsibility:

- local script owns publish ordering, validation, and tag creation
- GitHub Actions owns binary build, packaging, and GitHub Release creation

That split keeps crates.io publishing tied to an explicit human release action while letting GitHub handle reproducible artifact creation once the release tag exists.

# What repository changes are required before publishing can work?

The workspace should be made publish-ready before scripting the release flow.

That should include:

- adding explicit version requirements to all internal crate dependencies that currently use only local `path` dependencies
- ensuring every crate intended for publication has consistent package metadata
- deciding whether all six crates should be published or whether any should be marked `publish = false`

Based on the current manifests, the likely publish set is:

- `nao-base`
- `nao-pal`
- `nao-recipe`
- `nao-engine`
- `nao-tui`
- `nao`

The likely publish order is:

1. `nao-base`
2. `nao-pal`
3. `nao-recipe`
4. `nao-engine`
5. `nao-tui`
6. `nao`

That order should be encoded explicitly instead of inferred at runtime from fragile shell parsing.

# What should `scripts/release.sh` do?

`scripts/release.sh` should be a guarded release entrypoint, not just a shell alias for `cargo publish`.

The script should:

1. read the release version from one canonical place
2. verify all publishable crates use that same version
3. verify internal dependency versions match the release version
4. run explicit pre-release checks before publishing anything
5. verify the git worktree is clean
6. run repository verification before publishing
7. publish crates in dependency order
8. wait for each newly published crate to become available before publishing dependents
9. create an annotated git tag for the release version
10. push the tag after all publishes succeed

The script should not push the tag before publishing finishes.
Otherwise GitHub may create a public release for a version whose crates failed halfway through publication, which is a messy self-own.

The pre-release checks should be explicit and opinionated.
At minimum, the script should verify:

- the current commit is the intended release target
- the git worktree is clean
- required tools are installed
- publishing authentication is available
- every publishable crate uses the shared release version
- internal dependency versions match the shared release version
- `cargo publish --dry-run` succeeds for each publishable crate in publish order
- `./scripts/check-code.sh` passes

# Where should the release version come from?

The release flow needs one source of truth for the version.
The repository should keep one shared version across all published crates and read it from the CLI crate manifest, then verify every other publishable crate matches it exactly.

This plan should explicitly avoid independent crate versioning.
That would complicate publishing, tagging, release notes, and artifact naming for very little gain right now.

If the repository wants an even cleaner source of truth later, it can migrate to workspace package versioning or a dedicated version bump workflow, but the shared-version rule should remain.

# How should crates.io publishing be made reliable?

Publishing Rust workspaces is annoying in one specific way: dependent crates may fail to publish immediately after their dependencies because the crates.io index and API are not always instantly consistent.

The release script should therefore include a retry or polling step between publishes.
The first version can stay simple:

- publish one crate at a time
- after each publish, poll `cargo search` or another lightweight registry check for the released version
- continue only once the dependency is visible

Without that wait, the release script will be flaky for no good reason.

# What should the GitHub release workflow do?

The GitHub workflow should trigger on version tags, likely `v*`.
When triggered, it should:

1. check out the tagged revision
2. install the Rust toolchain
3. build the release binary for Linux
4. package the binary into a versioned archive
5. generate a checksum file
6. create or update the GitHub Release for the tag
7. upload the archive and checksum as release assets

For the first version, targeting `ubuntu-latest` and building the `nao` CLI crate is enough.
Do not overengineer cross-compilation, signing, or multi-platform packaging yet unless you actually need them.

# What artifact format should be shipped first?

The simplest artifact is a tarball containing:

- the `nao` binary
- a small `README` or install note if needed

The archive name should include the version and target, for example:

`nao-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`

The workflow should also upload a checksum file such as:

`nao-v0.1.0-x86_64-unknown-linux-gnu.sha256`

Checksums are cheap and useful.
Skipping them would be lazy in the bad way.

# What secrets and permissions are needed?

The release setup should document and configure:

- a `CARGO_REGISTRY_TOKEN` secret for crates.io publishing from local environments or CI if that ever moves to GitHub
- GitHub workflow permissions for release creation, specifically `contents: write`

Because this plan keeps crates.io publishing local, the GitHub workflow does not need the crates.io token in the first version.

# What implementation order is recommended?

The recommended order is:

1. Make all publishable crates actually publishable by adding internal dependency versions and filling any missing package metadata.
2. Decide and document which crates are part of the public release set.
3. Add `scripts/release.sh` with explicit pre-release checks, ordered publish steps, registry wait logic, and tag creation after successful publishing.
4. Add a tag-triggered GitHub workflow that builds the Linux release binary and publishes release assets.
5. Document the release process in repository documentation.
6. Verify the workflow with dry-run and non-destructive checks before doing the first real release.

# What concrete work items should be tracked?

- [ ] Add version requirements to all internal path dependencies for publishable crates.
- [ ] Ensure each publishable crate has complete and consistent package metadata.
- [ ] Decide whether any workspace crates should be excluded from publication with `publish = false`.
- [ ] Add `scripts/release.sh`.
- [ ] Make `scripts/release.sh` fail fast on dirty worktrees, version mismatches, and missing required tools.
- [ ] Make `scripts/release.sh` run explicit pre-release checks before publishing anything.
- [ ] Make `scripts/release.sh` run `./scripts/check-code.sh` before publishing.
- [ ] Make `scripts/release.sh` verify `cargo publish --dry-run` succeeds for each publishable crate before real publishing starts.
- [ ] Make `scripts/release.sh` publish crates in explicit dependency order.
- [ ] Make `scripts/release.sh` wait for each published crate version to become available before continuing.
- [ ] Make `scripts/release.sh` create an annotated `v<version>` tag only after successful publication.
- [ ] Make `scripts/release.sh` push the created tag.
- [ ] Add a GitHub Actions workflow triggered by version tags.
- [ ] Build the Linux release binary in that workflow.
- [ ] Package the binary into a versioned archive.
- [ ] Generate and upload a checksum file.
- [ ] Create a GitHub Release and attach the release assets.
- [ ] Document the release procedure, prerequisites, and rollback limits.
- [ ] Verify the release script with `cargo publish --dry-run` for each publishable crate where possible.
- [ ] Verify the tag workflow on a non-release tag pattern or temporary test repository before the first real release.
- [ ] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- `cargo publish --dry-run -p <crate>` for each publishable crate in publish order
- confirmation that internal dependency versions resolve correctly in the packaged manifests
- a local dry run of `scripts/release.sh` with the destructive publish and push steps disabled or gated
- a test of the GitHub Actions workflow using a temporary tag or a workflow-dispatch variant before the first real release
- `./scripts/check-code.sh`

The first real release should be treated as part of verification too.
If the dry run path is too different from the real path, the plan is missing the point.

# What assumptions should remain explicit?

This plan assumes:

- all publishable crates will continue to share one version number
- Linux-only release artifacts are acceptable for the first iteration
- crates.io publication should remain a human-triggered local action for now
- the GitHub release should be created from a pushed `v<version>` tag
- Git tags do not need signing in the first version

# What risks and missing pieces matter most?

The main risks are:

- crates failing to publish because internal dependency versions are missing or inconsistent
- release tags being pushed for versions that were not fully published
- flaky dependent crate publishes caused by crates.io propagation delays
- shipping a GitHub artifact whose version does not match the published crate version

The main missing pieces beyond the two requested tasks are:

- release notes or changelog generation
- a version bump workflow
- rollback guidance, especially since crates.io publishes are effectively irreversible
- optional future hardening such as signed tags, artifact signing, provenance, or multi-platform builds

# Anything else is missing?

Yes, three things matter enough to include up front:

- publishability fixes in the Cargo manifests
- explicit pre-release checks in `scripts/release.sh` and documentation that says who bumps versions and when
- a rollback/recovery note for partial failures, because crates.io does not do take-backs

Everything else can wait.
Those three cannot.
