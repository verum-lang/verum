# `core.collections.bloom` — Audit

Conformance review for `core/collections/bloom.vr` — `BloomFilter`,
classic Bloom filter with HMAC-SHA256 keyed Kirsch-Mitzenmacher
double hashing.  Capacity is sized via
`m = -n·ln(p) / ln(2)^2` (bits) and `k = m/n · ln(2)` (probe count),
approximated from a small table.

## Status

**regression-only** — Every BloomFilter constructor routes through
`fill_secure(&mut key[..])` (32-byte HMAC key generation), which
lowers to the `core.sys.common.random_bytes` intrinsic.  That
intrinsic is missing from the VBC dispatch table — same defect
class as the closed Reservoir replacement-phase defect (see
`core-tests/collections/reservoir/regression_test.vr`).

Failure surface — every constructor (`new`, `with_target`,
`with_defaults`, `try_new`, `try_with_target`) panics with:

```
[lenient] BloomFilter.try_new compiled to panic-stub:
undefined function: fill_secure (in function BloomFilter.try_new)
```

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
