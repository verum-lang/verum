# GPU & Tensor Capability Audit — 2026-07-16

Full-stack audit of Verum's GPU / tensor / SIMD computing capabilities:
stdlib surface → intrinsic registry → VBC opcodes → Tier-0 interpreter →
MLIR/LLVM backends → device runtimes → tests → docs.  Commissioned as the
grounding for the capability-rethink epic (pool task GPU-TENSOR-RETHINK-1);
every claim below carries a `file:line` anchor at commit `8f82d9f16`.

The one-paragraph verdict: **Verum already owns a real CPU tensor engine
and a real MLIR JIT — and a complete but disconnected autodiff engine —
behind a surface layer whose plumbing (registry rows, wire shapes, name
resolution) is where the defects live.**  The distance to "advanced GPU
and tensor computing" is not a rewrite; it is (1) wiring three existing
islands together, (2) making the plumbing honest, and (3) building the
device-codegen leg that today exists only as parse-level syntax and
README aspiration.

---

## 1. What is REAL today

### 1.1 CPU tensor engine (Tier-0) — real numeric library
`crates/verum_vbc/src/interpreter/tensor.rs` (7.6 KLOC) +
`interpreter/kernel/cpu.rs` (13 KLOC, 166 pub fns):

* Real heap storage: `TensorData` owns `NonNull<u8>` via `alloc_zeroed`,
  refcounted (`tensor.rs:231-316`); `TensorHandle` carries dtype/shape/
  strides (negative strides supported) (`tensor.rs:447-468`).
* 15 dtypes (`dtype.rs:36-104`: F64/F32/F16/BF16, I64..I8, U64..U8, Bool,
  Complex64/128); elementwise binop/unop + broadcasting cover ALL 15
  (`cpu.rs:214-363`, real `BroadcastIterator`).
* Real loop nests everywhere probed: matmul (`tensor.rs:2155-2236`),
  conv2d with groups/stride/padding/dilation/bias (`tensor.rs:4328-4519`),
  Householder QR, Jacobi SVD (`cpu.rs:6918-7172`), symmetric eig,
  Cholesky, lstsq, inverse, einsum, Padé matrix-exp.
* Zero `todo!()`/stub markers in the two engine files; gaps are
  **unsupported-dtype guards**, not fake code: heavy ops (matmul, conv2d,
  transpose, linalg, axis-reduce) are F32/F64-only; full-reduce covers 11
  dtypes; the cpu reduce kernel ignores its axis arg (axis-reduce lives in
  tensor.rs, F32/F64-only).

### 1.2 MLIR JIT backend — real, DEFAULT-ON, CPU-targeted
`interpreter/kernel/mlir_jit_backend.rs`: builds genuine LLVM-dialect MLIR
(8 kernel templates), verifies, runs real pass pipelines
(`linalg-to-loops → scf-to-cf → … → convert-to-llvm`), executes through
the MLIR C-API `ExecutionEngine` (invoke_packed), content-addressed disk
kernel cache, graceful `Option` degradation to the scalar CPU ladder.
Registered by default (`backend.rs:507-514`, `mlir-jit` in default
features). `verum_mlir_sys` links real `libMLIR*.a` from `llvm/install`
(21.1.8). **No gpu/nvvm/rocdl/spirv lowering exists** — the JIT targets
the host CPU. Equivalence vs the scalar ladder is pinned by
`tests/jit_cpu_equivalence.rs` (cpu.rs is the reference oracle).

### 1.3 Metal backend — real FFI, dead in default builds
`interpreter/kernel/metal.rs`: 40+ real MSL kernels, real
`new_library_with_source` compile, real `dispatch_thread_groups` +
`wait_until_completed` submission — gated
`#![cfg(all(target_os = "macos", feature = "metal"))]`; the `metal`
feature is NOT in defaults. **Stock builds perform zero GPU offload.**
No CUDA/ROCm/Vulkan backend exists anywhere in the workspace (a commented
placeholder in `backend.rs:498`).

### 1.4 stdlib ML/tensor surface — enormous and largely real
`core/math/` (from the module inventory):

* `tensor.vr` (3 KLOC): 13 public types, **290 public fns**, 74 distinct
  `TENSOR_*` opcodes — creation/elementwise/activations/losses/metrics/
  reductions/shape/indexing/linalg/conv/pool/init/schedules.
* `nn.vr` (1.6 KLOC): Linear/Embedding/Conv1d/2d/LayerNorm/RMSNorm/
  BatchNorm/Dropout/MultiHeadAttention/FeedForward/TransformerBlock/RoPE,
  SGD/AdamW + LR schedulers, loss fns.
* `autodiff.vr` (1.4 KLOC): JAX-style `vjp/jvp/grad/value_and_grad/
  jacobian/hessian/hvp/gnvp`, custom VJP, checkpointing (incl. optimal
  schedule), grad scoping, 24 `@vjp_rule` attributes.
* `linalg.vr` (2.2 KLOC): pure-Verum BLAS-1/2/3 + LU/QR/Cholesky/eig/SVD/
  Schur (opcode-free reference implementations).
* `gpu.vr` (1.8 KLOC): device registry/selection, buffers, pinned memory,
  streams, events, CUDA graphs, peer access, `LaunchConfig`+`Dim3`,
  `GPUKernel` protocol, profiling — 49 `GPU_*` opcodes.
* First-class grammar: `tensor<2,3>f32{...}` literal (`verum.ebnf:1541`,
  `ExprKind::TensorLiteral`), `Tensor` AST type with layout.
* AOT has a real `passes/tensor_fusion.rs` (Matmul/Softmax/LayerNorm/
  FlashAttention fusion pass).

### 1.5 LLM application layer — a modern inference stack at .vr level
`core/math/{agent,ssm,distributed,rag}.vr` (61 distinct opcodes):

* agent.vr: BPE + SentencePiece tokenizers, KVCache + PagedKVCache
  (copy-on-write beam search), `flash_attention(_with_cache)` /
  `paged_attention`, SpeculativeDecoder, ContinuousBatcher,
  QuantizedLinear (+`quantize_per_channel`), LLMAgent/ReActAgent/
  function-calling, prompt/JSON-schema machinery.
* ssm.vr: S4 + Mamba (selective scan via higher-order `SSM_SCAN`), FFT
  conv path (RFFT/IRFFT/COMPLEX_*), MoE (router strategies + balance
  losses), Jamba hybrid — on const-generic records.
* distributed.vr: full collective surface (all_reduce/all_gather/
  reduce_scatter/broadcast/scatter/gather/P2P/barrier), DDP, tensor
  parallel (Column/RowParallelLinear), 1F1B pipeline, FSDP, actor mesh
  (Monarch-style), RDMA refs.
* rag.vr: HNSW (real graph, pure Verum), BM25, RRF hybrid retrieval,
  chunkers — nearly opcode-free.

### 1.6 GPU CPU-simulator — real per-thread kernel execution
`gpu_simulator.rs` + `handlers/gpu.rs:706-746`: `GPU_LAUNCH` iterates the
full grid×block ThreadIterator, sets a per-thread `GpuThreadContext`
(threadIdx/blockIdx/warp/lane), and **executes the kernel's bytecode
per-thread through the nested dispatch loop**, with real shared-memory
blocks and read-modify-write atomics. Documented limitation: sequential
threads, `__syncthreads()` no-op (cross-thread producer/consumer across
barriers computes wrong results).

---

## 2. The disconnects (defect classes found by this audit)

### 2.1 Autodiff: three islands, zero gradients  ⟶ AD-TAPE-WIRING-1
`interpreter/autodiff.rs` (3.6 KLOC) is a **complete, correct
reverse-mode Wengert-list engine** — `GradientTape`, ~140 `TapeOp`s, 44
mathematically-correct VJP rules, true reverse accumulation
(`backward_reverse` :712). It is **dead code**: nothing ever records onto
the tape.

* `handle_grad_begin` (`handlers/system.rs:441`) **discards the wrt
  registers**; `handle_grad_end` (:465) runs backward() over the empty
  tape and writes `0.0` into every grad register.
* No arithmetic/tensor forward handler calls `record_op` — grad_tape is
  touched only by the grad opcodes themselves.
* `dispatch_module_backward` is an identity stub ("Gradient Operations
  (Single-Process Stubs)", `kernel/mod.rs:2795+`).
* `core/math/autodiff.vr`'s `vjp` expects `GRAD_END` to return a
  **pullback closure**; the handler returns scalars.
* Not one test in the repo computes and checks a real gradient value
  (`379_interp_autodiff_multivariate.vr` hardcodes analytic derivatives;
  `151_ml_autodiff_aot.vr` is a tagged stub; L3 `001_scalar_grad.vr`
  asserts numel only).

**Every `grad(f)(x)` in Verum today returns 0.0.**

### 2.2 GPU plumbing: wire-format anarchy  ⟶ T0177 (in flight)
The `GpuExtended` opcode has THREE shapes across FOUR consumers: the
registry-driven emitter packs `[dst?][args…]` keyed on `return_count`
(`expressions.rs:34901`), the interp handler reads `[dst][args…]`
unconditionally (`handlers/gpu.rs`), the AOT lowering agrees with interp
(`llvm/instruction.rs:35037+`), and the legacy structured codec uses a
third shape with an immediate byte (`bytecode.rs:1192`). 32 `GPU_*`
registry rows say `return_count: 0` while `gpu.vr` declares `-> Int` —
result: `gpu_malloc → gpu_memset` deterministically **SIGSEGVs the
interpreter** (reads a garbage register index from the next instruction's
bytes), and every "status Int" evaluates to unit.

### 2.3 Tensor value path from .vr  ⟶ T0176
`tensor_fill([2,2],3.0,0)` → `tensor_get_scalar(t,0)` reads `0.0` on
Tier-0 (deterministic); `GetScalar` ignores its packed idx operand
(`tensor_extended.rs:3868` vs emitter `expressions.rs:34786`); the opaque
handle f-string renders a foreign variant name. The REAL engine of §1.1
sits **behind this broken register-plumbing layer** — the language cannot
reach what the runtime already implements.

### 2.4 Name resolution rolls dice  ⟶ T0175 (in flight)
Typecheck of mounted intrinsic-wrapper generics is **nondeterministic**
(5/6 cold runs fail, 1/6 green, same binary): one resolution face yields a
rigid-generic scheme ("expects 0 explicit type arguments"). Blocks the
new `core-tests/intrinsics/simd` suite and any generic stdlib wrapper.

### 2.5 Kernel authoring: parse-only theatre
`@kernel(block_size=…) fn` + `global_id()` **parse but bind to nothing**
(generic advisory attribute, `verum.ebnf:443-447`; no resolver/codegen
site). `gpu_launch` takes an opaque `kernel_id: Int` — there is no way to
author, from Verum source, the kernel being launched. 47 of 76 L3 GPU
specs are `@test: parse` ("downgraded to parse-only"); the one AOT Metal
spec self-stubs `__metal_*` to 0 and passes by skipping.

### 2.6 Doc/claim drift (10 candidates, D1-D10)
Highest-severity: `README.md:64` claims AOT "MLIR (GPU targets: PTX,
HSACO, SPIR-V, Metal)" — no such lowering exists — and "No JIT" — while
the default tensor path IS an MLIR JIT. `TEST_INDEX.md` marks "GPU tensor
operations ✅ Complete". `core/simd` spec doc lists SIMD opcodes
"0xC0-0xCF `SIMD_*`" that don't match `SimdSubOpcode`. `GPUBackend` vs
`GpuBackend` casing split between `math/gpu.vr` and `simd/gpu.vr`.
`docs/architecture/low-level-features.md` is referenced by core/simd
comments but does not exist. `mod.vr` re-exports dangling names
(`apply_rope`, `from_fn`, `Dim`, `Shape`, …) that no declaration backs.

### 2.7 LLM-serving stub perimeter  ⟶ LLM-SERVING-STUBS-1
The handler layer is ~75-80% real; the labeled stubs concentrate here
(`kernel/mod.rs`): `dispatch_sample_top_k` → argmax (ignores k, :2920),
`topk_top_p` → argmax, `repetition_penalty` → clone, `kv_cache op` → nil,
`speculative_verify` stub, `module_backward` → identity — while
`sample_top_p` and `sample_temperature` are REAL. Also `SAMPLE_TOP_K`
has no .vr surface at all; no pmap/vmap transforms; distributed
collectives degrade to single-process identity/zeros (inherent for one
process — needs a loud contract, not silence). Tokenizers are real but
feature-gated (`tokenizers`); the fallback returns a dummy handle.

### 2.8 Breadth gaps (honest unsupported-guards, not bugs)
* Heavy tensor ops F32/F64-only (matmul/conv/linalg/axis-reduce); f16/
  bf16 exist as dtypes + capability flags but not in heavy kernels.
* No quantized dtypes (no int4/fp8/qint8) and no quantize/dequantize fns
  — while the interp kernel already ships `dispatch_quantized_matmul`,
  `dispatch_paged_attention`, tokenizers, distributed collectives —
  **capabilities exist below the surface with no .vr reachability**.
* `Tensor<T, Shape>` static shapes blocked on const-generic parser
  support (`tensor.vr:267-269`) — ties into T0145/T0103 mono work.
* `@simd`/`@vectorize` attributes are advisory-only (global
  `[codegen].vectorize` toggle exists; per-loop steering does not).
* SIMD raw layer is a scalar fallback on BOTH tiers (tier-coherent,
  pinned by `core-tests/intrinsics/simd/regression_test.vr`); true
  multi-lane register values are the T0112 umbrella.

---

## 3. The staged plan (filed as pool tasks)

Ordered by leverage; each stage independently shippable.

| Stage | Task | Content |
|---|---|---|
| 0 | T0177 / T0176 / T0175 | Plumbing honesty: GPU wire shape, tensor value path, deterministic resolution. Already claimed/in-flight this session. |
| 1 | **T0180** AD-TAPE-WIRING-1 | Connect the three autodiff islands: forward ops record onto `grad_tape` (interp) + AOT twin; `GradBegin` keeps wrt set; `GradEnd` materializes the pullback (closure or tape-replay handle); kill the identity `module_backward`; first REAL gradient conformance suite (`grad(x²)(3)==6` and up through matmul/softmax/attention VJPs). |
| 2 | **T0181** GPU-DEVICE-LANE-1 | Ship the Metal leg: `metal` feature default-on for macOS hosts + `@cfg(gpu)`-gated conformance lane; un-stub `1072_metal_gpu_compute.vr`; wire `enumerate → select → alloc → memcpy → launch(prebuilt kernel) → readback` e2e vs CPU oracle. |
| 3 | **T0182** KERNEL-AUTHORING-DESIGN-1 | Design doc + prototype: `@kernel` fn → MLIR `gpu` dialect → device codegen (MSL first, PTX/SPIR-V after), `global_id()`/`Dim3` binding, launch-from-source replacing opaque `kernel_id`. Depends on stage 2. |
| 4 | **T0183** TENSOR-DTYPE-BREADTH-1 | f16/bf16 heavy-op kernels (via JIT templates where scalar is too slow), axis-reduce dtype widening, quantized dtype design (int8/int4/fp8) + surface the existing quantized_matmul/paged_attention/tokenizer kernels as .vr API. |
| 4b | **T0184** LLM-SERVING-STUBS-1 | Real top-k / top-k-top-p / repetition-penalty samplers + `SAMPLE_TOP_K` surface; KV-cache op + speculative-verify honest implementations; loud single-process contract for collectives (structured error or documented degradation, never silent zeros). |
| 5 | **T0186** STATIC-TENSOR-SHAPES-1 | `Tensor<T, meta Shape>` on const generics once T0145/T0103 land; compile-time shape checking; static matmul/transpose family (`tensor.vr:3001-3008`). |
| 6 | **T0187** L3-GPU-SPEC-REENABLE-1 | Flip the 47 parse-only L3 specs to run/aot as stages 1-3 unlock them; delete the self-stubbed Metal spec pattern. |
| — | **T0185** DOC-TRUTH-GPU-1 | Fix D1-D10 drift now (README GPU-targets + No-JIT lines, casing unification, dead doc references, dangling re-exports). Cheap, immediate. |

Umbrella: **T0179** GPU-TENSOR-RETHINK-1.

Cross-references: T0112 (SIMD vector lowering), T0116 (residual intrinsic
registration incl. SIMD compares / GPU set), T0130 (byte-conversion
containers), T0132 (dead SIMD clusters behind blanket allows), T0140
(absorbed by T0176), tech-debt register rows A10/C3/C6/A15.

---

## 4. Method note

Findings above were produced by seven parallel read-only audits (surface,
execution stack, runtime/devices, autodiff, kernel-authoring, docs,
debt cross-ref) + live probes against a clean-worktree toolchain at
`8f82d9f16` (probes preserved under the session scratchpad; regression
pins land in `core-tests/intrinsics/{simd,gpu}` with their tasks).
