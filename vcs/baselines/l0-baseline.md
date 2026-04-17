# VCS L0 Baseline (2026-04-17)

Captured on `main @ 12cd8dd` (before Phase 1.1 interpreter TLS init fix).

## Compile-time-only run

`vtest run --level L0 --parallel 4 --compile-time-only`

| Metric | Value |
|---|---|
| Total specs | 2918 |
| Passed | 2870 (98.4%) |
| Failed | 48 (1.6%) |
| Skipped | 0 |
| Duration | ~41 min wall clock |

**Goal for Phase 0.2**: L0 `100%` on both interpreter and AOT paths.
**Current gap**: 48 specs (1.6%) — mix of missing stdlib methods (e.g.
`Tensor.from_slice`), parser grammar gaps, and individual VBC features
surfaced during the migration.

## Full L0 run (with AOT differential)

`make test-l0 PARALLEL=4` — aborted at SIGABRT / SIGSEGV during
`vtest-diff-aot`. Two distinct blockers surfaced:

1. **`fat_ref.into_pointer_value()` panic** in
   `verum_codegen/src/llvm/instruction.rs:12456` at SliceGet /
   SliceGetUnchecked / SliceSubslice / SliceSplitAt — resolved by
   commit 04de418 (route through `as_ptr` helper).
2. **Residual SEGFAULT** further into the run (around spec ~1000).
   Cause not yet isolated; most likely another `.into_pointer_value()`
   site (57 remaining in that file) or the LLVM pass-pipeline stability
   issue tracked as Phase 1.3.

## Failure patterns observed (compile-time-only, 48 specs)

Representative categories visible in the test log:

- `Tensor.from_slice`, tensor indexing with `[[0, 0]]` — stdlib tensor
  methods not yet wired (no method found).
- Multi-dimensional subscript syntax `t[[0, 0]]` — parser accepts but
  downstream infers wrong type.
- Interrupt / MMIO — platform-specific pattern gaps.

The 48-failure list is not yet individualized (`--compile-time-only`
with `--format summary` does not emit per-spec FAIL lines). Phase 1.4
regression sweep will enumerate and triage.

## How to re-run

```bash
# Full L0 (requires 1.1–1.3 fixes landed for completion):
cd vcs && make test-l0 PARALLEL=4

# Compile-time-only (faster, skips runtime / AOT — baseline captured here):
cargo build -p vtest --release && \
  ./target/release/vtest run --level L0 --parallel 4 \
    --format summary --compile-time-only vcs/specs/L0-critical
```
