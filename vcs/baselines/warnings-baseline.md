# Compiler Warning Inventory (Phase 0.3 baseline)

Captured from `cargo build --workspace` on 2026-04-17 against commit 939562f
(CI restoration). Workspace has **25 genuine Rust warnings** across two crates,
plus build-system noise from vendored LLVM sources that is not in our code.

Phase 3 removes this file after the warning backlog is eliminated and
`cargo clippy --workspace --all-targets -- -D warnings` becomes a blocking CI
gate.

## Category A — `feature-incomplete-wire`

These warnings disappear naturally when Phase 2.2 (verify --solver), 2.1
(attributes in codegen) and the FFI integration tasks land. Do NOT
silence with `#[allow(dead_code)]` before those phases — the warning is a
useful reminder.

| File | Line | Warning | Resolving phase |
|---|---|---|---|
| crates/verum_smt/src/cvc5_backend.rs | 216 | `CVC5_KIND_AND` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 217 | `CVC5_KIND_OR` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 218 | `CVC5_KIND_NOT` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 219 | `CVC5_KIND_IMPLIES` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 220 | `CVC5_KIND_EQUAL` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 221 | `CVC5_KIND_LT` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 222 | `CVC5_KIND_LEQ` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 223 | `CVC5_KIND_GT` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 224 | `CVC5_KIND_GEQ` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 225 | `CVC5_KIND_ADD` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 226 | `CVC5_KIND_SUB` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 227 | `CVC5_KIND_MULT` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 228 | `CVC5_KIND_DIV` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 229 | `CVC5_KIND_MOD` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 230 | `CVC5_KIND_ITE` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 231 | `CVC5_KIND_FORALL` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 232 | `CVC5_KIND_EXISTS` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 233 | `CVC5_KIND_SELECT` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 234 | `CVC5_KIND_STORE` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 209 | `CVC5_KIND_AND` block preceding | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 380 | enum `Cvc5Result` never used | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 1580 | multiple fields never read | 2.2 |
| crates/verum_smt/src/cvc5_backend.rs | 1607 | field `raw`, field `tm` never read | 2.2 |
| crates/verum_vbc/src/codegen/mod.rs | 4852 | `get_current_ffi_platform` never used | 2.1/FFI |

## Category B — `intentional-allow` (documented dead)

| File | Line | Warning | Reason |
|---|---|---|---|
| crates/verum_vbc/src/interpreter/kernel/mod.rs | 143 | `MIN_GPU_SIZE` never used | Threshold constant reserved for GPU dispatch heuristic; kept as documented configuration. Phase 3 decides: either use it (cutoff in kernel dispatch) or add explicit `#[allow(dead_code)]` with rationale. |

## Category C — build-system noise (not our code)

C++ diagnostics from vendored LLVM/LLD and the macOS `ar` tool's `illegal
option -- D` error. These originate in `crates/llvm/verum_llvm_sys`,
`crates/llvm/verum_tblgen`, `crates/llvm/verum_mlir_sys`, and
`crates/cvc5-sys`, and are not actionable in our Rust code. They are
excluded from the CI warning gate via the crate filter.

## Process

To refresh this inventory after changes:
```bash
cargo build --workspace 2>&1 | tee target/warnings-full.txt
grep -E "^warning:" target/warnings-full.txt \
    | grep -vE "verum_llvm_sys@|verum_tblgen@|verum_mlir_sys@|cvc5-sys@|verum_compiler@|generated " \
    | sort -u
```
