#!/usr/bin/env bash
set -euo pipefail

bash scripts/check.sh

# Cargo resolves versioned internal dependencies through crates.io during
# publish dry-runs. For a first publish of a new version, publish each
# prerequisite crate in this order, then rerun this script before continuing.
cargo publish --dry-run -p nats-micro-shared
cargo publish --dry-run -p nats-micro-macros
cargo publish --dry-run -p nats-server
cargo publish --dry-run -p nats-micro
