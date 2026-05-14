# `core/async/timer.vr` — audit

Async time-based primitives: `Sleep`, `SleepUntil`, `TimerInterval`,
`Delay<F>`, `Timeout<F>`, `TimeoutError`, `Debounce`, `Throttle`,
plus the constructor factories.

## Public API surface

| Name | Shape | Status under interpreter |
|---|---|---|
| `Sleep` (record `{ deadline: Maybe<Instant>, duration: Duration }`) + `Sleep.new`, `sleep`, `sleep_secs` | one-shot deferred Future | green (4 unit, 1 property) outside `sleep(Duration.from_millis(N))` (task #15) |
| `sleep_ms`, `sleep`-with-`from_millis` | from_millis-dependent | **blocked by task #15** |
| `SleepUntil` (record `{ deadline: Instant }`) + `SleepUntil.new`, `sleep_until` | absolute-deadline deferred Future | green (2 unit) |
| `TimerInterval` (record `{ period: Duration, next_tick: Maybe<Instant> }`) + `.new`, `.immediate`, `.reset` | repeating tick generator | green (4 unit, 1 property) outside `.period()` getter (task #17) |
| `TimerInterval.period()` (getter method) | field-name-shadowed accessor | **blocked by task #17** (stack-overflow) |
| `Delay<F>.new`, `delay` factory | delayed-first-poll wrapper | green (2 unit) |
| `Timeout<F>.new`, `timeout` factory | first-arm-wins wrapper | **blocked by task #16** (field-layout write out of bounds) |
| `timeout_ms` | ms-convenience factory | **blocked by task #14** (cross-module name collision) |
| `TimeoutError` (unit type `()`) | timeout indicator | green (2 unit, 1 property) |
| `Debounce` (record `{ delay: Duration, deadline: Maybe<Instant> }`) + `.new`, `.trigger`, `.reset`, `.is_settled` | input-debounce state machine | green (3 unit) |
| `Throttle` (record `{ interval: Duration, last_allowed: Maybe<Instant> }`) + `.new`, `.try_acquire`, `.reset` | rate-limiter state machine | green (3 unit, 2 property) |

## Cross-stdlib usage

* `core.time.{Duration, Instant}` — every constructor consumes one.
  Task #15 surfaces when constructing through `Duration.from_millis`.
* `core.async.future.Future` — `Delay<F>`, `Timeout<F>` wrap inner
  futures.
* `core.async.executor.current_runtime()` — Sleep / SleepUntil poll
  paths register timers when a runtime is in scope.

## Crate-side hardcodes

None observed.

## Language-implementation gaps

1. **Task #14 — `timeout_ms` cross-module name collision.** Selective
   mount fails to disambiguate from same-named symbols in net.dns,
   runtime.supervisor, meta.contexts. Compiler routes to a wrong
   1-arg overload.
2. **Task #15 — `Duration.from_millis` dispatch routes `from_nanos`
   to an Int receiver.** Repro at `sleep(Duration.from_millis(N))`.
3. **Task #16 — `Timeout<F>.new` field-write out of bounds.**
   `Timeout` has 3 declared fields; codegen writes to field index 5
   (off by 2). Related to task #9's field-layout cross-mount race
   pattern.
4. **Task #17 — `TimerInterval.period()` stack-overflow.** Method
   name `period` shadows the same-named field; `self.period` in the
   getter dispatches as `self.period()` recursively.
5. **Pre-fix landed**: `core/async/timer.vr:535` had `pub async fn
   acquire` (`pub` is not a Verum keyword — grammar/verum.ebnf:393
   permits `public|internal|protected`). Corrected to
   `public async fn acquire`.
6. **Task #10 — AOT generate_native SIGABRT** (global blocker).

## Action items landed in this branch

* Created `core-tests/async/timer/{unit,property,integration,regression}_test.vr,audit.md`.
* 29 tests under interpreter — all green; 6 `@ignore` regression
  pins for tasks #14 + #15 + #16 + #17.
* Pre-fix: `pub async fn acquire` → `public async fn acquire` at
  `core/async/timer.vr:535`.
* Pinned: Sleep / SleepUntil / Delay construction surface,
  TimerInterval next_tick partition (new vs immediate), Debounce
  trigger/reset round-trip, Throttle monotonic refusal +
  reset-then-acquire round-trip across 4 representative interval
  sizes, TimeoutError Eq reflexivity over List of 3 inhabitants.

## Action items deferred

* Tasks #10 + #14 + #15 + #16 + #17.
* Full runtime poll path coverage (Sleep deadline triggered → Ready,
  Delay first-poll-after-delay-elapsed, Timeout first-arm-wins
  semantics) deferred pending executor test-bed.
