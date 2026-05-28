# `net/proxy/retry` audit

Module: `core/net/proxy/retry.vr` (~283 LOC) — RetryLayer with
exponential backoff + budgets + idempotency-key gating.

* `RetryBudget` — global shared retry counter; `try_consume()`
  decrements and clamps at 0.
* `RetryLayer` — record holding `max_attempts`, `backoff_base_ms`
  (default 50), `backoff_max_ms` (default 5_000), optional
  `RetryBudget`, and `NonIdempotentPolicy` (defaults to Disabled).
* `NonIdempotentPolicy` — `Disabled` | `WhenIdempotencyKeyed
  { header_name: Text }`.
* Backoff formula: `2^attempt × backoff_base_ms` capped at
  `backoff_max_ms` (lines 162-167).

The middleware surface (`wrap`, `RetryHandler.handle`) requires
a `WeftRequest` + inner `Handler` mock; @ignore'd until harness
lands.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.proxy` reverse proxy | wraps upstream handler in RetryLayer. |
| `core.net.weft` middleware stacks | `.layer(RetryLayer.new(3))`. |
| HTTP client | per-call retry of idempotent methods. |

## 2. Crate-side hardcodes

None — pure Verum. `AtomicInt` / `MemoryOrdering` stdlib-side.

## 3. Language-implementation gaps

### §3.1 RT-1 — Backoff formula constants

Lines 109-112 of retry.vr — defaults: `backoff_base_ms: 50`,
`backoff_max_ms: 5_000`. The formula `pow2(attempt) ×
backoff_base_ms` (line 165) at attempts 1, 2, 3, ... yields
100, 200, 400, ... ms; cap fires at attempt 7 (`2^7 × 50 =
6400 > 5_000 → cap to 5_000`). LOCK-IN pinned in unit tests
via `backoff_ms_for` accessor... however `backoff_ms_for` is
module-private (line 162), so the pin is on `RetryLayer.new`
field preservation + the constants table by inspection.

### §3.2 RT-2 — RetryBudget concurrent semantics

`try_consume` does `fetch_sub(1) + 1 → if prev > 0 -> true
else fetch_add(1) → false` (lines 64-74). The `prev > 0`
condition means a budget of N permits exactly N consumes.
LOCK-IN pinned at unit level. Concurrent-budget-exhaustion
@ignore'd until atomic-contention harness.

### §3.3 RT-3 — NonIdempotentPolicy default = Disabled

Line 114 — `RetryLayer.new` sets `non_idempotent_policy:
NonIdempotentPolicy.Disabled`. Pinned via variant `is` check
on the public `non_idempotent_policy` field.

### §3.4 RT-4 — `with_idempotency_key_retry` header_name

Lines 131-133 — `header_name: f"idempotency-key"`. Pin the
canonical RFC 9110 header name.

### §3.5 RT-5 — `pow2` helper is module-private

Lines 170-175 — exponential backoff multiplier helper. Not
callable from a user test, so the formula is pinned indirectly
via the constants table.

## 4. Action items landed in this branch

* `core-tests/net/proxy/retry/unit_test.vr` — 16 unit tests:
  RetryBudget.new + try_consume + refill, RetryLayer.new
  default-constants (50 / 5000 / max_attempts / Disabled),
  RetryLayer builder chain (`with_backoff`, `with_budget`,
  `with_idempotency_key_retry`, `with_idempotency_header`),
  NonIdempotentPolicy 2-variant disjointness.
* `core-tests/net/proxy/retry/regression_test.vr` — 7
  regression pins (3 active LOCK-IN + 4 @ignore'd functional).
* `core-tests/net/proxy/retry/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Backoff formula 2^n × base capped at max | this folder + module-public helper | 2h |
| RetryBudget concurrent consume under contention | this folder + atomic harness | 4h |
| RetryHandler.handle with mock Handler + 502/503/504 retry | this folder + Handler mock | 1 day |
| Non-idempotent method retry gated on header | this folder + WeftRequest mock | 4h |
| ErrorCategory.is_retryable filter | this folder + WeftError fixtures | 4h |
