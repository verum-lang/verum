# Red-Team Round 3 — Performance / DoS Guardrails

Tracks #174. Each `.vr` test in this directory pins one
performance-class invariant — typically a regression-sanity check
that the audit-sweep wins haven't been undone.

These are NOT micro-benchmarks (those live in
`crates/verum_compiler/benches/`). They're correctness tests that
exercise a perf-class hot path and assert the *bulk-copy primitive
contract* — e.g. `with_capacity(N) + push N times` MUST NOT trigger
a resize cascade.

## Coverage

| File | Vector pinned |
|---|---|
| `wire_frame_alloc_baseline.vr` | Vector 2.1 — wire-frame allocation reduction (audit-confirmed defense) |

## Pending vectors

Round-3 vectors that require harness infrastructure beyond a `.vr`
test:

- 1.1 / 1.2 / 1.3 compilation-time DoS — synthetic deep-generic /
  exponential-SMT / module-fan-out generators.
- 2.2 refinement caching — hot-loop with refinement-typed args
  benchmark.
- 2.3 / 2.4 task / channel scaling — multi-thread harness.
- 3.x cache-line / memory-bandwidth — pinned multi-thread test.
- 4.x bytecode pathological cases — synthetic bytecode generator.
- 5.x stdlib loading at scale — synthetic-module generator.

See `vcs/red-team/round-3-perf.md` for full vector inventory +
audit-confirmed defenses already pinned.
