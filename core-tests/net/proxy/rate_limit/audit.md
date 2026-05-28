# `net/proxy/rate_limit` audit

Module: `core/net/proxy/rate_limit.vr` (~331 LOC) — token-bucket
/ leaky-bucket / sliding-window rate limiters + keyed fan-out.

* `RateDecision` — `Admit` | `NotNow(Duration)`. Admission
  decision; `NotNow` carries minimum wait before next try would
  succeed.
* `TokenBucket` — capacity C, refill rate R → allows bursts up
  to C, enforces long-run rate R.
* `LeakyBucket` — fixed-rate drain through a queue of capacity
  C. Shapes output to exactly R tokens/sec.
* `SlidingWindow` — 2-bucket approximation: live count = prev ×
  (1 - elapsed/window) + current. Constant-time, constant-
  memory.
* `KeyedRateLimiter<K, L>` — per-key fan-out via `LimiterFactory<L>`.
* `CHEAP_COST` (1) / `HEAVY_COST` (10) — convenience cost constants.

The umbrella `core-tests/net/proxy/unit_test.vr` already covers
the cost-constants; this per-submodule suite extends with
RateDecision variant disjointness + bucket-construction +
SlidingWindow-construction.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.proxy` reverse proxy | per-route TokenBucket / SlidingWindow. |
| `core.net.weft` rate-limit middleware | wraps the inner handler. |
| API gateway | KeyedRateLimiter<UserId, TokenBucket> per-user. |

## 2. Crate-side hardcodes

None — pure Verum. Integer-only arithmetic over microseconds.

## 3. Language-implementation gaps

### §3.1 RL-1 — RateDecision payload type-equality requires Duration Eq

`NotNow(Duration)` carries a `Duration`. Eq comparison on
RateDecision relies on Duration's Eq impl. LOCK-IN pinned via
unit test asserting `Admit == Admit`, `NotNow(a) == NotNow(a)`
when underlying Durations match.

### §3.2 RL-2 — try_admit semantics gated on Instant fixture

The `try_admit(&mut self, cost, now: Instant)` surface mutates
internal `tokens` / `queued` / `current` state based on the
provided `now`. Real algorithm semantics — initial-burst
admission, refill drain, sliding-window rollover — require a
controllable `Instant` source. @ignore'd until clock fixture.

### §3.3 RL-3 — KeyedRateLimiter.factory requires Heap<dyn LimiterFactory<L>>

`KeyedRateLimiter.new(factory)` takes `Heap<dyn LimiterFactory<L>>`.
Constructing a concrete `LimiterFactory` impl requires either an
inline closure or a custom type — both are functional surface.
Data-surface tests verify the public field accessors `len()` /
`keys()` post-construction.

### §3.4 RL-4 — Cost constants CHEAP_COST=1, HEAVY_COST=10

Lines 328-330. Already pinned in umbrella unit_test; re-asserted
here for module-local granularity + relative-order invariant.

## 4. Action items landed in this branch

* `core-tests/net/proxy/rate_limit/unit_test.vr` — 16 unit
  tests: RateDecision 2-variant + Admit Eq + NotNow Eq +
  TokenBucket / LeakyBucket / SlidingWindow construction +
  CHEAP_COST / HEAVY_COST + relative-order invariant + boundary
  ctor (zero-capacity, zero-rate, max-capacity).
* `core-tests/net/proxy/rate_limit/regression_test.vr` — 6
  regression pins (3 active LOCK-IN + 3 @ignore'd timing).
* `core-tests/net/proxy/rate_limit/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TokenBucket initial-burst admission | this folder + clock fixture | 2h |
| TokenBucket refill at rate_per_s | this folder + clock fixture | 2h |
| LeakyBucket drain at rate_per_s | this folder + clock fixture | 2h |
| SlidingWindow rollover at window_ms | this folder + clock fixture | 4h |
| KeyedRateLimiter per-key fan-out + first-touch | this folder + Factory impl | 4h |
| Concurrent admit under contention | this folder + async harness | 1 day |
