# `intrinsics/gpu` audit

Module: `core/intrinsics/gpu.vr` (~48 intrinsics) — device management,
kernel launch, memory transfer, thread/block indexing, barriers.

## Coverage decision: CPU-EMULATION (unit + regression) + audit for the rest

The interpreter (Tier 0) and the AOT runtime (Tier 2) both implement the
host-side GPU ops as CPU fallbacks — no accelerator required — so the
device / memory / stream / event / enumeration surface IS testable on the
CI host:

* `unit_test.vr` — device get/set, malloc/free, memset+memcpy round-trip
  (status), stream create/query/destroy, event create/record/query/sync/
  destroy lifecycle, `enumerate_*` (non-negative handle).
* `regression_test.vr` — one `@test` per wire-shape defect (see T0177 below).

Assertions are restricted to the **tier-invariant** contract: status ops
return `0`, handles/enumerations are `>= 0`. We deliberately do NOT assert
value round-trips (e.g. `gpu_set_device(3)` then `gpu_get_device() == 3`):
the interpreter tracks GPU context state, but the AOT runtime stubs
(`emit_gpu_stub_body`, `crates/verum_codegen/src/llvm/tensor_ir.rs`) are
stateless and return `const_zero`. Asserting equality would be a cross-tier
divergence; stateful AOT emulation is a separate concern (see Action items).

### Still AUDIT-ONLY (device-gated)

The GPU thread-model intrinsics (`ThreadIdX`/`BlockIdX`/`SyncThreads`/…, the
`GpuSubOpcode` 0xA0–0xBF space) are only meaningful inside a launched kernel
and need a device harness the CI host cannot provide. Kernel launch on the
interpreter is a flat CPU-fallback stub (consumes `[dst][kernel_id][grid]
[block][shared_mem][stream][args]`, returns 0; no real dispatch). Device-
backed conformance belongs in a hardware-gated CI lane (`@cfg(gpu = "…")`),
deferred until that lane exists.

## Defect classes fixed (T0177 / GPU-OPERAND-SHAPE-1)

The `GpuExtended` (0xF8) carrier wire shape is `[dst][args...]` iff
`return_count > 0` (`emit_intrinsic_gpu_extended`,
`crates/verum_vbc/src/codegen/expressions.rs`); `core/intrinsics/gpu.vr` is
the signature authority (every fn returns `-> Int`). Two divergences closed:

* **GPU-OPERAND-SHAPE-1** — 31 `GPU_*` registry entries carried
  `return_count: 0` while `.vr` declares `-> Int`, so the emitter dropped the
  leading `dst` register on the wire. The interpreter's sequential reader then
  consumed one register too many (a garbage register index → out-of-bounds →
  SIGSEGV; reproduced with `gpu_malloc(64,0); gpu_memset(p,171,0)`).
  Fix: `crates/verum_vbc/src/intrinsics/registry.rs` (`return_count` 0→1, GPU
  section invariant comment) + interpreter arity splits in
  `crates/verum_vbc/src/interpreter/dispatch_table/handlers/gpu.rs`
  (`SyncStream`/`SyncDevice`, `PinMemory`/`UnpinMemory`,
  `Memcpy`/`MemcpyAsync`, `Memset`/`MemsetAsync`, and a flat `Launch` stub).
  The AOT lowering (`lower_gpu_extended`,
  `crates/verum_codegen/src/llvm/instruction.rs`) already parsed
  `[dst][args...]` via a bounds-checked `get_arg`, so it never crashed and
  needed no change — the registry flip realigns operand[0]=dst for it too.
* **GPU-RETURN-CONTRACT-1** — because of `return_count: 0`, those calls
  evaluated to unit (`()`) instead of the declared `Int` (probes printed
  `()`). The same registry flip restores the typed status return. Pinned by
  `regression_test.vr::test_gpu_return_contract_1_free_is_typed_int`.
* **Envelope-authoritative pc advance** (structural class-kill, mirrors the
  T0193 tensor-tree hardening): `handle_gpu_extended` now repositions pc to
  `operands_start + operand_byte_count` after every arm, so a future arity
  drift between an arm and the emitter degrades to a wrong VALUE instead of
  an instruction-stream desync (the SIGSEGV mechanism above). The T0192
  residue (`GetMemoryInfo` over-read, `EventCreateWithFlags` under-read, the
  PEER-trio `param_count` drift) is thereby defanged from latent-desync to
  wrong-value; T0192 still owns making those arities exact.

Wire authority is documented at the `// GPU` block in
`crates/verum_vbc/src/instruction.rs` and the GPU encode section of
`crates/verum_vbc/src/bytecode.rs`: the `GpuExtended` carrier is the sole
0xF8 wire format; the structured `Gpu*` named variants are vestigial (never
constructed by codegen, never produced by the decoder — structured 0xF8
decode was removed) and retained only for the AOT/MLIR lowering match arms.

## Action items

* Stateful AOT GPU emulation (device id, stream/event handles) so set→get
  round-trips agree across tiers — currently interpreter-only.
* Device-backed conformance in a hardware-gated CI lane (`@cfg(gpu = "…")`).
