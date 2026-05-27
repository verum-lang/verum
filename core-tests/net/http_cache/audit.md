# `net/http_cache` audit

Module: `core/net/http_cache.vr` (~433 LOC) — RFC 9111 §5.2
Cache-Control directive parser + §4.2 age formula +
freshness lifetime + cache-decision pipeline.

Tests cover the data-surface algebra: CacheControl 16-field
record (all RFC 9111 §5.2 directives), CacheControlError
1-variant (Malformed), MAX_CACHE_CONTROL_DIRECTIVES DoS
hardening constant, AgeInputs 5-field record, DecisionInputs
5-field record, CacheDecision 4-variant (Fresh / StaleServable
/ MustRevalidate / MustNotCache) disjointness.

Full functional surface (`parse`, `current_age`,
`freshness_lifetime_sec`, `decide`) is locked-in behind
HTTPCACHE-1 in `regression_test.vr`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.cache.*` adapters | TTL ↔ Cache-Control directive translation. |
| `core.net.weft` middleware | server-side cache header emission. |
| `core.net.http` clients | client-side `decide` for response caching. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic.

## 3. Language-implementation gaps

### §3.1 HTTPCACHE-1 — `parse` / `current_age` / `freshness_lifetime_sec` / `decide` SIGSEGV

**Stable trigger**: same precompile-cascade defect class as
CIDR-1 / URL-1 / URITPL-1 / HTTPRNG-1 / CONNEG-1 / LINKHDR-1.

The data-surface (CacheControl, CacheDecision variant
construction + Eq) compiles. Functional surface locked-in by
6 @ignore'd regression pins.

### §3.2 MAX_CACHE_CONTROL_DIRECTIVES pinned at 64

CVE-2011-3192-class hardening — real-world headers carry ≤ 5
directives. 64 is generous.

## 4. Action items landed in this branch

* `core-tests/net/http_cache/unit_test.vr` — 23 unit tests
  covering CacheControl 16-field record construction +
  no-cache/no-store distinction (§5.2.2.4 vs §5.2.2.5) +
  s_maxage > max_age + immutable (RFC 8246) + stale_*; 
  CacheControlError Eq + Malformed payload disjointness;
  MAX_CACHE_CONTROL_DIRECTIVES (3 pins); AgeInputs (2);
  DecisionInputs construction; CacheDecision 4-variant
  disjointness (6).
* `core-tests/net/http_cache/regression_test.vr` — 6
  @ignore'd LOCK-IN pins for HTTPCACHE-1.
* `core-tests/net/http_cache/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close HTTPCACHE-1 (batched with CIDR-1 family) | VBC codegen | 3-5 days |
| Full parse coverage (every §5.2 directive incl quoted strings) | this folder | 1h, gated on §3.1 |
| Property test: parse + decide preserves no-store / private dispatch | this folder | 2h, gated on §3.1 |
| RFC 9111 §4.2.2 heuristic freshness (Last-Modified → 10% rule) | stdlib add | 4h |
| RFC 7234 Vary-aware cache key derivation | stdlib add | 1 day |
