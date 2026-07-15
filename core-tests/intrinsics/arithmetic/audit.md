# `intrinsics/arithmetic` audit

Module: `core/intrinsics/arithmetic.vr` (~388 LOC) — arithmetic intrinsics
that map directly to LLVM / CPU instructions: basic, checked, overflowing,
wrapping, saturating, comparison, wide/extended, and integer utilities.

Tests: `unit_test.vr` (API surface), `property_test.vr` (ring/order laws),
`integration_test.vr` (cross-stdlib), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

VBC integer Values are **untyped i64 at runtime** — the NaN-boxed `Int` tag
carries no sub-width information.  Therefore **bit-width is a static-only
concept**: it must be baked into the bytecode at codegen time, it cannot be
recovered by the interpreter from the operand value.

Consequence: the **generic** `wrapping_*` / `saturating_*` / `checked_*`
intrinsics are **64-bit-natural by design**.  Narrow-width semantics live in
the dedicated type-specific intrinsics (`wrapping_add_u8`, `saturating_add_u16`,
…) whose registry entry encodes `(width, signed)` via the `WrappingOpcode` /
`SaturatingOpcode` codegen strategies.  Tests respect this split: generic
forms tested at `Int` (i64), narrow forms via the type-specific intrinsics.

## 1. What is verified GREEN (interp, 106 live tests)

* **Basic** — add, sub, mul, div (trunc-toward-zero), rem (sign-of-dividend),
  neg, abs, abs_signed, signum.
* **Checked (Int width)** — checked_add/sub/mul/div return `None` on
  overflow / div-by-zero; checked_neg / checked_abs return `Some` for the
  non-`MIN` case.
* **Checked u64** — checked_add_u64 / checked_sub_u64 / checked_mul_u64 with
  unsigned overflow detection.
* **Overflow tuple** — add_overflow / sub_overflow / mul_overflow AND the
  `overflowing_add/sub/mul` aliases added this branch (§2).
* **Wrapping (Int width)** — wrapping_add/sub/mul/neg/shl/shr (modular at 2^64).
* **Wrapping type-specific** — _u8 / _i8 / _u32 width-correct truncation.
* **Saturating (Int width)** — saturating_mul/neg/abs clamp at i64 bounds.
* **Saturating type-specific** — _u8 / _u16 / _u32 / _i32 clamp at native bound.
* **min, max, clamp** (the `Poly*` ArithExtended forms).
* **Utilities** — ilog2 (floor).
* **Algebraic laws** — add/mul commutativity, associativity, identities;
  sub = add∘neg; neg involution; abs/signum laws; wrapping ring laws;
  saturating bounded-by-MAX/MIN.

## 2. Defects FIXED on this branch (source-level)

### ARITH-CHECKEDNEG-WIDTH-1 — `checked_neg`/`checked_abs` emitted no width

`checked_neg` / `checked_abs` use `ArithExtendedOpcode(CheckedNeg/CheckedAbs)`.
The interpreter handler
(`interpreter/dispatch_table/handlers/arith_extended.rs::CheckedNeg/CheckedAbs`)
reads `dst, src, width:u8, signed:u8`.  But codegen's
`emit_intrinsic_arith_extended` (`codegen/expressions.rs`) had **no arm** for
these sub-opcodes — they fell to the generic `_` fallback that emitted only
`[dst, src]`.  The interpreter then read the *next instruction's* bytes as
`width`/`signed`, yielding a garbage width, so `checked_neg(Int.MIN)` returned
`Some` instead of `None`.

**Fix**: explicit `CheckedNeg | CheckedAbs` arm emitting `width=64, signed=1`
(the documented generic-Int contract). **Validated** for the AOT path and for a
direct `@intrinsic("checked_neg", …)` / fresh user-defined generic wrapper
under interp. **Still red under interp via the *stdlib* wrapper** because that
call resolves to the precompiled archive body — see §2.5.  Pinned (`@ignore`)
by `regression_checked_neg_i64_min_is_none` / `…_abs_…`.

### ARITH-OVERFLOWING-ALIAS-1 — `overflowing_add/sub/mul` had no registry entry

`core/intrinsics/arithmetic.vr` documents `overflowing_add` et al. as
returning the same `(result, overflow_flag)` tuple as `add_overflow`, but the
registry had **no entry**, so `lookup_intrinsic` returned None and
`compile_intrinsic_call` emitted `LoadNil` (→ `nil`).  **Fix**: alias registry
entries to the identical `InlineSequence(Overflowing*)` strategy.  **GREEN**
(works because these resolve/inline at the user call site via the fresh
binary).  Pinned by `regression_overflowing_*` (now un-ignorable once the
remaining `overflowing_neg/shl/shr` land — see §3.1).

### 2.5 — wrapper defects: (A)+(C) FIXED, (B) open (task #4 / #3)

**UPDATE (2026-06-21): defects (A) and (C) below are CLOSED.** Direct
`VERUM_TRACE_PC` proved they were the *same* bug — a bare-name collision with
`core.math.checked` — not "operand framing"/"stale archive". `core/math/checked.vr`
exports a large bare-name surface (`checked_*`, `saturating_*`, `wrapping_*`,
`widening_mul`) that collides with `core/intrinsics/arithmetic.vr`. Resolution
ranked candidates by argument type, so a typed `Int` arg (e.g. `Int.MAX`) made
the unmounted concrete `core.math.checked.saturating_add(Int64,Int64)` win over
the mounted generic intrinsics fn — **despite the explicit mount**.

**FIX (commit 051a0e74d):** `CodegenContext.explicit_mount_names` +
`type_aware_lookup` mount-preference — an explicit `mount X.{name}` now owns the
bare slot, beating arg-type overload. Plus `SaturatingNeg`/`SaturatingAbs` width
arm in `emit_intrinsic_arith_extended` (the mount fix exposed they were
`LoadNil`-masked by the collision). `checked_neg/abs(Int.MIN)→None` and
`saturating_add/sub/neg/abs` clamp correctly. Un-ignored.

Remaining open below: only **(B)** and **(D)**.

#### Historical breakdown (for reference)

The `@ignore`d wrappers were initially assumed to be one "stale archive" bug.
Direct archive-body inspection (`verum_vbc` `dump_arch_fn` helper) +
`VERUM_TRACE_CALLS` proved that hypothesis WRONG — the archive bodies are mostly
correct.  The failures split into **three independent root causes**:

**(A) Bare-name collision — `checked_neg`, `checked_abs`.**
Despite `mount core.intrinsics.arithmetic.{checked_neg}`, the call resolves to
`core.math.checked.checked_neg(a: Int64) -> CheckedResult<Int64>`
(`func_id 13693`), which returns `CheckedResult.Overflow` — **not** `Maybe.None`
— so `is None` is false. The explicit mount does not win over a same-named
function elsewhere in stdlib. (The intrinsics `checked_neg` archive body is in
fact correct — `ArithExtended{CheckedNeg, [1,0,64,1]}` — i.e. the
ARITH-CHECKEDNEG-WIDTH-1 fix *did* land; it is simply never reached.)

**(B) Lenient `LoadNil` stubs — `eq`, `is_power_of_two`, `checked_rem`.**
Archived body is `LoadNil; Mov; Ret`. `checked_rem` and `is_power_of_two` have
no registry entry (`lookup_intrinsic → None → LoadNil`) — fixable with registry
entries (cf. the `overflowing_*` aliases). `eq` HAS a registry entry
(`DirectOpcode(EqI)`) yet is still stubbed → a precompiler / `IntrinsicCodegen`
gap for Bool-returning generic `DirectOpcode` wrappers.

**(C) Correct body, wrong runtime result — `saturating_add`/`saturating_sub`.**
Archived body is correct (`ArithExtended{SaturatingAdd, [2,0,1,64,1]}`,
width=64 signed=1) yet the **called** wrapper returns `nil`, while a **direct**
`@intrinsic("saturating_add", …)` with the byte-identical instruction returns
`Int.MAX`. `checked_add` (sub_op 0, 3 operands, no width) works; `saturating_add`
(sub_op 48, 5 operands incl. width/signed) does not → suspect `ArithExtended`
operand-length framing / width-byte handling when executed as a called archive
body vs a freshly-emitted instruction.

**Controls (all three):** a direct `@intrinsic(...)` works; a fresh user-defined
`fn f<T>(x:T){@intrinsic(...)}` works; only the stdlib wrapper path fails.

Each is a separate fundamental fix; all affected tests are pinned `@ignore`
referencing task #4.

### 2.6 INTRINSIC-NESTED-CALL-DISPATCH-1 (task #5, OPEN) — codegen

`mul(*a, add(*b, *c))` over iterator-derefed `Int`s returns `*a + *b + *c` —
the **outer `mul` is computed as `add`** (the inner intrinsic's identity leaks
to the outer call). `a=b=c=-1000` → `-3000` instead of `2_000_000`; the
non-nested `add(mul(a,b), mul(a,c))` is correct. A single-literal
`mul(1000, add(1000,1000))` const-folds correctly, so only the runtime-
dispatched nested-call-with-deref path is affected. Pinned `@ignore`:
`property_test.vr::law_mul_distributes_over_add`.

### Fixed this branch (additional)

* **Comparison wrappers `eq/ne/lt/le/gt/ge`** — `emit_intrinsic_direct_opcode`
  (`codegen/expressions.rs`) had no arm for the `EqI/NeI/LtI/LeI/GtI/GeI`
  DirectOpcodes, so they fell to `_ => LoadNil` and the stdlib wrappers compiled
  to nil-returning stubs. Added `CmpI`-emitting arms. **GREEN** (also un-blocked
  `assert_eq`-driven property tests, e.g. `law_mul_distributes` for positive
  operands, `law_min_max_coherent`).

## 3. Defects OPEN (ranked) — ARITH-MISSING-INTRINSICS-1

### 3.0 STATUS UPDATE 2026-07-15 — ARITH-PURE-BODY-1 landed; pins now blocked by ARCHIVE-GENERIC-BODY-NIL-1

Fourteen of the missing intrinsics (3.3–3.7 below + `overflowing_neg/shl/
shr`, `checked_rem`, `is_power_of_two`) no longer NEED registry entries:
they received PURE VERUM BODIES in `core/intrinsics/arithmetic.vr`
(textbook 32-bit-split `widening_mul`, sign-bias unsigned-carry
`carrying_add`/`borrowing_sub`, `clz`-based `leading_sign_bits`/
`checked_next_power_of_two`, division-loop `ilog10`, guard-based
`checked_shl/shr/rem`, `saturating_div`, `overflowing_neg/shl/shr` over the
wrapping family).  LLVM folds these shapes to `umulh`/`adcs` at Tier-1; the
interpreter executes them directly — no new opcodes, no width channel, no
VBC-GENERIC-INSTANTIATION dependency.

A LOCALLY-COMPILED twin of each body is proven correct
(`probe_genbody.vr`, `probe_local_vs_baked.vr`); the BAKED copies currently
execute to nil / mis-tagged Maybe — ARCHIVE-GENERIC-BODY-NIL-1 (session
task #22, same root family as INTRINSIC-GENERIC-WRAPPER-ARCHIVE-1 §0 and
the 0x2000_00xx stub-id class).  The suite pins carry that reason now and
flip live with #22; `probe_arith14.vr` (scratchpad) is the full
14-function acceptance battery.

The original class analysis (registry-gap fix shapes) is kept below for
3.1/3.2, which remain registry-level (their interp sub-opcodes exist).

| # | Intrinsics | Note / fix shape |
|---|---|---|
| 3.1 | `overflowing_add/sub/mul/neg/shl/shr` | interp already implements `OverflowingAddI/SubI/MulI`; registry is **missing the `overflowing_*` name→sub-opcode entries** (only `add_overflow` etc. are wired). Cheapest high-value fix: add registry entries aliasing to the existing sub-opcodes; add `OverflowingNeg/Shl/Shr` arms. |
| 3.2 | `wrapping_div`, `wrapping_rem`, `wrapping_abs`, `wrapping_next_power_of_two` | no registry entry; need sub-opcode + width-aware interp + LLVM lowering. `wrapping_div`/`rem` must NOT panic on `T.MIN / -1` (wrap to `T.MIN` / `0`). |
| 3.3 | `widening_mul`, `widening_mul_signed` | full-width 2-result multiply (lo, hi); needs tuple-returning sub-opcode (mirror `Overflowing*` tuple alloc). |
| 3.4 | `carrying_add`, `borrowing_sub` | 3-operand `(a,b,carry)→(res,carry_out)`; bignum primitive. |
| 3.5 | `checked_shl`, `checked_shr`, `checked_next_power_of_two` | `checked_shl/shr` → `None` when shift ≥ bit width; `checked_next_power_of_two` → `None` on overflow. |
| 3.6 | `ilog10`, `leading_sign_bits` | integer utilities; `ilog2` already works — mirror its InlineSequence. |
| 3.7 | `saturating_div` | clamp form of div. |

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.base.primitives` | mounts the bulk of this module to back `Int`/`UInt*` operator methods (`add`, `checked_add`, `wrapping_*`, `saturating_*`, `min`/`max`/`clamp`). |
| hashers (`core.collections.*`, nanoid/snowflake) | `wrapping_add` / `wrapping_mul` for rolling/mixing hashes. |
| numeric accumulators | `saturating_add` for bounded counters; `checked_*` for fallible totals. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the strategy table; generic
  vs `*_uN`/`*_iN` width split lives here. Adding a missing intrinsic = a row
  here + a dispatch arm.
* `crates/verum_vbc/src/codegen/expressions.rs::emit_intrinsic_arith_extended`
  — width/signed byte emission per sub-opcode (the ARITH-CHECKEDNEG-WIDTH-1
  site). Any new ArithExtended sub-opcode that the interpreter reads width for
  MUST get an arm here, not the `_` fallback.
* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/arith_extended.rs`
  + `arith_helpers.rs` — interpreter semantics; `type_bounds(width, signed)` is
  the canonical bound source.
* AOT parity: every interp arm needs the matching LLVM lowering in
  `crates/verum_codegen` (verify generic + type-specific forms agree across
  tiers; cross-tier divergence is a kernel incident).

## 6. Action items

**Landed this branch (source-level)**
* ARITH-CHECKEDNEG-WIDTH-1 fix (codegen width emission for CheckedNeg/CheckedAbs)
  — correct; AOT + direct-/fresh-wrapper validated; interp-via-stdlib-wrapper
  gated by #4.
* ARITH-OVERFLOWING-ALIAS-1 — `overflowing_add/sub/mul` registry aliases (GREEN).
* Full arithmetic test suite (unit/property/integration/regression):
  **106 live tests GREEN under interp**, 42 `@ignore` pins across the open
  defects below.

**Deferred — CRITICAL (tracked: INTRINSIC-GENERIC-WRAPPER-ARCHIVE-1, task #4)**
* The precompile→archive→monomorphization pipeline produces unreliable generic
  intrinsic free-function wrappers (§2.5). Fixing this un-blocks the bulk of the
  `@ignore` pins (eq/ne/lt/le/gt/ge, is_power_of_two, checked_rem,
  saturating_add/sub binary, checked_neg/abs(MIN), mul/add distribution).

**Deferred (tracked: ARITH-MISSING-INTRINSICS-1, task #3)**
* §3.1 `overflowing_neg/shl/shr` (need new sub-opcodes/InlineSequences).
* §3.2–3.7 missing wrapping_div/rem/abs, widening_mul, carrying_add,
  borrowing_sub, checked_shl/shr, checked_next_power_of_two, ilog10,
  leading_sign_bits, saturating_div — registry + interp + LLVM, cross-tier.

**Deferred — AOT cross-tier validation**
* This branch validated `--interp` only (per project norm — see INVENTORY's
  "partial under --interp" rows). AOT exhibits broad `MakeVariantTyped` /
  signature-mismatch warnings; full `--aot` arithmetic sweep is a follow-up.
