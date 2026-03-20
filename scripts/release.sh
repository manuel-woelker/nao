#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_BRANCH="${NAO_RELEASE_BRANCH:-main}"
WAIT_ATTEMPTS="${NAO_RELEASE_WAIT_ATTEMPTS:-30}"
WAIT_SECONDS="${NAO_RELEASE_WAIT_SECONDS:-10}"
DRY_RUN=0

CRATE_DIRS=(
  "crates/base"
  "crates/pal"
  "crates/recipe"
  "crates/engine"
  "crates/tui"
  "crates/cli"
)

CRATE_PACKAGES=(
  "nao-base"
  "nao-pal"
  "nao-recipe"
  "nao-engine"
  "nao-tui"
  "nao"
)

usage() {
  cat <<'EOF'
Usage: ./scripts/release.sh [--dry-run]

Options:
  --dry-run    Run all pre-release checks and cargo publish dry runs without
               publishing crates, creating tags, or pushing anything.
  --help       Show this help.
EOF
}

log() {
  printf '==> %s\n' "$*"
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_tool() {
  local tool="$1"
  command -v "$tool" >/dev/null 2>&1 || fail "required tool not found: $tool"
}

manifest_value() {
  local manifest_path="$1"
  local key="$2"

  sed -nE "s/^${key}[[:space:]]*=[[:space:]]*\"([^\"]+)\"$/\\1/p" "$manifest_path" | head -n1
}

crate_version() {
  local crate_dir="$1"
  manifest_value "$ROOT_DIR/$crate_dir/Cargo.toml" "version"
}

release_version() {
  crate_version "crates/cli"
}

current_branch() {
  git -C "$ROOT_DIR" symbolic-ref --quiet --short HEAD || true
}

assert_clean_worktree() {
  if [[ -n "$(git -C "$ROOT_DIR" status --short)" ]]; then
    fail "git worktree is not clean"
  fi
}

assert_release_branch() {
  local branch
  branch="$(current_branch)"
  [[ -n "$branch" ]] || fail "release requires a checked out branch, not a detached HEAD"
  [[ "$branch" == "$RELEASE_BRANCH" ]] || fail "release must run from branch '$RELEASE_BRANCH' (current: '$branch')"
}

assert_publish_auth() {
  if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    return
  fi

  if [[ -f "${CARGO_HOME:-$HOME/.cargo}/credentials.toml" ]] || [[ -f "${CARGO_HOME:-$HOME/.cargo}/credentials" ]]; then
    return
  fi

  fail "crates.io credentials not found; set CARGO_REGISTRY_TOKEN or run cargo login"
}

assert_tag_absent() {
  local tag_name="$1"

  if git -C "$ROOT_DIR" rev-parse -q --verify "refs/tags/$tag_name" >/dev/null 2>&1; then
    fail "tag already exists locally: $tag_name"
  fi
}

assert_shared_version() {
  local expected_version="$1"
  local crate_dir

  for crate_dir in "${CRATE_DIRS[@]}"; do
    local actual_version
    actual_version="$(crate_version "$crate_dir")"
    [[ -n "$actual_version" ]] || fail "missing version in $crate_dir/Cargo.toml"
    [[ "$actual_version" == "$expected_version" ]] || fail "$crate_dir/Cargo.toml uses version $actual_version, expected $expected_version"
  done
}

assert_internal_dependency_versions() {
  local expected_version="$1"
  local manifest_path

  for crate_dir in "${CRATE_DIRS[@]}"; do
    manifest_path="$ROOT_DIR/$crate_dir/Cargo.toml"

    while IFS= read -r dependency_line; do
      local dependency_name
      dependency_name="$(printf '%s\n' "$dependency_line" | sed -nE 's/^([a-z0-9-]+)[[:space:]]*=.*/\1/p')"
      local dependency_version
      dependency_version="$(printf '%s\n' "$dependency_line" | sed -nE 's/.*version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p')"

      [[ -n "$dependency_version" ]] || fail "$manifest_path is missing a version for internal dependency $dependency_name"
      [[ "$dependency_version" == "$expected_version" ]] || fail "$manifest_path pins $dependency_name to $dependency_version, expected $expected_version"
    done < <(grep -E '^[a-z0-9-]+[[:space:]]*=.*path[[:space:]]*=' "$manifest_path" || true)
  done
}

run_repo_checks() {
  log "running repository checks"
  "$ROOT_DIR/scripts/check-code.sh"
}

run_publish_dry_runs() {
  local crate_dir
  local package_name
  local index

  for index in "${!CRATE_DIRS[@]}"; do
    crate_dir="${CRATE_DIRS[$index]}"
    package_name="${CRATE_PACKAGES[$index]}"
    log "running cargo publish --dry-run for $package_name"
    cargo publish \
      --manifest-path "$ROOT_DIR/$crate_dir/Cargo.toml" \
      --locked \
      --dry-run
  done
}

wait_for_crate_version() {
  local package_name="$1"
  local version="$2"
  local attempt
  local url="https://crates.io/api/v1/crates/$package_name/$version"

  for ((attempt = 1; attempt <= WAIT_ATTEMPTS; attempt += 1)); do
    if curl --fail --silent --show-error "$url" >/dev/null; then
      return
    fi

    sleep "$WAIT_SECONDS"
  done

  fail "timed out waiting for $package_name $version to become available on crates.io"
}

publish_crates() {
  local version="$1"
  local crate_dir
  local package_name
  local index

  for index in "${!CRATE_DIRS[@]}"; do
    crate_dir="${CRATE_DIRS[$index]}"
    package_name="${CRATE_PACKAGES[$index]}"
    log "publishing $package_name $version"
    cargo publish \
      --manifest-path "$ROOT_DIR/$crate_dir/Cargo.toml" \
      --locked

    log "waiting for $package_name $version to appear on crates.io"
    wait_for_crate_version "$package_name" "$version"
  done
}

create_and_push_tag() {
  local tag_name="$1"
  log "creating annotated tag $tag_name"
  git -C "$ROOT_DIR" tag -a "$tag_name" -m "Release $tag_name"

  log "pushing tag $tag_name"
  git -C "$ROOT_DIR" push origin "$tag_name"
}

run_pre_release_checks() {
  local version="$1"
  local tag_name="$2"

  log "running pre-release checks for version $version"
  require_tool git
  require_tool cargo
  require_tool curl
  require_tool sed
  require_tool grep

  assert_release_branch
  assert_clean_worktree
  assert_shared_version "$version"
  assert_internal_dependency_versions "$version"
  run_repo_checks
  assert_clean_worktree
  run_publish_dry_runs

  if [[ "$DRY_RUN" -eq 0 ]]; then
    assert_publish_auth
    assert_tag_absent "$tag_name"
  fi
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --dry-run)
        DRY_RUN=1
        ;;
      --help)
        usage
        exit 0
        ;;
      *)
        fail "unknown argument: $1"
        ;;
    esac
    shift
  done
}

main() {
  parse_args "$@"

  local version
  version="$(release_version)"
  [[ -n "$version" ]] || fail "failed to determine release version from crates/cli/Cargo.toml"

  local tag_name="v$version"

  run_pre_release_checks "$version" "$tag_name"

  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "dry run completed; skipping publish, tag creation, and push"
    return
  fi

  publish_crates "$version"
  create_and_push_tag "$tag_name"
  log "release $tag_name completed"
}

main "$@"
