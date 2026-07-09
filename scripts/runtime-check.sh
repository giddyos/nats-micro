#!/usr/bin/env bash
set -euo pipefail

if ! command -v nats-server >/dev/null 2>&1; then
  echo "scripts/runtime-check.sh requires the nats-server binary on PATH." >&2
  exit 1
fi

cargo test --workspace --all-features
