#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-features --lib --bins -- -D warnings
cargo test --workspace --all-features --exclude nats-server --lib --bins
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
  cargo clippy -p nats-micro --all-features --test "$test" -- -D warnings
  cargo test -p nats-micro --all-features --test "$test"
done
cargo check -p nats-micro --no-default-features
cargo check -p nats-micro --no-default-features --features client
cargo check -p nats-micro --no-default-features --features encryption
cargo check --workspace --release --all-features
