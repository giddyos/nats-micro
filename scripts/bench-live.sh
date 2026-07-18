#!/usr/bin/env bash
set -euo pipefail

if ! command -v nats-server >/dev/null 2>&1; then
  echo "scripts/bench-live.sh requires nats-server on PATH" >&2
  exit 1
fi

port="${NATS_MICRO_BENCH_PORT:-4223}"
store="$(mktemp -d)"
nats-server -js -p "$port" -sd "$store" >"$store/server.log" 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  wait "$server_pid" 2>/dev/null || true
  rm -rf "$store"
}
trap cleanup EXIT

ready=0
for _ in {1..100}; do
  if (echo >"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    ready=1
    break
  fi
  sleep 0.05
done
if [[ "$ready" != "1" ]]; then
  cat "$store/server.log" >&2
  exit 1
fi

NATS_URL="nats://127.0.0.1:$port" \
  cargo bench -p nats-micro --all-features --bench request_dispatch -- \
  live_generated_client_round_trip
