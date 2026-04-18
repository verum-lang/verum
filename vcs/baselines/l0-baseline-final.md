# L0 Sweep Final Baseline (2026-04-18)

## Coverage

9 of 10 L0 categories measured. Excluded: `vbc/` (subdir of 1500+
[run] specs that requires AOT execution; runs in 60+ minutes
wall-clock and is captured separately by `vcs/scripts/aot-stress.sh`).

| Category | Specs | Pass | Rate |
|---|---|---|---|
| builtin-syntax     |  44 |  44 | 100.0% |
| lexer              |  97 |  97 | 100.0% |
| parser             | 105 | 105 | 100.0% |
| types              |  77 |  77 | 100.0% |
| memory-safety      | 131 | 131 | 100.0% |
| mmio               |   6 |   4 |  66.7% |
| modules            |   3 |   3 | 100.0% |
| reference_system   | 124 | 118 |  95.2% |
| stdlib-runtime     |   8 |   8 | 100.0% |
| **Total**          | **595** | **587** | **98.7%** |

## Remaining 8 failures (root-caused)

### mmio: type-checker doesn't enforce `Register<T, Mode>` capability boundary

- `readonly_write_fail.vr` — `r: Register<UInt32, ReadOnly>; r.write(x)`
  should be a typecheck error; currently the impl block for
  `Register<T, ReadWrite>::write` is found regardless of the mode
  type parameter.
- `writeonly_read_fail.vr` — symmetric.

Fix requires: per-instantiation impl-block filtering during method
dispatch in `verum_types::infer`. Tracked.

### reference_system/cbgr: stdlib `static mut` persistence + Epoch monotonicity

The 6 failures (`cbgr_comprehensive`, `cbgr_overview`,
`heap_allocation_layout`, `tier_comprehensive`, `tier_fallback`,
`packed_epoch_caps`) all call `Epoch.current()` / `Epoch.advance()`
and assert `new_epoch > old_epoch`. After commit b0a9b44 the type
exists and the calls dispatch cleanly (no more `Stack overflow:
depth 1024 exceeds maximum 1024`); the runtime monotonicity
invariant doesn't yet hold inside the Tier 0 interpreter because
`static mut COUNTER = COUNTER + 1` writes don't propagate across
frames. Fix requires either:
- thread-local backing store with proper cell semantics, or
- a dedicated interpreter intrinsic for the global counter.

AOT honors static slots correctly; this is an interpreter-only gap.

## How to reproduce

```bash
cargo build --release --bin verum --bin vtest
cd vcs
target/release/vtest run --quiet \
  specs/L0-critical/builtin-syntax \
  specs/L0-critical/lexer \
  specs/L0-critical/parser \
  specs/L0-critical/types \
  specs/L0-critical/memory-safety \
  specs/L0-critical/mmio \
  specs/L0-critical/modules \
  specs/L0-critical/reference_system \
  specs/L0-critical/stdlib-runtime
```

## Delta from prior session baseline (vcs/baselines/l0-baseline.md)

That baseline reported 2870/2918 = 98.4% on compile-time-only L0
across all categories. This baseline is more focused (98.7% across
9 of 10 categories, including the [run] specs that the prior
baseline skipped via `--compile-time-only`). The vbc/ subdirectory
is excluded here for the wall-clock reasons noted above.
