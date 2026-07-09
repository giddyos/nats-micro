#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features
cargo check -p nats-micro --no-default-features
cargo check -p nats-micro --no-default-features --features client
cargo check -p nats-micro --no-default-features --features encryption
cargo check --workspace --release --all-features
