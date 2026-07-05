# `intrinsics/gpu` audit

Module: `core/intrinsics/gpu.vr` (~48 intrinsics) — device management,
kernel launch, memory transfer, thread/block indexing, barriers.

## Coverage decision: AUDIT-ONLY (no `*_test.vr`)

Every GPU intrinsic requires a live accelerator (CUDA / Metal / ROCm /
Vulkan) and a kernel-launch context.  The test host is a CPU-only build;
there is no device to run against, and the GPU thread-model intrinsics
(`ThreadIdX`/`BlockIdX`/…, GpuSubOpcode space) are only meaningful inside a
launched kernel.  A conformance test would need a device harness that the
CI host cannot provide.

## Contract notes (pinned by inspection)

* Device ops route through `GpuSubOpcode` (registry `GPU_*` entries).
* The CPU-fallback thread model (`ThreadIdX = 0xA0` in the sampling
  sub-opcode space) emulates the GPU indexing for scalar execution — that
  fallback IS exercised by the tensor JIT path where a GPU is absent.

## Action items

* Device-backed conformance belongs in a hardware-gated CI lane
  (`@cfg(gpu = "…")`), not the core-tests suite.  Deferred until a
  device-harness lane exists.
