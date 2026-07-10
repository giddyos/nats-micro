#!/usr/bin/env bash
set -euo pipefail

run() {
  echo "+ $*"
  "$@"
}

fixture_check() {
  local name="$1"
  shift
  run cargo check --manifest-path "nats-micro/tests/fixtures/${name}/Cargo.toml" "$@"
}

fixture_test() {
  local name="$1"
  shift
  run cargo test --manifest-path "nats-micro/tests/fixtures/${name}/Cargo.toml" "$@"
}

run cargo fmt --all -- --check
run cargo clippy --workspace --all-features --lib --bins -- -D warnings
run cargo test --workspace --all-features --exclude nats-server --lib --bins
for test in \
  client_errors \
  contract_json \
  encryption_dispatch \
  encryption_primitives \
  error_responses \
  extractors \
  prelude \
  ui
do
  run cargo clippy -p nats-micro --all-features --test "$test" -- -D warnings
  run cargo test -p nats-micro --all-features --test "$test"
done
run cargo check -p nats-micro --no-default-features
run cargo check -p nats-micro --no-default-features --features client
run cargo check -p nats-micro --no-default-features --features encryption
run cargo check -p nats-micro --no-default-features --features client,encryption
run cargo check -p nats-micro --all-features
run cargo check --workspace --release --all-features
run cargo test -p nats-micro --doc --all-features

run bash scripts/check-fixture-manifests.sh

fixture_test service-authoring-basic
fixture_test generated-client-app
fixture_check consumer-jetstream
fixture_check encryption-client
fixture_test renamed-dependency-full
fixture_test no-default-features-service
fixture_check napi-surface
fixture_test raw-thiserror-direct-dep
fixture_test existing-thiserror-service-error

if command -v nats-server >/dev/null 2>&1; then
  run cargo test -p nats-micro --all-features --test live_runtime
  fixture_test runtime-roundtrip
  fixture_test consumer-jetstream
else
  echo "nats-server not found; skipping runtime NATS fixture tests"
  if [[ "${NATS_MICRO_REQUIRE_NATS_SERVER:-0}" == "1" ]]; then
    echo "NATS_MICRO_REQUIRE_NATS_SERVER=1 set, failing because nats-server is unavailable" >&2
    exit 1
  fi
fi
