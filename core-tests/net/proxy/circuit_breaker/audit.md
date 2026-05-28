# `net/proxy/circuit_breaker` audit

Module: `core/net/proxy/circuit_breaker.vr` (~182 LOC) — Hystrix-
style three-state circuit breaker layer (Closed / Open / HalfOpen)
with consecutive-failure threshold + cooldown timer. Layered into
`core.net.weft` request pipelines via the `Layer<H>` impl. The
state machine encodes state as `AtomicInt` (STATE_CLOSED=0,
STATE_OPEN=1, STATE_HALF_OPEN=2) — there is no public variant
enum at the data surface, only the public ctor + `current_state`
accessor returning Int.

Tests cover the algebraic data-surface that's reachable from a
user test module:

* `CircuitBreakerLayer.new(failure_threshold, reset_timeout_ms,
  success_threshold)` field preservation.
* `CircuitBreakerLayer.default()` — pins the docstring values
  (5 / 30_000 / 2) against config drift.
* `current_state()` after construction == STATE_CLOSED (0).
* Boundary inputs (zero / negative / large thresholds).

Live state transitions (Closed→Open→HalfOpen→Closed) require a
clock fixture + inner `Handler` mock; that surface is covered at
language level (`vcs/specs/L2-standard/net/proxy/`).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` middleware stacks | `CircuitBreakerLayer.wrap(inner)` per route. |
| `core.net.proxy` reverse proxy | wraps the upstream handler chain. |
| Service mesh / sidecar | per-upstream breaker isolates blast-radius. |

## 2. Crate-side hardcodes

None — pure Verum. `AtomicInt` / `MemoryOrdering` are stdlib-side.

## 3. Language-implementation gaps

### §3.1 CB-1 — State machine encoded as Int, not variant

`STATE_CLOSED` / `STATE_OPEN` / `STATE_HALF_OPEN` are module-
private `const Int` (lines 53-55) not a `type CircuitState is
Closed | Open | HalfOpen` variant. Consumers see `Int` from
`current_state()`. Pattern-match callers must use `==` against the
literal Int, not `is` patterns. Roadmap: expose a public variant
enum once the AtomicInt-as-variant codegen lands.

### §3.2 CB-2 — State transitions gated on clock fixture

`dispatch` reads `Instant.now()` to compute the cooldown delta.
Live transitions (Closed→Open at threshold; Open→HalfOpen after
`reset_timeout_ms`; HalfOpen→Closed on probe success) require
either a mock clock or a real elapsed-time fixture. Functional
surface @ignore'd here, covered at `vcs/specs/L2-standard/`.

### §3.3 CB-3 — `classify_as_failure` private + not user-callable

The 5xx + retryable-category gate is a module-private free
function (line 173). Callers can't inject custom classifiers.
Roadmap: expose `classify: Heap<dyn Classify>` parameter on
`CircuitBreakerLayer.new(...)`.

## 4. Action items landed in this branch

* `core-tests/net/proxy/circuit_breaker/unit_test.vr` — 13 unit
  tests covering `CircuitBreakerLayer.new` field preservation +
  `default()` constants (5 / 30_000 / 2) + `current_state` ==
  STATE_CLOSED after construction + boundary-value ctor.
* `core-tests/net/proxy/circuit_breaker/regression_test.vr` —
  6 regression pins (3 active LOCK-IN + 3 @ignore'd functional).
* `core-tests/net/proxy/circuit_breaker/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Closed→Open transition at failure_threshold | this folder + clock fixture | 4h |
| Open→HalfOpen after reset_timeout_ms | this folder + clock fixture | 2h |
| HalfOpen→Closed on probe success | this folder + Handler mock | 2h |
| HalfOpen→Open on probe failure | this folder + Handler mock | 2h |
| Public `CircuitState` variant enum | stdlib | 1 day, gated on CB-1 codegen |
| Pluggable `Classify` protocol | stdlib | 4h |
