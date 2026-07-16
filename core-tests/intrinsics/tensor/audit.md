# intrinsics/tensor — suite audit

**Status (2026-07-16, T0193/T0176/T0140):** first real suite. The
previous audit deferred to the crate-level JIT-vs-scalar equivalence
gate ("a .vr suite would duplicate it") — the 2026-07-16 probe
DISPROVED that rationale: the JIT gate exercises the MLIR kernels,
not the intrinsic WIRE, and the wire was broken for most of the
surface. Of ~70 wrappers in `core/intrinsics/tensor.vr`, only the
~12 `*FromArgs` arms matched the registry emitter's operand shape.

## Defect classes closed (see T0193 for the full matrix)

1. **Wire-shape divergence** — the registry emitter packs
   `[dst][mode?][arg-regs...]`; ~30 interpreter arms read inline
   f64/varint/u8 immediates (Arange parsed register bytes as three
   f64s; Identity parsed a register index as its size; Softmax's
   axis byte was a register index; TENSOR_BROADCAST pointed at the
   DISTRIBUTED collective-broadcast arm). Multi-output arms
   (SVD/QR/LU/Eig/Topk) read 2–4 dst registers the emitter never
   allocated.
2. **Envelope-authoritative advance** — the dispatcher previously
   trusted each arm's reads for stream position; any arity drift
   desynced the bytecode stream (the GPU-memset SIGSEGV mechanism,
   T0177). `handle_tensor_extended` now repositions pc from the
   operand-byte-count envelope after every arm.
3. **Dtype-blind writes** — fill/from_slice/set_scalar wrote through
   `data_ptr_f64_mut()` (null for every dtype except F64): F32/int
   tensors silently stayed zero. `TensorHandle::{set_element_f64,
   fill_f64}` are the dtype-converting write twins of
   `get_element_f64`.
4. **Axis-ignoring kernels** — softmax/argmax ignored `axis`
   (`_axis`); now lane-walking implementations + log-softmax (LSE)
   + argmax index tensors + randn/randint kernels.
5. **T0140** — TokenizerDecode/FormatValue/GenerateRequestId
   discarded results (`nil`, "would need module mutation") though
   `alloc_string_value` exists; now heap strings.
6. **WithMode wire** — mode byte moved AFTER dst so `operands[0]`
   is uniformly dst (AOT read the mode byte as the destination
   register for RANDN/RANDINT/LOG_SOFTMAX).

## Tier contract

- `unit_test.vr` — core surface whose Tier-1 IR bodies EXIST
  (new/fill/from_slice/get/set/binop/unop/matmul/reduce/reshape/
  transpose/softmax/clone/contiguous). Interp green; AOT 1/20 —
  the bodies are value-SHALLOW (from_slice's IR writes one element,
  reduce/softmax ignore axis): T0179/T0201 staging. The
  reduce_all double-vs-i64 signature abort that killed every AOT
  compile is fixed (mode 1 routes through verum_tensor_reduce
  with axis −1).
- `integration_test.vr` — factories/indexing/decomp/einsum/conv/norm:
  green on `--interp`; at Tier-1 each op is a LOUD `verum_panic`
  stub ("no Tier-1 lowering yet", tensor_ir.rs `emit_panic_stub`)
  until the T0179 epic lands real IR bodies (was: declared-no-body
  externs → link-fail/const-zero class).
- `regression_test.vr` — minimal pins of the T0176/T0193 probe
  lines, both-tier for the core pins.

## Single-output contracts (documented in tensor.vr)

QR → Q, SVD → singular values, LU → U, EIG/EIGH → eigenvalues,
TOPK → values, SPLIT/SPLIT_AT → List of handles. Full multi-output
surface: T0179.

## Known residue

- Handle display: `f"{t}"` renders garbage type names ("Release") —
  Box<TensorHandle> has no ObjectHeader; needs the heap-object
  handle design (T0179 stage; also fixes the leak — handles are
  never freed today).
- `intrinsics/codegen.rs` `IntrinsicCodegen` is a ZERO-caller
  parallel strategy executor (duplicate-impl class).
- AOT dtype breadth beyond F64 fill lanes: T0183.
- MLIR JIT lowering path (`intrinsics/lowering.rs`) not yet
  differentially covered against the interpreter; the crate-level
  jit_cpu_equivalence gate remains the kernel-level authority.
