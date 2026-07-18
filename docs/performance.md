# Performance and safety verification

The runtime performance contract is structural first: borrowed frame data,
monomorphized endpoint calls, bounded in-task futures, and no dynamic state or
handler lookup. Run measurements on an otherwise idle machine with frequency
scaling and background load controlled.

## Allocation and Criterion checks

```bash
cargo test -p nats-micro --test allocation_baseline
cargo bench -p nats-micro --all-features --bench request_dispatch
bash scripts/bench-live.sh
```

`allocation_baseline` asserts zero framework allocations for the raw borrowed
request/static response class after frame receipt. It also checks request and
consumer worker sources for boxed handlers, per-message spawning, semaphores,
payload clones, and the removed dynamic primitives.

The Phase 1 v1 measurement for that class was six framework allocations per
dispatch; the v2 assertion is zero. The removed v1 Criterion implementation is
available at repository commit `1db33f9`. Use a separate worktree when a
throughput comparison is required:

```bash
git worktree add /tmp/nats-micro-v1 1db33f9
cargo bench -p nats-micro --all-features --bench request_dispatch
```

Keep the in-memory raw dispatch budget at least 3x faster than that measured v1
run, and keep generated client encode/decode within 5% of an accepted v2
baseline. Do not turn one machine's absolute nanoseconds into a repository
threshold.

## Code growth, assembly, and feature size

Install the tools once:

```bash
cargo install cargo-llvm-lines cargo-asm cargo-bloat
```

Reproducible inspections:

```bash
cargo llvm-lines -p nats-micro --lib --all-features
cargo asm -p nats-micro --lib 'nats_micro::subject::subject_matches'
cargo bloat -p nats-micro --release --no-default-features --features macros,client,json
cargo bloat -p nats-micro --release --all-features
```

Record both `cargo bloat` totals when accepting a release. The
non-encryption core is the production-size baseline; encryption and N-API are
explicit feature costs.

## Miri

The borrowed request and local router do not use unsafe code. Miri still checks
their lifetime-sensitive tests:

```bash
rustup +nightly component add miri
cargo +nightly miri setup
cargo +nightly miri test -p nats-micro --lib --no-default-features
cargo +nightly miri test -p nats-micro --test static_runtime
```

The trybuild test `static_borrowed_request_spawn` separately proves that a
borrowed request cannot escape into a spawned `'static` task.

## Fuzzing

The standalone `fuzz/` workspace covers subject matching and service error
decoding:

```bash
cargo install cargo-fuzz
cargo fuzz --fuzz-dir fuzz run subject_matching
cargo fuzz --fuzz-dir fuzz run error_decoding
```

Before release, run each target with a time limit appropriate to CI or the
release workstation, for example `-- -max_total_time=300`.

## Validation record

On 2026-07-18, the allocation target passed all five assertions, the full
all-feature suite passed, and `cargo check --manifest-path fuzz/Cargo.toml`
compiled both fuzz targets. Criterion `--quick` measured the historical v1 raw
dispatch at 240.20–242.90 ns and the finalized v2 raw borrowed dispatch at
46.493–46.555 ns on the same host, a conservative 5.1x speedup. The v2
in-memory generated-client round trip measured 576.87–582.29 ns. These are
comparison evidence, not absolute cross-machine thresholds.
The managed single-server live generated-client round trip measured
93.888–98.002 µs.

The validation host did not have
`cargo-llvm-lines`, `cargo-asm`, or `cargo-bloat` installed, and Miri was not
available for its stable Apple Silicon toolchain. The commands above are the
required follow-up probes on a release host with those tools installed; no
size, assembly, or Miri result is inferred from tool absence.
