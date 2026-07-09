#!/usr/bin/env bash
set -euo pipefail

allows_direct_thiserror() {
  case "$1" in
    raw-thiserror-direct-dep|existing-thiserror-service-error)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

for manifest in nats-micro/tests/fixtures/*/Cargo.toml; do
  fixture="$(basename "$(dirname "$manifest")")"

  if grep -Eq '(^[[:space:]]*async-nats[[:space:]]*=|package[[:space:]]*=[[:space:]]*"async-nats")' "$manifest"; then
    echo "fixture $fixture must not directly depend on async-nats" >&2
    exit 1
  fi

  if ! allows_direct_thiserror "$fixture" \
    && grep -Eq '(^[[:space:]]*thiserror[[:space:]]*=|package[[:space:]]*=[[:space:]]*"thiserror")' "$manifest"; then
    echo "fixture $fixture must not directly depend on thiserror" >&2
    exit 1
  fi
done
