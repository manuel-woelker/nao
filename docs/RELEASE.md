# What is this document for?

This document describes how to cut a `nao` release.
Use it when publishing crates to crates.io and shipping a GitHub release artifact.

# What does the release flow do?

The release flow has two parts:

- `./scripts/release.sh` publishes the crates to crates.io and pushes the release tag
- `.github/workflows/release.yml` builds the Linux binary and publishes the GitHub Release when that tag appears

The current release process uses:

- one shared version across all published crates
- a `v<version>` git tag such as `v0.1.0`
- Linux-only release artifacts for the first version

# What must be true before running the release script?

Before running `./scripts/release.sh`, make sure:

- you are on `main`
- the worktree is clean
- all publishable crates use the same version
- internal crate dependencies pin that same version
- crates.io authentication is available through `CARGO_REGISTRY_TOKEN` or `cargo login`

The script checks these conditions itself and fails fast if any of them are wrong.

# How should a release be prepared?

First, bump the shared crate version with:

```bash
./scripts/release.sh prepare 0.1.1
```

That command updates:

- the package version in every publishable crate manifest
- the internal dependency version pins between workspace crates

It does not commit anything for you.
That part should stay explicit so the version bump is reviewable and the release tag points at a real commit, not some weird local-only state.

After `prepare`, review the manifest changes and commit them on `main`.

# How should a release be created?

Run:

```bash
./scripts/release.sh publish
```

If you want to validate the release without publishing or tagging, run:

```bash
./scripts/release.sh publish --dry-run
```

The script will:

1. run pre-release checks
2. run `./scripts/check-code.sh`
3. run `cargo publish --dry-run` for each publishable crate
4. publish crates in dependency order
5. wait for each published crate version to appear on crates.io
6. create an annotated `v<version>` tag
7. push the tag to `origin`

The tag is created only after all publishes succeed.
That is deliberate.
Publishing first avoids creating a public GitHub release for a version whose crates only half-published.

The publish step also fails early if any crate already exists on crates.io at the selected shared version.
That is much better than discovering it halfway through dependency verification like an idiot.

# What crates are published and in what order?

The release script publishes these crates in this order:

1. `nao-base`
2. `nao-pal`
3. `nao-recipe`
4. `nao-engine`
5. `nao-tui`
6. `nao`

If that order changes, update both the manifests and the release script together.

# What does the GitHub workflow publish?

When a `v<version>` tag is pushed, GitHub Actions:

1. checks out the tagged revision
2. builds the `nao` release binary on Linux
3. packages the binary and `README.md` into a tarball
4. generates a SHA-256 checksum
5. creates or updates the GitHub Release
6. uploads the tarball and checksum as release assets

# What should be done if a release fails halfway through?

Crates.io publishes are effectively irreversible.
If part of the release succeeds and a later crate fails, do not try to pretend nothing happened.

Instead:

- stop and inspect which crates published successfully
- fix the root cause on `main`
- bump all published crate versions to a new shared version
- retry the release with the new version

Do not reuse the partially published version.
That way lies nonsense.
