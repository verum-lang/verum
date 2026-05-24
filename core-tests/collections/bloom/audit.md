# `core.collections.bloom` — Audit

Conformance review for `core/collections/bloom.vr` — `BloomFilter`,
classic Bloom filter with HMAC-SHA256 keyed Kirsch-Mitzenmacher
double hashing.  Capacity is sized via
`m = -n·ln(p) / ln(2)^2` (bits) and `k = m/n · ln(2)` (probe count),
approximated from a small table.

## Status

**partial** — All 4 non-`@ignore`'d regressions green (`new`,
`with_target`, `with_defaults`, `try_with_target`).  The 4
`@ignore`'d §B pins are gated on a SEPARATE defect class
(HMAC-SHA256 internal `[Byte; 64]` IndexOutOfBounds), not the
cross-module dispatch defect that was the original blocker.

**Task #47 CLOSED 2026-05-24** — cross-module Call name-encoding
via stage-3 stub pre-register + finalize-time
`emit_missing_stub_descriptors_with_callm(false)` descriptor
synthesis.  `BloomFilter.try_new`'s body now compiles cleanly
without panic-stub; at archive load, `ArchiveBodyRemap`'s Tier-2b
name fallback resolves the cross-module `Call(stub_id)` to the
real `core.sys.common.random_bytes` user-side FunctionId.

Original pre-fix failure surface (preserved for historical
context):

```
[lenient] BloomFilter.try_new compiled to panic-stub:
undefined function: fill_secure (in function BloomFilter.try_new)
```

## Re-diagnosis 2026-05-23

The original audit (above) attributed the failure to a missing
`core.sys.common.random_bytes` VBC intrinsic. Investigation on
2026-05-23 disproved that framing — `random_bytes` IS wired (see
`core/sys/common.vr:1206` + the syscall-registry `getrandom` /
`getentropy` / `BCryptGenRandom` plumbing in
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/ffi_extended.rs`).

The **actual** root cause is cross-module function-id resolution:

* `core/collections/bloom.vr:56` does `mount core.security.util.rng.{fill_secure};`
  and calls bare `fill_secure(&mut key)` from `BloomFilter.try_new`.
* `core.security.util.rng` lives under `core.security`, which already
  depends on `core.collections` because `security/{password_hash, token,
  merkle, otp, cose, kdf/pbkdf2, jwt, kdf/argon2, aead/chacha20_poly1305,
  aead/aes_gcm}.vr` all mount `core.collections.List`.
* The augmentation pass at `crates/verum_compiler/src/core_compiler.rs::
  augment_dependencies_from_mounts` detects the back-edge
  `core.collections → core.security` and DROPS it (cycle-tolerant gate)
  to preserve the existing forward edge.
* `core.collections` therefore compiles FIRST.  When `bloom.try_new`'s
  body is lowered, the per-module ctx's `lookup_function("fill_secure")`
  returns None → codegen emits a lenient panic-stub for
  `BloomFilter.try_new` carrying the message
  `undefined function: fill_secure`.

Same architectural class hits HyperLogLog, CountMinSketch,
AliasSampler.

### Why tactical stdlib-source workarounds don't help

The investigation tried THREE re-routings of the CSPRNG draw to
`core.sys.common.random_bytes` (which `core.sys` already exports
without a topo cycle):

1. `mount core.sys.common.{random_bytes};` — bare-name collides with
   `core.sys.linux.auxv.random_bytes() -> *const Byte` (different
   signature, same simple name).
2. `mount core.sys.common.{random_bytes as csprng_fill};` — the
   `mount X as Y` rename alias has known AOT-path issues
   (commit `b59c43f89`).
3. Fully-qualified call `core.sys.common.random_bytes(...)` — passes
   compile-time (no panic-stub) but at runtime the `Call(id)` mis-
   resolves through `ArchiveBodyRemap`'s Tier-3 identity fallback to
   whatever function happens to occupy the resolved id (observed live
   failures: `DequeIntoIter.zip_longest@pc=8` and
   `DequeDrain.map@pc=12`).

### Why a global pre-register of free-fn stubs doesn't close it either

A stage-3 pre-register pass (mirroring the existing stage-1 canonical-
type-static-method + stage-2 variant-constructor pre-registers in
`crates/verum_compiler/src/pipeline/stdlib_bootstrap.rs`) was prototyped
during the investigation and reverted. It would register every
uniquely-named public free fn as a stub in `global_function_registry`,
which makes `lookup_function("fill_secure")` succeed during bloom's
compile (no panic-stub).  But the per-module `finalize_module_from_state`
deliberately does NOT call `emit_missing_stub_descriptors` — so bloom.vbc's
output `module.functions` table carries NO descriptor for the stub id,
and `ArchiveBodyRemap`'s Tier-2 name fallback can't fire (the
`archive_id_to_name` lookup misses for the stub id).  The Tier-3 identity
fallback then lands the `Call` on whatever unrelated function occupies
the raw stub id.

Adding the descriptor-emit call IS the natural next step, but the
documented history at `crates/verum_vbc/src/codegen/mod.rs:5687-5750`
shows that even the surgical Call-id-only descriptor synthesis at stdlib
precompile scale blows `runtime.vbca` from 12.9 MB to 134 MB (~800K
synthesized stubs across ~500 modules).

### Architectural fix path

Task **#47** is the bytecode-format change that closes this defect
class universally: encode cross-module `Call` operands as `StringId`
(function name) instead of raw `func_id`.  The user-side merge then
resolves by name once per Call site, not once per
`(module × imported-function)` pair — eliminating the explosion and
making cross-module dispatch order-independent.

Estimated scope: multi-day VBC + codegen work, plus archive-format
version bump + migration of every loader callsite.

### What the test surface should look like post-#47

Once #47 lands, every `@ignore`'d test in §A becomes active without
stdlib-source changes — the cross-module call to `fill_secure`
resolves correctly by name regardless of compile order.

Working surface today: only `BloomConfig` value construction and
field access — 3 unit + 3 property + 2 integration + 2 PASS-GUARDs
(10 / 10 green on `--interp`).

## 1. Cross-stdlib usage

Downstream consumers — cache-key dedup, URL-seen tracking,
log-line dedup, anti-replay caches.  Surface is prospective today.

## 2. Crate-side hardcodes

The defect that gates Bloom is the missing
`core.sys.common.random_bytes` intrinsic in the VBC dispatch
table.  Same intrinsic landing-pad gates `BloomFilter` /
`HyperLogLog` HMAC keys, `Reservoir.offer` replacement-phase
randomness, and every other CSPRNG consumer.

## 3. Language-implementation gaps

| Gap | Impact | Fix path |
|---|---|---|
| `core.sys.common.random_bytes` intrinsic missing from VBC dispatch table | Blocks every CSPRNG consumer including Bloom, HyperLogLog, Reservoir | Register the intrinsic in `crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic` and wire to the platform random-bytes syscall.  Estimated 1-2 days. |

## 4. Defect inventory

Per `regression_test.vr`:

### §A — CSPRNG-gated constructors (8 pins)

* §A.1 `BloomFilter.with_target(cap, fp)` panics
* §A.2 `BloomFilter.new(cfg)` panics
* §A.3 `BloomFilter.with_defaults()` panics
* §A.4 `insert(&[Byte])` / `contains(&[Byte])` round-trip not reachable
* §A.5 `check_and_set(&[Byte])` idempotence not reachable
* §A.6 `admitted` counter not reachable
* §A.7 `clear` resets `admitted` not reachable
* §A.8 `try_with_target` invalid-fp validation not reachable

## 5. Action items

### Landed in this branch

1. Unit-test surface — 3 tests on the BloomConfig construction
   surface (defaults capacity / defaults fp_rate / literal round-
   trip).
2. Property-test surface — 3 algebraic laws (defaults capacity
   positive; defaults fp_rate in range; literal preserves both
   fields).
3. Integration tests — 2 cross-type scenarios (defaults match
   explicit literal; capacity arithmetic).
4. Regression suite — 8 @ignore'd pins for §A + 2 PASS-GUARDs for
   the config surface.

### Deferred

1. **Register `core.sys.common.random_bytes` intrinsic** in VBC
   dispatch table — unblocks every CSPRNG consumer at once
   (Bloom + HyperLogLog + Reservoir).
2. Construction surface tests post-intrinsic-landing —
   `with_target` size computation (`m`/`k` from `cap` × `fp_ppm`),
   `try_with_target` error cases (zero / too-large capacity,
   invalid fp_rate).
3. False-positive-rate property tests — for known n and ppm,
   measure observed false-positive rate stays under bound.
