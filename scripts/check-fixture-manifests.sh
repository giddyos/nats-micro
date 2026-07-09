#!/usr/bin/env bash
set -euo pipefail

for manifest in nats-micro/tests/fixtures/*/Cargo.toml; do
  fixture="$(basename "$(dirname "$manifest")")"

  if grep -Eq '^[[:space:]]*async-nats[[:space:]]*=' "$manifest"; then
    echo "fixture $fixture must not directly depend on async-nats" >&2
    exit 1
  fi

  if [[ "$fixture" != "raw-thiserror-direct-dep" ]] \
    && grep -Eq '^[[:space:]]*thiserror[[:space:]]*=' "$manifest"; then
    echo "fixture $fixture must not directly depend on thiserror" >&2
    exit 1
  fi
done
