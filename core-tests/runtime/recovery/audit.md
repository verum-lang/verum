# `runtime/recovery` audit

Module: `core/runtime/recovery.vr` (1083 LOC) — retry policy + circuit
breaker + backoff + jitter + RuntimeRecoveryStrategy ADT + inline
variants (InlineCircuitBreaker / InlineRetryPolicy).

Tests: 30 unit tests over the data-only subset (4 ADTs + 3 records +
2 smart ctors).  Live retry/circuit-breaker behaviour requires async-
spawn + sys.sleep binding — deferred to `vcs/specs/L2-standard/runtime/
recovery/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.async.spawn.SpawnConfig` | embeds a `RuntimeRecoveryStrategy` to define how each spawned task handles errors. |
| `core.io.engine.IoEngine` | wraps fallible I/O calls in `RecoveryCircuitBreaker` for upstream connection failure handling. |
| `core.database.{postgres, mysql, redis}` | uses `RecoveryRetryPolicy` for connection-establishment retries. |
| `core.net.http.Client` | uses `RecoveryBackoffStrategy.Exponential` for rate-limited request handling. |
| `core.mesh.xds` (Envoy xDS client) | uses circuit breakers + composed strategies for ADS push failures. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `@repr(UInt8)` on RecoveryCircuitState | wire-format ordinal: Closed=0, Open=1, HalfOpen=2 | Drift here breaks atomic-state-transition snapshots in metrics + circuit-breaker introspection. |
| `from_u8` recovery `_ => Closed` fallback | unknown UInt8 → Closed (fail-safe to "allow all") | Recovery from a corrupted atomic state silently re-enables circuit; logging recommended (audit §C). |
| Default `circuit_breaker(N)` parameters: required_successes=3, timeout_ms=60000 | hardcoded magic numbers | Production tuning needs explicit override. |
| Default `retry(N)` parameters: base_ms=100, max_ms=5000, multiplier=2, jitter=Proportional(10%) | hardcoded | Same. |

## 3. Language-implementation gaps

### §A — `RetryPredicate.Custom(fn(&Text) -> Bool)` consumes `&Text`

The custom predicate sees the error message as Text only.  A
caller wanting to filter by error TYPE (e.g., retry on
`ConnectionResetError` but not `AuthError`) must inspect the
formatted message — brittle to formatter changes.  Recommend:
add a second-form `Custom_typed(fn(&dyn Error) -> Bool)`.

### §B — `RecoveryBackoffStrategy.None` variant aliases `JitterConfig.None`

Both share the bare `None` name.  Under the bare-variant cross-module
collision class (task #17/#39) this could cause dispatch confusion if
both types are in scope at the same call site.  Source uses qualified
form (`RecoveryBackoffStrategy.None` / `JitterConfig.None`) so OK
today; pin the discipline.

### §C — `from_u8` silently coerces unknown values to `Closed`

`RecoveryCircuitState.from_u8(99)` returns `Closed` — a corrupted
atomic state is silently coerced to "allow all traffic", which is
the WORST safe-fallback (fails open vs fail closed).  Recommend:
return `Maybe<RecoveryCircuitState>` and require the caller to
explicitly handle the unknown case OR log a warning.

### §D — `RuntimeRecoveryStrategy.Composed(List<RuntimeRecoveryStrategy>)` recursion

Composed strategies allow nesting other Composed lists — infinite
recursion hazard if someone constructs a cycle.  Recommend: bound
the nesting depth at construction OR forbid Composed-of-Composed.

### §E — Jitter PRNG uses unbiased `random_u64() % N`

The `jitter.apply` impl uses `random_u64() % (jitter_range * 2 + 1)`.
This has modulo bias when `2^64 % N != 0`.  For the typical jitter
range (10ms..5000ms) the bias is negligible (<2^-53) but should be
documented as a "good enough" approximation.

## Action items landed in this branch

* `core-tests/runtime/recovery/unit_test.vr` — 30 unit tests covering
  4 ADTs + 3 records + 2 smart ctors.
* `core-tests/runtime/recovery/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `RetryPredicate.Custom_typed` overload | `core/runtime/recovery.vr` | 1 h |
| §B Bare-variant collision discipline pin | this folder | 30 min (add property test for qualified form) |
| §C `from_u8` returns `Maybe` | `core/runtime/recovery.vr` + callers | 2 h |
| §D Composed strategy depth bound | `core/runtime/recovery.vr` | 1 h |
| §E PRNG bias documentation | `core/runtime/recovery.vr` docstring | 15 min |
| Live retry + sleep + record_retry round-trip | `vcs/specs/L2-standard/runtime/recovery/` | gated on async spawn |
| Live circuit-breaker state transitions (Closed → Open → HalfOpen → Closed) | sister | gated on atomic intrinsics |
| Display/Debug for RecoveryCircuitState + RetryPredicate | this folder | 30 min |
