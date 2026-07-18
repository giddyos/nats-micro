#!/usr/bin/env bash
set -euo pipefail

run() {
  echo "+ $*"
  "$@"
}

run cargo fmt --all -- --check
run cargo clippy --workspace --all-features --all-targets -- -D warnings
run cargo test --workspace --exclude nats-server
run cargo test --workspace --all-features --exclude nats-server
run cargo check -p nats-micro --no-default-features
run cargo check -p nats-micro --no-default-features --features macros,json
run cargo check -p nats-micro --no-default-features --features macros,client,json
run cargo check -p nats-micro --no-default-features --features macros,client,json,protobuf
run cargo check -p nats-micro --no-default-features --features macros,client,json,encryption
run cargo check -p nats-micro --all-features
run cargo check --workspace --release --all-features
run cargo test -p nats-micro --doc --all-features
run cargo check -p nats-micro --examples --all-features

if [[ "${NATS_MICRO_REQUIRE_NATS_SERVER:-0}" == "1" ]]; then
  run bash scripts/runtime-check.sh
fi
