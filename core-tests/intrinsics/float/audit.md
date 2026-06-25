# `intrinsics/float` audit

Module: `core/intrinsics/float.vr` (~373 LOC) — IEEE-754 float intrinsics that
map to LLVM / libm: elementary functions, rounding, fused multiply-add,
sign/comparison, trig, hyperbolic, classification, IEEE bits, and special
values.  ~150 public functions (generic + `_f32`/`_f64`).

Tests: `unit_test.vr` (API surface, exact-representable points),
`property_test.vr` (sign/min-max/rounding/pow/fma laws), `integration_test.vr`
(geometry / Horner / clamp), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

VBC floats are IEEE **f64** at runtime (Float32 widens to f64); the generic
forms are f64-natural and `_f32`/`_f64` carry their width.  Most elementary /
trig / rounding functions dispatch via `MathExtendedOpcode` (≈2ns).  Tests pin
exact-representable points (`sqrt(4)=2`, `sin(0)=0`, `log(1)=0`, …) so equality
holds without tolerance.

## Tier summary

* **Interp: 85/85 GREEN** (7 `@ignore` for the §3 open defects).
* **AOT: 70/85** — 15 failures are a pre-existing AOT-codegen cluster
  (`FLOAT-AOT-LIBM-1`, task #21): `fneg`/`fms` (the `PolyNeg` AOT path doesn't
  negate floats) and the libm-backed `hypot`/`cbrt`/`expm1`/`log1p`/`powi`
  (no/incorrect AOT lowering).  Not this branch's wiring — revealed by the new
  tests.

## 1. What is verified GREEN (interp)

* **Elementary** — sqrt, cbrt, exp, exp2, expm1, log, log1p, log2, log10, pow,
  powi, hypot.
* **Rounding** — floor, ceil, round, **trunc** (after the collision fix §2).
* **Fused** — fma, **fms** (after §2).
* **Sign/compare** — copysign, fabs, **fneg** (§2), minnum, maxnum, fmod.
* **Trig** — sin, cos, tan, asin, acos, atan, atan2.
* **Hyperbolic** — sinh, cosh, tanh.
* **Classification** — is_nan, is_inf, is_finite, is_normal, is_infinite.
* **Special values** — infinity, nan, **epsilon / min_positive / max_float**
  (§2); IEEE bits f64/f32 (shared with `conversion`).

## 2. Defects FIXED on this branch

### FLOAT-TRUNC-COLLISION-1 — `trunc` returned its input unchanged

`float.trunc<T>` and `conversion.itrunc` both lowered to `@intrinsic("trunc")`.
That bare name resolves to the **integer**-truncation entry (a value-preserving
`Mov` at i64/f64 width), so float `trunc(2.9)` returned `2.9` instead of `2.0`.
(Pre-`conversion`-branch the shared alias pointed at a non-existent name → both
returned `nil`; the collision predates this work.)

**Fix** (`core/intrinsics/float.vr`): route float `trunc<T>` to its dedicated
`@intrinsic("trunc_f64")` (round-toward-zero, `MathSubOpcode::TruncF64`),
leaving the bare `"trunc"` to the integer truncation.

### FLOAT-FNEG-1 — `fneg` (and `fms`) returned nil / garbage

`fneg<T>` lowered to `@intrinsic("fneg")`, which had no registry entry → `nil`;
and `fms = fma(a, b, -c)` negates its third operand through a local `fneg`
helper, so it produced garbage.

**Fix** (`intrinsics/mod.rs::lookup_intrinsic`): alias `"fneg" => "neg"` — the
polymorphic negate (`ArithSubOpcode::PolyNeg`) dispatches int-vs-float by
operand type, so it negates floats correctly.  Closes `fneg` and `fms`.

### FLOAT-CONSTS-1 — `epsilon`/`min_positive`/`max_float` returned nil

These used `@intrinsic("f64_epsilon")` / `f64_min_positive` / `f64_max`, none of
which were registered → `nil`.

**Fix** (`core/intrinsics/float.vr`): return the plain IEEE-754 double literals
(`2.220446049250313e-16`, `2.2250738585072014e-308`, `1.7976931348623157e308`)
— no intrinsic needed; they const-fold.

## 3. Defects OPEN (pinned `@ignore`)

### FLOAT-ROUNDMODES-1 — `roundeven` / `rint` / `nearbyint` → nil  (task #18)

Round-half-to-even family; `@intrinsic` names unregistered.  No round-half-even
`MathSubOpcode` exists (`round_f64`/`RoundF64` is ties-**away**-from-zero, a
different rule).  Needs a `roundeven`/`rint`/`nearbyint` `MathSubOpcode`
(`llvm.roundeven`/`rint`/`nearbyint`) + interp + LLVM + registry.

### FLOAT-MINMAX-1 — `minimum` / `maximum` → nil  (task #19)

IEEE 754-2019 `minimum`/`maximum` (NaN-returns-NaN, signed-zero-aware) —
distinct from the working `minnum`/`maxnum` (NaN-returns-the-other).  No
opcode; needs dedicated `MathSubOpcode` + interp + LLVM + registry.

### FLOAT-CLASSIFY-1 — `is_subnormal` / `is_sign_negative` / `is_sign_positive` → nil  (task #20)

`@intrinsic` names unregistered.  Implementable via bit ops on `f64_to_bits`
(sign bit = bit 63; subnormal = biased-exponent 0 with non-zero mantissa) now
that `f64_to_bits` works — but the generic `<T>`-over-f64 typing of `f64_to_bits`
in a `.vr` body needs care — or via dedicated classify `MathSubOpcode`s.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.math.*` (libm, linalg, stats) | the entire elementary/trig/rounding surface. |
| `core.text.numeric` (float fmt/parse) | classification + `copysign` + bits + rounding. |
| `core.time` | `floor`/`round` for sub-second arithmetic. |
| graphics / geometry | `hypot`/`sqrt`/`fma` (vector length, Horner eval). |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic` — the float-math
  generic→`_f64` alias map (`sqrt`→`sqrt_f64`, …) and the new `fneg`→`neg`.
* `crates/verum_vbc/src/intrinsics/registry.rs` — the `MathExtendedOpcode`
  entries (`sqrt_f64`, `trunc_f64`, `minnum_f64`, …).
* `crates/verum_vbc/src/instruction.rs` — `MathSubOpcode` enum (the Rounding /
  Special / Binary-float bands); new round-modes / minimum-maximum / classify
  ops land here + interp + LLVM.
* AOT parity: `crates/verum_codegen/src/llvm/instruction.rs` MathExtended
  lowering.

## 6. Action items

**Landed this branch**
* FLOAT-TRUNC-COLLISION-1 — float `trunc` → `trunc_f64`.
* FLOAT-FNEG-1 — `fneg` → `neg` (PolyNeg); closes `fms`.
* FLOAT-CONSTS-1 — `epsilon`/`min_positive`/`max_float` as literals.
* Full float test suite (unit/property/integration/regression).

**Deferred (tracked)**
* FLOAT-ROUNDMODES-1 (#18), FLOAT-MINMAX-1 (#19), FLOAT-CLASSIFY-1 (#20) —
  interp-level nil (need new opcodes).
* FLOAT-AOT-LIBM-1 (#21) — AOT-only: `fneg`/`fms` (PolyNeg float arm) +
  `hypot`/`cbrt`/`expm1`/`log1p`/`powi` (libm AOT lowering).  Interp green.
