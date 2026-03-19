#!/usr/bin/env bash

set -euo pipefail

rustc --version
cargo --version
codex --version

cargo fetch
