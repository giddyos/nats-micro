# Request dispatch baselines

The Criterion target in this directory keeps the v1 dynamic dispatcher and the
v2 static dispatcher comparable while the breaking rewrite proceeds. Allocation
tests live in `../tests/allocation_baseline.rs`.

## Recorded v1 baseline

Recorded on 2026-07-18 with an Apple M1 Max and `rustc 1.97.0`:

| Benchmark | Allocations | Median time | Approximate throughput |
|---|---:|---:|---:|
| `v1_dynamic_raw_static_response` | 6 | 225.64 ns | 4.43 million requests/s |
| `v2_static_raw_borrowed_response` | 0 | 40.19 ns | 24.9 million requests/s |

The allocation count starts after a raw frame is available and includes the v1
owned request/context preparation plus dynamic handler dispatch. It excludes
transport-internal `async-nats` work.

The first v2 measurement is approximately 5.6× faster than the recorded v1
baseline for this isolated dispatch class. It is a microbenchmark, not an
end-to-end NATS throughput claim.

Reproduce the timing sample with:

```console
cargo bench -p nats-micro --bench request_dispatch -- \
  --warm-up-time 1 --measurement-time 2 --sample-size 20
```
