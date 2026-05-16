# `core.collections.ttl_cache` — Audit

Conformance review for `core/collections/ttl_cache.vr` —
`TtlCache<K, V>`, hybrid LRU + per-entry-TTL cache.  Backed by
`Map<K, Entry<V>>` for O(1) value lookup + `List<K>` for LRU order
tracking + `Instant`-denominated expiry timestamps.

## Status

**regression-only** — Fundamental source-level defect §A:
`ttl_cache.vr:109` called `Instant.add_duration(Duration)`, a non-
existent method (the inherent impl declares `checked_add(Duration)
-> Maybe<Instant>` at `instant.vr:179`).  Same defect occurred at a
second site in `core/net/quic/path_mtu.vr:138`.

Source fix landed in this branch — both call sites now use
`match now.checked_add(ttl) { Some(t) => t, None => fallback }`.
The verum binary embeds the precompiled stdlib at compile time, so
the CallM operand referring to `Instant.add_duration` remains in
the embedded archive until a fresh `cargo build --release`.

The 9 unit tests that touch the insert path are pinned in
`regression_test.vr` §A as `@ignore`d; they will flip to active
after the next binary rebuild and become the working-surface
PASS-GUARDs.

Working surface — exercised by 5 unit + 5 property + 3 integration
+ 4 PASS-GUARD tests (all green on `--interp`):

* Construction (`TtlCache.new(cfg)`, `with_defaults()`)
* Empty-cache read-only ops (`get` / `remove` / `purge_expired` —
  return None / false / 0 respectively)
* Initial stats (size=hits=misses=evicted=expired=0)
* `is_empty()` / `len()` coherence on fresh
* TtlCacheConfig + Duration interop (`from_secs` / `from_millis`)
* Multi-instance independence

## 1. Cross-stdlib usage

Same shape as `LruCache` — session caches, JWT replay, DNS,
hot-key bookkeeping.  Surface is prospective today.

## 2. Crate-side hardcodes

No runtime intercepts specific to TtlCache; every operation pushes
through Map / List / Instant / Duration whose runtime intercepts
are tested separately.  The defect that gates the insert path is
SOURCE-level: a stale method-name in stdlib source code that the
precompile-and-embed pipeline froze into the verum binary.

## 3. Language-implementation gaps

| Gap | Impact | Fix path |
|---|---|---|
| `ttl_cache.vr:109` called `Instant.add_duration` (non-existent) — **CLOSED at source level** | Every TtlCache insert path panics at runtime with "method 'Instant.add_duration' not found on receiver of runtime kind `Int`" | Source fix landed: use `match now.checked_add(ttl) { Some(t) => t, None => Instant.now() }`.  Awaits `cargo build --release` to refresh the embedded precompiled stdlib in the verum binary. |
| `path_mtu.vr:138` had the same defect — **CLOSED at source level** | QUIC PMTU SearchComplete transition crashed | Same fix pattern landed at `core/net/quic/path_mtu.vr:138`. |
| No injectable clock for test-side time travel | Can't exercise expiry-driven `get` / `purge_expired` paths deterministically | Add `TtlCache.new_with_clock(cfg, clock)` accepting a `Clock` trait, defaulting to `Instant.now`-backed real clock.  Estimated 1 day. |

## 4. Defect inventory

Per `regression_test.vr`:

### §A — TtlCache insert path panics on Instant.add_duration

* §A.1 `insert(K, V)` panics
* §A.2 `insert_with_ttl(K, V, Duration)` panics
* §A.3 capacity-pressure eviction not reachable (cascades from §A.1)
* §A.4 `remove` after insert not reachable
* §A.5 `stats.hits` not exercisable (cascades from §A.1)
* §A.6 `clear` after insert not reachable
* §A.7 `purge_expired` on a populated cache not reachable

All 7 entries are `@ignore`'d in `regression_test.vr`; they flip to
active when the precompiled stdlib in the verum binary refreshes.

## 5. Action items

### Landed in this branch (source-level)

1. `core/collections/ttl_cache.vr:108-117` — replaced
   `now.add_duration(ttl)` with a `match now.checked_add(ttl)`
   block (overflow saturates to `Instant.now()`, the safest fail-
   mode for TTL scheduling).
2. `core/net/quic/path_mtu.vr:138` — parallel fix at the QUIC PMTU
   SearchComplete transition site.

### Landed in this branch (test infrastructure)

1. Unit-test surface — 6 tests covering construction, empty-cache
   read-only ops, and stats initial state.
2. Property-test surface — 5 algebraic laws (with_defaults canonical;
   miss counter monotone; purge on empty; remove absent idempotent;
   is_empty iff len-zero).
3. Integration tests — 3 cross-type scenarios (config + Duration;
   read-only round-trip; multi-instance independence).
4. Regression suite — 7 `@ignore`'d defect-pins for §A (insert path)
   + 4 PASS-GUARDs for the working surface.

### Deferred

1. **Binary rebuild** — `cargo build --release` to refresh the
   embedded precompiled stdlib in `target/release/verum`.  Once
   refreshed, the 7 §A pins flip green and TtlCache promotes to
   `partial` / `complete`.
2. **Test-clock fixture** — deterministic expiry tests for `get`-
   after-expiry and `purge_expired` removing only expired entries.
