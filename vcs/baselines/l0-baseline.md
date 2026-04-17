# VCS L0 Baseline (2026-04-17)

Captured after this session's Phase 0/1/2/3 work landed on `main`.

## Compile-time-only (no AOT execution)

`vtest run --level L0 --parallel 4 --compile-time-only`

| Metric | Value |
|---|---|
| Total specs | 2918 |
| Passed | 2870 (98.4%) |
| Failed | 48 (1.6%) |
| Skipped | 0 |
| Duration | ~41 min wall clock |

Representative failure categories observable in the log:
- Missing `Tensor.from_slice` / multi-dimensional subscript method
- Parser accepts `t[[0, 0]]` but type inference disagrees
- Interrupt / MMIO platform-specific patterns not yet implemented
- (48-failure per-spec list not yet enumerated — runner emits only
  summary-format in this mode)

## Full L0 run (interpreter + AOT differential)

Previously `make test-l0 PARALLEL=4` SIGABRT'ed at spec ~400 inside
`vtest-diff-aot` on the AOT slice-op panic
(`fat_ref.into_pointer_value()` at
`verum_codegen/src/llvm/instruction.rs:12456`).

After commit 04de418 (route through `as_ptr` helper) + commit 7e12603
(interpreter TLS init) + commit faa31c3 (warning cleanup) + commit
c0a79b6 (CI -D warnings), `make test-l0` progresses to ~2100 specs
before the log ends on a SIGSEGV without a preceding Rust panic —
i.e., a silent LLVM / native-code crash during AOT codegen of the
test runner's spawned `verum build` subprocess.

**Reproduction**: single-spec compilation of
`vcs/specs/L0-critical/vbc/integration/002_algorithms.vr` and the
specs that follow it. The crash is non-deterministic and surfaces as
a `make: *** [test-l0] Segmentation fault: 11`.

**Probable cause** (Phase 1.3 remaining work): one of the 57
remaining `.into_pointer_value()` call sites in
`instruction.rs` has the same IntValue / PointerValue confusion, OR
the LLVM pass pipeline hits a stability issue under sustained module
compile load. Both are tractable given a stable reproducer; a
single-spec run usually does not reproduce, which points at an
accumulated-state or race condition bias.

**Workaround**: `--compile-time-only` (the 98.4% baseline above) runs
typecheck only and does not trigger the AOT crash.

## How to re-run

```bash
# Full L0 with differential AOT (currently SIGSEGVs around spec ~2100)
cd vcs && make test-l0 PARALLEL=4

# Compile-time-only (safe baseline capture)
cargo build -p vtest --release && \
  ./target/release/vtest run --level L0 --parallel 4 \
    --format summary --compile-time-only vcs/specs/L0-critical
```

## Goal

100% on both compile-time-only and full L0. Currently:
- Compile-time: 98.4% (48 spec failures — stdlib/feature gaps)
- Full: blocked by Phase 1.3 AOT stability residual
