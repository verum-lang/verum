# Audit — `core/base/retry.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/retry.vr` (218 lines) |
| Tests | NEW — `unit_test.vr` (~90 LOC, options + retry sequences), `property_test.vr` (~90 LOC, max-attempts cap + backoff monotonicity + first-call / exhaustion @property) |
| Hardcodes in `crates/` | clock for delays via `core.time.sleep_ms` runtime intrinsic |

## §1  Backoff strategies

`RetryBackoff` is a sum type:
- `Immediate` — zero delay
- `Linear { base_ms }` — fixed inter-attempt delay
- `Exponential { base_ms, factor }` — geometric growth
- `Jittered { base_ms, max_ms }` — exponential with random jitter

All four implement `delay_for_attempt(attempt)`. The contract:
**delay is monotonic non-decreasing in attempt number** (verified via
property tests for Linear and Exponential; Immediate returns 0
unconditionally).

## §2  Async / cancellation

Today `retry` is synchronous. An async-aware variant would need
context-system integration (the test runner cannot directly await).
Out of scope for this audit.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/retry/`
- [x]  `unit_test.vr` — RetryOptions construction, retry-succeeds-first,
       retry-eventually-succeeds, retry-exhausts, retry_immediate
- [x]  `property_test.vr` — max-attempts cap, exponential / linear /
       immediate monotonicity, first-call @property, exhaustion @property
- [x]  This audit document

## §4  Action items deferred

1. **Total-time bound** — for jittered backoff, total elapsed time
   must be bounded. Pin the contract.
2. **abort-on-permanent-error semantics** — when `should_retry`
   returns false, retry must stop immediately. Test with a
   permanent-error closure.
3. **Async retry** — once context-system test isolation lands.
4. **Negative max_attempts** — invariant: `retry(0, ...)` calls
   zero times and returns... what? Pin the contract.
