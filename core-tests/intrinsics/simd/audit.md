# `intrinsics/simd` audit

Module: `core/intrinsics/simd.vr` — the RAW SIMD intrinsic layer (lane
access, element-wise arithmetic/compares/bitwise/shifts, reductions).

Suite (NEW 2026-07-16, replaces the audit-only decision): unit (34) +
property (17 law sweeps) + integration (7) + regression (8 pins).

## 1. What changed vs the old audit-only decision

The 2026-07-05 audit declared this module untestable for want of (a) a
vector constructor and (b) generic instantiation.  Both premises fell:

* **SIMD-SPLAT-UNDECLARED-1** — `simd_splat` had a registry row
  (`InlineSequence(SimdSplat)`) and BOTH tier implementations since day
  1; only the `.vr` declaration was missing.  Declared 2026-07-16
  (`core/intrinsics/simd.vr` §0), along with the rest of the
  registered-but-undeclared surface (arith/cmp/select/bitwise/shifts —
  Sections 5-7).
* **T0175 / INTRINSIC-RESOLVE-NONDET-1** — the real instantiation
  blocker was NOT a language gap: baked descriptors interned an opaque
  concrete param (`idx: UInt32` → `__opaque_type_N`) as a THIRD explicit
  generic var, and hashed-`Set` iteration randomized the scheme's var
  order — mounted `simd_extract<V, T>` typechecked nondeterministically
  (≈1/6 runs green).  Fixed by the ONE-authority scheme builder
  `build_metadata_function_scheme` (appearance-order vars; opaque
  existentials implicit).  Local generic fns were always fine — the
  defect was metadata-scheme birth, not inference.

## 2. Contract under test — tier-coherent SCALAR FALLBACK

Both tiers implement the raw layer as a scalar fallback (interp
`handlers/simd_extended.rs`; AOT `lower_simd_extended`): a "vector"
register carries ONE lane; splat/extract are identity, reductions are
identity, element-wise ops are the scalar op.  The suite pins:

* lane-count-INVARIANT laws in unit/property (survive future true
  vectors): splat∘extract=id, insert-read-back, per-lane arithmetic
  mirrors scalar arithmetic, comparison trichotomy, select routing,
  Boolean algebra, min/max bracketing;
* the lane-count-SENSITIVE fallback facts ONLY in regression pins
  (reduce_add/mul/xor of a splat == the lane), explicitly marked to
  flip when true multi-lane values land (T0112 + interp twin).

Float operands are exact binary fractions so `assert_eq` is bit-exact
across tiers.

## 3. Fixes landed with this suite (T0116)

* **SIMD-REDUCE-BITWISE-REGISTRY-1** — `simd_reduce_and/or/xor` had NO
  registry rows, so every call lowered to silent LoadNil, while the
  sub-ops (0x34-0x36), the interpreter handlers
  (`handlers/simd_extended.rs`) and the AOT scalar arms
  (`lower_simd_extended`, landed with the cmp wiring) ALL existed.
  Landed here: the 3 registry rows
  (`InlineSequence(SimdReduceAnd/Or/Xor)`) + MLIR `vector.reduction`
  and/or/xor legs + the library-call name-map arms.  Pinned by
  `simd_pin_reduce_{and,or}_wired`.
* Surface declarations (splat + Sections 5-7) as above —
  **SIMD-SPLAT-UNDECLARED-1**: the registry row and both tier
  implementations pre-existed; only the `.vr` declaration was missing.

## 4. Crate-side drift surfaces

* Emission: registry `InlineSequence(Simd*)` →
  `emit_intrinsic_library_call("verum_simd_*")` → **name-mapped back to
  `Instruction::SimdExtended` sub-ops** (expressions.rs ~35028) — the
  "library call" layer is an indirection, not an FFI symbol; adding a
  simd intrinsic requires BOTH the registry row and the name-map arm.
* AOT scalar fallback: `lower_simd_extended`
  (verum_codegen/llvm/instruction.rs ~25181); vectorized lowering for
  the typed `core.simd.Vec<T,N>` API lives in llvm/simd.rs.
* T0112 (A10): 30+ scalar arms + MaskedStore/Scatter silent no-op
  stores — the true-vector upgrade umbrella; this suite's regression
  pins are the contract witnesses that must flip with it.

## 5. Coverage decisions

* Memory ops (load/store/masked/gather/scatter) are NOT suite-driven:
  raw-pointer surface with silent-no-op AOT stores (T0112) — pinning
  "writes are dropped" as a contract would bless a defect. They join
  the suite when T0112 lands.
* `simd_shuffle`/`simd_cast`/`from_scalars` deferred: AOT groups them
  in a passthrough arm whose dst semantics differ from interp's
  (first-element vs passthrough) — needs the T0112 pass anyway.
* Mask types are Bool at this layer (scalar fallback); `Mask<N>`
  algebra belongs to the typed `core/simd` suite.

## 6. Action items

**Landed (T0116, 2026-07-18)** — surface declarations; reduce-trio
wiring; this suite.  (The T0175 scheme-builder fix in `verum_types`
that unblocked mounted-generic instantiation landed independently,
82569ce38.)

**Deferred (tracked)** — T0112 true vector lowering (+ interp twin so
the fallback pins flip tier-coherently); memory-op sub-suite after it;
typed `Vec<T,N>` conformance under `core-tests/simd/`.
