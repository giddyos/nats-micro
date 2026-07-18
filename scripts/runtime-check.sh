#!/usr/bin/env bash
set -euo pipefail

if ! command -v nats-server >/dev/null 2>&1; then
  echo "scripts/runtime-check.sh requires the nats-server binary on PATH." >&2
  exit 1
fi

NATS_MICRO_REQUIRE_NATS_SERVER=1 cargo test -p nats-micro --all-features
