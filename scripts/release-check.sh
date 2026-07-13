#!/usr/bin/env bash
set -euo pipefail

bash scripts/check.sh

cargo publish --dry-run -p nats-micro-shared
cargo publish --dry-run -p nats-server

# Cargo resolves versioned internal dependencies through crates.io during
# publish dry-runs. For a first publish, leave this unset until prerequisite
# crates have been published, then rerun with NATS_MICRO_FULL_PUBLISH_DRY_RUN=1.
if [[ "${NATS_MICRO_FULL_PUBLISH_DRY_RUN:-0}" == "1" ]]; then
  cargo publish --dry-run -p nats-micro-macros
  cargo publish --dry-run -p nats-micro
fi
