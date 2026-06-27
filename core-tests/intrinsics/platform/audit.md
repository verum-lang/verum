# `intrinsics/platform` audit

Module: `core/intrinsics/platform.vr` (~93 LOC) — compile-time
platform-detection meta intrinsics + cycle-counter primitives.

Tests: `unit_test.vr` (per-intrinsic surface), `property_test.vr`
(complementarity / determinism / ranges / monotonicity), `integration_test.vr`
(compile-time dispatch patterns), `regression_test.vr` (defect pins).

## 0. Architectural model (load-bearing)

`is_debug` / `is_release` / `target_os` / `target_arch` /
`target_pointer_width` / `target_is_little_endian` are `meta fn`s with the
`CodegenStrategy::CompileTimeConstant` strategy — their value is materialised at
codegen time (`emit_intrinsic_compile_time_constant`) and is therefore a stable
constant within a build. Because they are platform-dependent, the suite asserts
**invariants** (valid ranges, byte-multiple width, debug/release
complementarity, determinism) plus the interpreter-specific contract
(`is_debug` true / `is_release` false), rather than host-specific magic numbers.

`rdtsc` / `rdtscp` read the hardware cycle counter at runtime;
`rdtsc` is pinned for callability + monotonicity.

> NOTE: the compile-time evaluator currently selects values via host `#[cfg]`
> (`target_os`/`target_arch`/`target_pointer_width`/endianness). Per
> `CLAUDE.md` every per-target decision should read the **target** triple
> (`module.get_triple()`), not host cfg — this is pre-existing debt shared by
> all platform meta intrinsics (the new entries follow the existing pattern for
> consistency; the triple migration is a separate, module-wide change).

## Tier summary

* **Interp: GREEN** once `PLATFORM-META-NIL` is fixed (see §2). All asserted
  intrinsics (build flags, target descriptors, `rdtsc`) pass.
* **AOT:** the same `CompileTimeConstant` lowering + `rdtsc` runtime call —
  validated alongside the intrinsics suite.

## 1. What is verified GREEN

* **build mode** — `is_debug()` true / `is_release()` false under interp;
  exactly one holds (`is_debug() != is_release()`).
* **target descriptors** — `target_os()` / `target_arch()` ∈ `0..4`;
  `target_pointer_width()` is a byte-multiple word (`% 8 == 0`, 32 ≤ w ≤ 128);
  `target_is_little_endian()` (supported targets are LE). All deterministic
  across repeated evaluation.
* **cycle counter** — `rdtsc()` callable and non-decreasing across sequential
  reads.
* **integration** — pointer-width→byte-width round-trip, endianness branch,
  `target_os` compile-time switch, `is_debug` guard pattern.

## 2. Defects FIXED on this branch (data-only)

### PLATFORM-META-NIL — `target_pointer_width` / `target_is_little_endian` / `is_release` → `nil`

These three returned `nil` while the neighbouring `is_debug` / `target_os` /
`target_arch` worked — they were missing **both** a registry entry AND a case
in `emit_intrinsic_compile_time_constant` (`expressions.rs`), so the name
resolved to `LoadNil`. (Same class as `CONTROL-EXPECT-NIL` / `CONV-INTWIDTH-1`.)

**Fix**:
* `registry.rs` — three `Platform` / `CompileTimeConstant` entries.
* `expressions.rs::emit_intrinsic_compile_time_constant` — `is_release`
  (`LoadFalse`, complement of `is_debug`), `target_pointer_width`
  (`LoadI 64`/`32` via host `target_pointer_width` cfg),
  `target_is_little_endian` (`LoadTrue`/`LoadFalse` via host `target_endian`
  cfg).

## 3. Defects OPEN / not value-tested

* **`target_has_atomic<T>()` / `target_has_feature(feature)`** — take a
  type/feature argument; not exercised here (likely the same registry/evaluator
  gap — to be probed + added with the triple migration).
* **`rdtscp() -> (UInt64, UInt32)`** — tuple-return; deferred (tuple-return
  surface, cf. `ATOMIC-CAS-AOT`).
* **`spin_hint()`** — side-effect-only hint; callable-only, not value-tested.
* **Host-cfg vs target-triple** — see §0 note; the values are correct for
  host==target (the interpreter case) but would miscompile a cross build.

## 4. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.sync` / `core.atomic` | `target_has_atomic` / `target_pointer_width` for layout. |
| `core.encoding` / serialization | `target_is_little_endian` for wire order. |
| build-mode-gated code | `is_debug` / `is_release` for assertions + fast paths. |
| `core.time` / profiling | `rdtsc` / `rdtscp` cycle counters. |

## 5. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/intrinsics/registry.rs` — the `Platform` entries.
* `crates/verum_vbc/src/codegen/expressions.rs::emit_intrinsic_compile_time_constant`
  — the per-intrinsic value computation (host `#[cfg]`; target-triple migration
  pending).

## 6. Action items

**Landed this branch (data-only)**
* PLATFORM-META-NIL — register + evaluate `is_release` /
  `target_pointer_width` / `target_is_little_endian`.
* Full platform conformance suite (unit/property/integration/regression + audit).

**Deferred (tracked)**
* `target_has_atomic` / `target_has_feature` / `rdtscp` / `spin_hint` coverage.
* Target-triple-driven evaluation (replace host `#[cfg]`) across all platform
  meta intrinsics.
