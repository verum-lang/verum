# `intrinsics/tensor` audit

Module: `core/intrinsics/tensor.vr` (~72 intrinsics) — tensor compute:
elementwise, matmul, conv, reductions, broadcast, reshape, autodiff hooks.

## Coverage decision: AUDIT-ONLY here; behaviour covered at the crate level

Tensor intrinsics operate on `Tensor<T, Shape>` values whose construction,
layout, and lowering are the MLIR compute pipeline's domain (linalg/tensor
dialects → LLVM JIT, the `mlir-jit` default feature).  The canonical
conformance for this surface is the JIT-vs-scalar bit-equivalence gate in
`crates/verum_vbc/tests/jit_cpu_equivalence.rs` — a build invariant.  A
`.vr`-level tensor suite here would either duplicate that gate or, for the
value-level ops, hit the same generic-instantiation blocker as SIMD
(task #3).

## Contract notes

* Elementwise + reduction ops fold through the same kernel path the CPU
  fallback and the MLIR JIT share; ε-equivalence for float reductions is
  the pinned invariant.
* Autodiff hooks (`tensor_grad_*`) integrate with the GradBegin/GradEnd
  opcodes (0xEB-0xEF).

## Action items

* If a `.vr`-level smoke suite is wanted, add it AFTER task #3 unblocks
  generic-vector construction; until then the crate-level JIT equivalence
  test is the authoritative conformance surface.
