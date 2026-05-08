# Audit — `core/base/snowflake.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/snowflake.vr` (241 lines) |
| Tests | NEW — `unit_test.vr` (~70 LOC), `property_test.vr` (~70 LOC, monotonicity + round-trip + bit-layout) |
| Hardcodes in `crates/` | clock-source via `core.time.now_ms()` runtime intrinsic |

## §1  Bit layout

Standard Twitter Snowflake: 1 sign bit + 41-bit timestamp + 10-bit
machine ID (max 1023) + 12-bit sequence (max 4095, reset per ms).
Verum's source matches this layout per the comments.

## §2  Clock source

`now_ms()` goes through the runtime intrinsic registry. **Drift
risk:** the `epoch_ms` baseline matters — if two services share a
worker_id space but use different epochs, IDs collide. The
`using_default_epoch` constructor uses the standard Twitter epoch
(2010-11-04T01:42:54.657Z); custom epochs are opt-in.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/snowflake/`
- [x]  `unit_test.vr` covering construction with valid params,
       next_id success, sequential monotonicity, parse round-trip
- [x]  `property_test.vr` covering monotonicity within burst,
       worker_id round-trip, parts-combine-to-id, invalid-worker-rejection
- [x]  This audit document

## §4  Action items deferred

1. **Sequence overflow into next ms** — when 4096 IDs are requested
   in the same ms, the generator must spin until the clock advances.
   Currently not tested; race condition prone.
2. **Cross-machine collision check** — two Snowflake generators with
   the same worker_id must not collide. Single-process test inadequate.
3. **Clock-skew handling** — what happens if `now_ms()` returns a
   timestamp earlier than the previous? Behaviour deferred to source.
