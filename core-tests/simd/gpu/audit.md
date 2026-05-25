# `simd/gpu` audit

Module: `core/simd/gpu.vr` (~293 LOC) — GPU compute types (CUDA /
ROCm / Metal / Vulkan) + thread/block/grid intrinsics +
shared-memory primitives.

Tests: 24 unit tests over GpuBackend 4-variant + TransferKind
3-variant + Grid + GpuBlock records via direct construction +
.total_threads.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.math.linalg.gpu` | matrix multiply / FFT / conv kernels target this API |
| `verum_compiler::gpu_lowering` | reads GpuBackend to select MLIR dialect (NVVM / ROCDL / Metal / SPIRV) |
| `verum_runtime::gpu_alloc` | TransferKind drives the memcpy direction at the runtime memory-management layer |

## 2. Crate-side hardcodes

* `verum_runtime::gpu::backend` mirrors GpuBackend 4-variant.
* `verum_runtime::gpu::dispatch` mirrors TransferKind 3-variant.
* `verum_compiler::gpu_targets` — CUDA SM version compatibility
  table (sm_70 → Volta, sm_80 → Ampere). The `GpuConfig.cuda(sm_version)`
  factory has hardcoded thresholds: tensor cores ≥ 70, async-copy ≥ 80.
  Pinned in source comments; drift here breaks CUDA runtime detection.

## 3. Language-implementation gaps

### §3.1 GpuConfig factories are cross-module ctors

`GpuConfig.metal()`, `.cuda(sm)`, `.rocm()`, `.vulkan()`, `.auto()`
each return a `GpuConfig` record. Subsequent field access on the
result hits the cross-module record-return defect class (see
`meta/span` audit §3.1).

Workaround: test backend determination via direct field-by-field
construction at the test site. Deferred until cross-module fix
lands.

### §3.2 GPU thread intrinsics not runtime-testable

`thread_id_x()` / `block_id_x()` / `block_dim_x()` / `grid_dim_x()` /
`sync_threads()` / `sync_warp()` / `warp_size()` / shared-memory
load/store/atomic ops are all `@intrinsic("gpu_*")` and require:
1. Running inside a `@device(gpu)` function context, and
2. A live GPU target compilation pipeline.

At the stdlib test layer (Tier 0 interp), they're effectively
unreachable. Live testing belongs at `vcs/specs/L3-extended/gpu/`
under a `@cfg(feature = "gpu")` gate.

### §3.3 `GpuConfig.auto()` uses `@cfg(target_os = ...)` branches

The auto-detect path returns different configs per target OS:
* macOS → Metal
* Linux → CUDA sm_80
* Windows → CUDA sm_80

Testing requires building under each target — covered at the
CI matrix level.

## Action items landed in this branch

* `core-tests/simd/gpu/unit_test.vr` — 24 unit tests over
  GpuBackend 4-variant + TransferKind 3-variant + Grid +
  GpuBlock + .total_threads (1D / 2D / 3D / max-CUDA / unit).
* `core-tests/simd/gpu/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| GpuConfig factory return-value tests (§3.1) | this folder | 30 min after cross-module fix |
| GpuDevice / GpuBuffer record-construction tests | this folder | 30 min |
| @cfg(feature = "gpu") integration tests for kernel launch (§3.2) | vcs/specs/L3-extended/gpu/ | 1-2 days |
| Property test: GpuBlock.total_threads = x * y * z exhaustively for boundary values | this folder | 30 min |
| Drift-pinning Rust unit test for GpuBackend → MLIR dialect routing | crates/verum_compiler/src/gpu_lowering.rs | 30 min |
