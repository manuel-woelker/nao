#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/.docker/docker-compose.yaml"

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: scripts/docker.sh [command...]

Runs the repository Rust container with `docker compose run --build --rm`.
Without a command, opens an interactive Bash shell.
EOF
  exit 0
fi

if [[ $# -eq 0 ]]; then
  exec docker compose -f "$COMPOSE_FILE" run --build --rm rust bash -l
fi

exec docker compose -f "$COMPOSE_FILE" run --build --rm rust "$@"
