# `intrinsics/simd` audit

Module: `core/intrinsics/simd.vr` (~10 intrinsics) — SIMD vector element
ops: `simd_extract` / `simd_insert` / `simd_shuffle` and the horizontal
reductions (`simd_reduce_{add,mul,min,max,and,or,xor}`).

## Coverage decision: AUDIT-ONLY (no `*_test.vr`)

Every intrinsic is generic over a SIMD VECTOR type `V` (`f32x4`, `i32x8`,
…) and operates on an EXISTING vector value.  The module declares NO
vector constructor (`splat` / `from_array` / vector literal), so a
conformance test cannot build a `V` to feed these ops from `.vr` alone —
the vector must come from the vector-type machinery, which is exercised at
the tensor/kernel layer, not here.

Moreover, constructing and element-typing a generic `V`/`T` at a call site
routes through VBC-GENERIC-INSTANTIATION-1 (task #3): the interpreter
compiles one dynamic body per generic and the instantiation type is
invisible at the `@intrinsic` arm — the same blocker that makes
`transmute<Float, UInt64>` an identity.  Until #3 lands, a value-level
SIMD test would exercise the blocker, not the SIMD op.

## Contract notes (pinned by inspection)

* Reductions fold a lane vector to a scalar `T`; `min`/`max` are
  IEEE-aware for float lanes (NaN handling follows the float-intrinsic
  `minnum`/`maxnum` contract).
* `simd_shuffle` mask `M` is a compile-time lane-index vector.

## Crate-side drift surfaces

* AOT lowering: LLVM `shufflevector` / `extractelement` / `insertelement`
  + vector-reduce intrinsics (`llvm.vector.reduce.*`).  The MLIR path
  routes vector ops through the `vector` dialect (mlir-jit feature).

## Action items

* Value-level SIMD conformance UNBLOCKS once VBC-GENERIC-INSTANTIATION-1
  (task #3) lands + a `simd_splat`/`from_array` constructor is added.
  Tracked under task #3 / #5.
* Meanwhile the AOT vector path is covered by `tests/jit_cpu_equivalence.rs`
  (JIT-vs-scalar bit-equivalence) at the crate level.
