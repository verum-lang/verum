# `cache/types` audit

Module: `core/cache/types.vr` (233 LOC) — abstract cache protocol +
value model. Defines `CacheValue` (4-variant) + `CacheTtl`
(3-variant) + `CacheError` (7-variant) plus the `CacheBackend`
protocol that backends implement.

Tests focus on the three data ADTs (testable without a backend
instance). `CacheBackend` protocol is tested at the language level
via `vcs/specs/L2-standard/cache/` against the in-memory mock
adapter.

## 1. Cross-stdlib usage

`CacheValue`:
| crate / module | what it does |
|---|---|
| `core.cache.adapters.redis` | Maps RESP-encoded values to/from CacheValue variants. |
| `core.cache.adapters.memory` | In-memory LRU storage keyed on CacheValue. |
| `core.cache.adapters.multi_tier` | Read-through composition (L1 mem + L2 redis). |

`CacheTtl`:
| Every cache `set(key, value, ttl)` call | Persistent vs Seconds(N) vs Millis(N). |

`CacheError`:
| Every cache fallible op | Returns `Result<T, CacheError>`. |

## 2. Crate-side hardcodes

None today. `cache::types` is pure Verum; concrete adapters layer
on it via the protocol. Future Rust-side intercepts (e.g.
zero-copy bytes for hot Bytes path) would need to preserve variant
shapes — pinned by `test_cache_value_variants_disjoint`.

## 3. Language-implementation gaps

### §3.1 `CacheTtl.Seconds` / `Millis` refinement type — N >= 1

Both variants carry `Int { >= 1 }` refinement constraint. At
construction `CacheTtl.Seconds(0)` should fail at compile-time (or
runtime panic). Test surface for refinement violations is hard
without an `@expected-error` test framework; the test
`test_cache_ttl_seconds_construction` uses 60 (valid). Adding
`@expected-runtime-panic` for `CacheTtl.Seconds(0)` would close
this gap.

**Effort:** small once `@expected-runtime-panic` test fixture
lands.

### §3.2 `CacheValue.Counter` is Int — no overflow protection

`Counter(Int)` admits overflow for INCR/DECR ops that exceed
Int range. Either:
* Document that callers must use saturating math; OR
* Switch to `Counter(BigInt)` for compatibility with Redis 64-bit counters.

The Redis adapter (when implemented) likely needs BigInt anyway —
RESP INCR returns a string, parsed as arbitrary-precision integer.

**Effort:** medium (~2h) — touches every adapter that does INCR.

### §3.3 No `CacheValue.Eq` round-trip tests (gated on BigInt)

The `@derive(Eq, Clone, Debug)` annotation should auto-generate
Eq impl. Testing Eq for CacheValue variants requires Eq on the
payloads (List<Byte>, Text, Int, List<Text>) — all of which have
Eq impls. But the @derive macro itself has been observed to mis-
dispatch under the bare-variant hazard (task #17/#39). Add Eq
round-trip tests once a stable @derive surface is verified.

**Effort:** 1h + property tests.

### §3.4 `CacheError.TypeMismatch` could be typed (TypeId pair)

Today TypeMismatch carries `expected: Text` + `found: Text` as
free-form strings. A typed `expected: TypeRef` + `found: TypeRef`
would let the runtime check structural compatibility (e.g.
`Bytes` vs `TextValue` is a soft mismatch the adapter could
re-encode through). Deferred to V1 cache adapters that need this.

## Action items landed in this branch

* `core-tests/cache/types/unit_test.vr` — 24 unit tests covering
  the three ADTs' construction + disjointness + CacheError Display
  rendering.
* `core-tests/cache/types/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `@expected-runtime-panic` test for `CacheTtl.Seconds(0)` refinement | this folder | gated on test fixture |
| Switch `Counter(Int)` → `Counter(BigInt)` for overflow safety | `core/cache/types.vr` + adapter consumers | 2h |
| Add `Eq` / `Hash` round-trip tests for all 3 ADTs | this folder + property_test.vr | 1h |
| Add property_test.vr for Display determinism + variant exhaustiveness | this folder | 30 min |
| Add `core-tests/cache/adapters/{memory,redis,multi_tier}` integration suites | sister folders | 1 day each |
