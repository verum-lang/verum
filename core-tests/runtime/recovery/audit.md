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

### §F — `RecoveryRetryPolicy.new` field-write-OOB (REGRESSION, current binary)

**Surface:** `RecoveryRetryPolicy.new(config)` panics under `--interp`:

```
field write out of bounds: field index 4 (offset 32+8 = 40)
exceeds object data size 32 type_id=0 type='?'
backtrace=[RecoveryRetryPolicy.new@pc=33 <- ...]
```

`RecoveryRetryPolicy` is a 4-field record `{ config, current_attempt,
total_retries, last_error }` (data size 32 = 4×8). The constructor body
writes a 5th slot (field index 4). The smoking gun is **`type_id=0
type='?'`** — the type is *unregistered* in the VBC type-layout table, so
field writes fall through to the global `intern_field_name` fallback and
shift out of bounds. This is the documented field-shift codegen class
([[use_after_free_error_field_shift_2026-05-27]] /
`enactment_field_access_oob` / `btree_pattern_match_ref_generic_class`).

**Why it matters / regression note:** this folder's `unit_test.vr` was
marked "all GREEN" in INVENTORY by an earlier session, but on the current
(May-31) binary the four `RecoveryRetryPolicy.new` unit tests
(`test_retry_policy_new_initial_state`, `..._should_retry_initially_true`,
`..._should_retry_zero_max_attempts_false`, `..._public_alias_resolves`)
**all fail** with this exact panic — verified in isolation
(`--filter test_retry_policy_new_initial_state` → 0 passed; 1 failed).
The embedded-stdlib layout registration drifted between binaries. All five
integration `rcv_it_retry_*`/`rcv_it_next_delay_*` tests and the four unit
tests are now pinned `@ignore`.

**Fix surface (compiler, needs rebuild):** register `RecoveryRetryPolicy`
in the archive type-layout table so `compile_record` / `compile_static_
method_call` resolve its field count by type_id instead of falling through
to `intern_field_name` global keying. Same root as task #17/#39. **Cannot
land this session** — concurrent `verum test` sessions + the precompile-
poisoning hazard forbid a compiler rebuild.

### §G — `InlineRetryPolicy.default()` field-write-OOB

**Surface:** `InlineRetryPolicy.default()` panics `field index 9
(offset 72+8 = 80) exceeds object data size 72 type_id=0 type='?'`.
`InlineRetryPolicy` is a 9-field `@repr(C)` record; the `default()` body
writes a 10th slot. Same `type_id=0` field-shift class as §F. Notably the
sibling `InlineCircuitBreaker.default()` (also `@repr(C)`, with `[Byte;24]`
+ `[Byte;18]` array fields) **resolves correctly** — confirming the defect
is per-type layout-registration order, not a blanket `@repr(C)` /
array-field problem. Pinned `@ignore` (`rcv_it_inline_retry_default_fields`).

### §H — `RecoveryCircuitState` Display dispatch falls through

**Surface:** `f"{RecoveryCircuitState.Closed}"` does **not** yield
`"closed"` even though `implement Display for RecoveryCircuitState` exists
(`fmt → write_str("closed"/"open"/"half-open")`). The Debug form
`f"{x:?}"` → `"RecoveryCircuitState.Closed"` works (verified green:
`rcv_it_circuit_state_debug`). So `f"{x}"` → `format_display(&x)` is not
dispatching the user Display impl for this enum under `--interp`. Pinned
`@ignore` (`rcv_it_circuit_state_display`).

**Scope (now characterised against `config`):** the gap is specific to
**nullary** enum variants. `config.RuntimeIoError.Other(42)` (payload
variant) Displays correctly as `"I/O error (code 42)"`, while the nullary
`RuntimeIoError.WouldBlock` / `RecoveryCircuitState.Closed` fall through.
`format_display` loses the Display impl on the bare-tag (no heap object /
no `type_id`) representation of a nullary variant; `format_debug` does not.
See `core-tests/runtime/config/audit.md §H` for the downstream functional
consequence (breaks `is_transient_error` on nullary `RuntimeIoError`).

**Fix surface (compiler, needs rebuild):** `format_display` enum dispatch
in the VBC interpreter / `safe_interpolation.rs` lowering.

## Action items landed in this branch

* `core-tests/runtime/recovery/property_test.vr` — 20 algebraic-law tests
  (backoff Fixed/None/Linear/Exponential schedules, jitter bounds,
  is_transient_error classifier, RecoveryCircuitState Eq). **All GREEN.**
* `core-tests/runtime/recovery/integration_test.vr` — 20 cross-method tests
  (strategy factories/Composed/Clone, CircuitBreakerError, Inline defaults,
  collections). 13 GREEN; 7 `@ignore` on §F/§G/§H.
* `core-tests/runtime/recovery/unit_test.vr` — 30 ADT tests; 4 newly
  `@ignore`d on §F regression.
* `core-tests/runtime/recovery/audit.md` — this file (§F/§G/§H added).

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
