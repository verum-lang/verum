# Micro Benchmark Baseline (2026-04-18)

Captured on `Apple M3 Max, 128 GB, macOS aarch64` with the release
binary built from the current `main` (post commit 4837b4e).

## Top-level summary

| Metric | Value |
|---|---|
| Total benchmarks | 35 |
| Passed | 28 (80%) |
| Failed | 7 (20%) — pre-existing perf-target misses, not regressions |
| Total wall-clock | 549.86 ms |
| Mean ns/op | 2200 ns |
| Fastest | `cbgr/tier1_checked/direct_access` — 16.28 ns |
| Slowest | `allocation/large/10mb` — 67803.7 ns |

## What this baseline guards

Every commit that wants to claim "no perf regression" should be
checked with:

```bash
make -C vcs bench-compare
```

which runs the same suite and diffs against
`vcs/baselines/bench-baseline-micro.json`. Regressions > 10 % on
any benchmark fail the CI gate (per the original [D] criterion in
the production-readiness plan).

## How this baseline was captured

```bash
cargo build --release --bin vbench
target/release/vbench micro --format json --output vcs/baselines/bench-baseline-micro.json --quiet
```

(no `--warmup`/`--iterations` flags — vbench's micro defaults already
do 100 warmup + 1000 iterations per measurement.)

## Notes

- The 7 failing micros are existing performance-target misses, not
  regressions from this session's runtime/codegen fixes. Listed in
  the JSON's `results[]` array; addressing them is separately tracked.
- The macro suite (`vbench macro`) and the L4-performance specs
  (`vbench specs`) are not yet baselined — they require longer wall
  clock and a quieter machine. Tracked as follow-ups.
