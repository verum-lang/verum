# `intrinsics/lowlevel/mod` audit

Module: `core/intrinsics/lowlevel/mod.vr` (~145 LOC) — the low-level
umbrella.  The per-arch submodules (x86_64 / aarch64 / kernel / mmio) are
@llvm_only or privileged and stay AUDIT-ONLY (`../audit.md`); the umbrella
itself carries a testable CROSS-PLATFORM surface:

* `CpuCapabilities` record + `detect_capabilities()` — runtime feature
  dispatch (statement-level @cfg branches per target_arch).
* `MAX_SIMD_WIDTH` / `PREFERRED_SIMD_WIDTH` — @cfg-selected constants.

Suite added 2026-07-15: unit (5; 1 @ignore'd acceptance pin).

## 0. Finding — CFG-CONST-SELECT-1 (task #20)

On the aarch64 host, `detect_capabilities()` takes the aarch64 STATEMENT
branch (`has_fma == true`, unconditional on that arch) while
`MAX_SIMD_WIDTH == 128` — the `@cfg(not(any(x86_64, aarch64)))` FALLBACK
const.  So @cfg selection on CONST ITEMS ignores target_arch (probable
last-declaration-wins among the three same-name consts) while
statement-level @cfg inside fn bodies branches correctly.  Two @cfg
mechanisms disagreeing on one file is the defect; the const leg is wrong.

Pinned: `test_max_simd_width_matches_target_arch` (@ignore'd until #20
lands — on the two supported dev arches the fallback 128 is always wrong:
aarch64 declares 2048, x86_64 declares 512).

## 1. What is verified GREEN

* `detect_capabilities()` returns a coherent record: 128-bit SIMD present
  (every supported dev/CI arch), deterministic across calls, and the SIMD
  width ladder is monotone (512 ⇒ 256 ⇒ 128).
* The constants resolve to DECLARED values (128|512|2048, 128|256) with
  `PREFERRED <= MAX`.

## 2. Action items

* Task #20 — const-item @cfg selection keyed off the target (same
  discipline as `module.get_triple()` for codegen); then un-ignore the pin.
