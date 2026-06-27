# `intrinsics/control` audit

Module: `core/intrinsics/control.vr` (~176 LOC) — control-flow intrinsics that
map to LLVM/CPU primitives: termination (`trap`, `unreachable`), debug
(`debugtrap`, `nop`), branch hints (`assume`, `likely`, `unlikely`, `expect`),
prefetch (`prefetch_read`, `prefetch_write`), panic/abort (`panic`, `abort`,
`debug_assert`, `unreachable_unchecked`, `panic_impl`), exception handling
(`catch_unwind` + the `IntrinsicSourceLocation` / `IntrinsicPanicInfo` records),
and randomness (`random_float`, `random_u64`).

Tests: `unit_test.vr` (ADT records + per-intrinsic surface), `property_test.vr`
(branch-hint identity laws + random invariants), `integration_test.vr` (hints
in realistic control flow), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

The branch-hint intrinsics are **semantically transparent**: `likely(c)`,
`unlikely(c)` and `expect(v, hint)` may steer the optimiser but MUST return
their primary operand unchanged — `likely`/`unlikely` are literally
`expect(c, true)` / `expect(c, false)`, and `expect<T>(v, hint)` returns `v`.
The second operand is an advisory hint only and may never influence the result.
This is the value contract the conformance suite pins.

The **terminating** intrinsics (`trap`, `unreachable`, `panic`, `abort`,
`unreachable_unchecked`, `panic_impl`) have return type `Never` — they diverge
and therefore cannot be value-tested inside a passing run; they are covered
only structurally (the module compiles and the `Never`-typed signatures keep
post-call code dead). `assume`/`unreachable`/`unreachable_unchecked` carry
UB-if-violated contracts and are exercised only in their sound form
(`assume(true)`).

## Tier summary

* **Interp: GREEN** once `CONTROL-EXPECT-NIL` is fixed (see §2). Branch hints
  (identity), `nop`/`assume(true)`/`debug_assert(true)` (callable no-ops), and
  `random_float` (unit interval) / `random_u64` (varies) all pass.
* **AOT:** branch hints lower to a `Mov` (the identity sequence) and the
  randomness primitives to their runtime calls — expected to match interp;
  validated alongside the rest of the intrinsics suite.

## 1. What is verified GREEN

* **branch hints** — `likely`/`unlikely` return their condition; `expect<T>`
  returns its value across Int / Bool / Text and is invariant under the hint
  operand; nested `expect` collapses to the original value.
* **no-ops** — `nop()`, `assume(true)`, `debug_assert(true, …)` are callable and
  return normally; interspersing `nop` does not perturb a computation.
* **randomness** — `random_float()` ∈ `[0.0, 1.0)` over many draws;
  `random_u64()` is non-constant.
* **integration** — `likely` as a loop-continue hint, `unlikely` as a
  rare-branch hint, and `expect` as a value hint all leave the computed result
  identical to the un-hinted form.
* **ADT records** — `IntrinsicSourceLocation` / `IntrinsicPanicInfo`
  construction + field independence (`unit_test.vr`).

## 2. Defects FIXED on this branch (data-only)

### CONTROL-EXPECT-NIL — `likely` / `unlikely` / `expect` returned `nil`

All three lower to `@intrinsic("expect", value, hint)`. The `"expect"` name had
**no registry entry**, so `lookup_intrinsic` returned `None` → codegen emitted
`LoadNil` → every branch-hint call evaluated to `nil` instead of passing its
operand through. (Same class as `CONV-INTWIDTH-1`.)

**Fix** (`crates/verum_vbc/src/intrinsics/registry.rs`): register `"expect"`
(category `Control`, `param_count 2`) with
`CodegenStrategy::InlineSequence(InlineSequenceId::Bitcast)` — the
`Bitcast`/`Sext` sequence emits `Mov dst, args[0]`, i.e. the identity of the
first operand while ignoring the extra hint operand, which is exactly
`expect`'s contract. Works on both tiers (the `Mov` is interpreter- and
LLVM-native).

## 3. Defects OPEN / not value-testable

* **Terminating intrinsics** (`trap`/`unreachable`/`panic`/`abort`/
  `unreachable_unchecked`/`panic_impl`) — diverge (`Never`); not value-tested
  here. A negative-path harness (expect-abort) belongs to a dedicated
  termination suite.
* **`catch_unwind(f)`** — exception-handling surface; behaviour is
  runtime-dependent (unwinding support) and is not yet pinned here.
* **`prefetch_read`/`prefetch_write`** — raw-pointer + side-effect-only hints;
  exercising them needs a valid backing pointer (same raw-pointer surface as
  `MEM-RAWPTR-HARNESS-1`).
* **`debugtrap()`** — a breakpoint hint; calling it may trap under some
  backends, so it is left to the termination suite rather than the value suite.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.base.panic` | `panic` / `panic_impl` / `unreachable` for diagnostics. |
| hot-path stdlib (collections, iterators) | `likely`/`unlikely`/`expect` branch hints. |
| `core.test` / assertions | `debug_assert` for debug-mode checks. |
| allocator / unsafe code | `assume` for optimiser invariants; `prefetch_*`. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the `expect` entry (+ any
  future `trap`/`nop`/`abort` strategy rows).
* `crates/verum_vbc/src/codegen/expressions.rs::emit_intrinsic_inline_sequence`
  — the `Bitcast`/`Sext` identity-`Mov` arm that `expect` reuses.
* interp control handlers + `crates/verum_codegen/src/llvm/` — per-tier
  semantics for `trap`/`unreachable`/`assume`/`prefetch`/`random_*`.

## 6. Action items

**Landed this branch**
* CONTROL-EXPECT-NIL — register `expect` → identity-Mov (fixes likely/unlikely/
  expect on both tiers).
* Full control conformance suite (unit/property/integration/regression + audit)
  over the value-producing + no-op surface.

**Deferred (tracked)**
* Termination-path suite for the `Never`-typed intrinsics (expect-abort harness).
* `catch_unwind` + `prefetch_*` once the unwinding / raw-pointer harnesses land.
