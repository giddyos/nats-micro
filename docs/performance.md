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
cargo rustc -p nats-micro --release --lib -- --emit=asm
rg 'subject_matches' target/release/deps/nats_micro-*.s
cargo bloat -p nats-micro --release --example minimal \
  --no-default-features --features macros,client,json
cargo bloat -p nats-micro --release --example minimal --all-features
```

Record both `cargo bloat` totals when accepting a release. The
non-encryption core is the production-size baseline; encryption and N-API are
explicit feature costs. `cargo asm` is also useful when its parser supports the
active Rust toolchain and object format; direct `--emit=asm` remains the
toolchain-native fallback.

## Miri

The borrowed request and local router do not use unsafe code. Miri still checks
their lifetime-sensitive tests:

```bash
rustup +nightly component add miri
cargo +nightly miri setup
cargo +nightly miri test -p nats-micro --lib --no-default-features
cargo +nightly miri test -p nats-micro --test miri_local_router --features test-util
```

The trybuild test `static_borrowed_request_spawn` separately proves that a
borrowed request cannot escape into a spawned `'static` task.

## Fuzzing

The standalone `fuzz/` workspace covers subject matching and service error
decoding:

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run --fuzz-dir fuzz subject_matching
cargo +nightly fuzz run --fuzz-dir fuzz error_decoding
```

Before release, run each target with a time limit appropriate to CI or the
release workstation, for example `-- -max_total_time=300`.

## Validation record

On 2026-07-18, the allocation target passed all five assertions, the full
all-feature suite passed, and `cargo check --manifest-path fuzz/Cargo.toml`
compiled both fuzz targets. Criterion `--quick` measured the historical v1 raw
dispatch at 240.20–242.90 ns and the finalized v2 raw borrowed dispatch at
46.378–46.455 ns on the same host, a conservative 5.1x speedup. A full
Criterion sample measured the direct-`BytesMut` in-memory generated-client
round trip at 591.06–594.82 ns, versus 584.00–586.94 ns from the clean Phase 7
baseline worktree. That is below the 5% regression budget. These are comparison
evidence, not absolute cross-machine thresholds.
The managed single-server live generated-client round trip measured
88.518–90.012 µs.

`cargo llvm-lines` reported 60,886 LLVM IR lines across 1,956 function copies.
For the minimal production example, `cargo bloat` reported a 6.4 MiB binary
with 2.8 MiB of `.text` for `macros,client,json`, and a 6.5 MiB binary with
2.9 MiB of `.text` for all features. Direct assembly inspection of
`subject_matches` showed a split/compare loop with `memcmp` for literal
segments and no allocator calls. `cargo-asm` 0.1.16 could not parse a
current-Rust Mach-O symbol alias, so the equivalent `rustc --emit=asm`
artifact was used.

Miri on nightly passed the no-default library suite and the generated-client
local-router target. Tokio integration tests that construct an I/O-enabled
runtime cannot execute under Miri on macOS because Miri does not implement the
`kqueue` foreign call; no result for those tests is inferred from that tooling
boundary. Both fuzz targets completed 10-second nightly/libFuzzer smoke runs
without a crash (461,217 subject-matching executions and 404,401
error-decoding executions).
