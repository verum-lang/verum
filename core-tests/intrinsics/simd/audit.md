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
identity, element-wise ops are the scalar op.

**The store family is the documented exception.**  `StoreAligned` /
`StoreUnaligned` / `MaskedStore` / `Scatter` have no scalar fallback at
all and ABORT on both tiers (T0112).  There is no width at which they
could be right: the wire erases both `T` and `N`, so writing the one
register value would leave `N-1` lanes stale and, for `T` narrower than
the 8-byte register, clobber the neighbouring element.  Until then a
refusal is the only non-corrupting answer — and it is the reason the
memory-op sub-suite still cannot be written against a round trip.

The suite pins:

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
  (verum_codegen/llvm/instruction.rs).  It is the ONLY `SimdExtended`
  lowering: `llvm/simd.rs` defines a typed vector API (`SimdLowering`)
  that is re-exported from `llvm/mod.rs` and called from NOWHERE, so no
  path ever produces an LLVM vector type.
* T0112 (A10): the silent no-op stores are CLOSED — all four store
  sub-ops now abort on both tiers (interp
  `simd_store_unimplemented`; AOT `emit_runtime_abort`), pinned by
  `simd_store_never_silently_drops_tests` (verum_vbc) and
  `simd_store_never_lowers_to_nothing` (verum_codegen).  The 30+ scalar
  value arms remain the true-vector upgrade umbrella; this suite's
  regression pins are the contract witnesses that must flip with it.

## 5. Coverage decisions

* Memory ops (load/store/masked/gather/scatter) are NOT suite-driven:
  a raw-pointer surface with no round trip to assert.  Stores now abort
  rather than drop (T0112), so there is nothing for a store to write
  and nothing for a load to read back; pinning "writes are dropped" as
  a contract would have blessed a defect, and pinning "stores abort"
  belongs with the crate-side pins, not here.  They join the suite when
  real vector lowering lands.
* **Loads are the remaining silent-wrong sibling, unfixed.**  Both
  tiers answer `LoadAligned` / `LoadUnaligned` / `MaskedLoad` /
  `Gather` with `dst = ptr` — the ADDRESS as data, never a
  dereference (interp `handlers/simd_extended.rs`, AOT's shared
  passthrough arm).  It is the T0184 fabricated-data class, not the
  dropped-write class, and it has a live stdlib caller:
  `core/simd/bytes.vr:81` (`find_byte` → `Vec16b.load_unaligned`), so
  making it loud is a behaviour change with real blast radius and needs
  its own decision.
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

**Landed (T0112, 2026-07-27)** — the four SIMD store sub-ops abort on
both tiers instead of reporting success for a write that never
happened.  The acceptance floor "never silently drop writes" is met;
true vector lowering is NOT part of it.

**Deferred** — true vector lowering (wire the element type and lane
count first, then the fallback pins flip tier-coherently); the
fabricated-address load family (§5); memory-op sub-suite after both;
typed `Vec<T,N>` conformance under `core-tests/simd/`.
