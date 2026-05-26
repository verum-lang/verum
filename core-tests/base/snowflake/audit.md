# `core/base/snowflake` — Audit

> Module: `core/base/snowflake.vr` — Twitter-style 64-bit
> time-ordered ID generator. 41-bit timestamp + 10-bit worker +
> 12-bit sequence, fits in `UInt64`.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `Snowflake` | record `{ epoch_ms, worker_id, last_ts_ms, sequence }` (all `UInt64`) | yes |
| `SnowflakeError` | sum `WorkerIdOutOfRange(UInt64) \| ClockRegressed(UInt64) \| ClockBeforeEpoch(UInt64)` | yes |
| `SnowflakeParts` | record `{ timestamp_ms, worker_id, sequence }` (all `UInt64`) | yes |

### 1.2 Free functions / methods

| Item | Signature |
|---|---|
| `Snowflake.new` | `(UInt64, UInt64) -> Result<Snowflake, SnowflakeError>` |
| `Snowflake.using_default_epoch` | `(UInt64) -> Result<Snowflake, SnowflakeError>` |
| `Snowflake.next_id` | `(&mut self) -> Result<UInt64, SnowflakeError>` |
| `Snowflake.worker_id` | `(&self) -> UInt64` |
| `Snowflake.epoch_ms` | `(&self) -> UInt64` |
| `using_default_epoch` (free fn alias) | `(UInt64) -> Result<Snowflake, SnowflakeError>` |
| `with_epoch` (free fn alias) | `(UInt64, UInt64) -> Result<Snowflake, SnowflakeError>` |
| `parse` | `(UInt64, UInt64) -> SnowflakeParts` |

### 1.3 Constants

| Constant | Value | Notes |
|---|---|---|
| `WORKER_BITS` | `10` | bits in the worker_id field |
| `SEQUENCE_BITS` | `12` | bits in the sequence field |
| `MAX_WORKER` | `1023` | `(1 << 10) - 1` |
| `MAX_SEQUENCE` | `4095` | `(1 << 12) - 1` |
| `WORKER_SHIFT` | `12` | low bit of worker_id within UInt64 |
| `TIMESTAMP_SHIFT` | `22` | low bit of timestamp_offset within UInt64 |
| `DEFAULT_EPOCH_MS` | `1_288_834_974_657` | Twitter's 2010-11-04 epoch |

### 1.4 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 24 unit tests | all green under `--interp` |
| `property_test.vr` | 14 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 10 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 6 + 3 `@ignore`'d | 6 green; 3 pinned on §D defect |

## §2 — Findings landed in this branch

### 2.1 `SystemTime.now()` mis-dispatches to `SysTimeOpsInstant.now()`

Every call into `Snowflake.next_id()` routes through `current_unix_ms()`
→ `SystemTime.now().timestamp_millis()`. The static-method dispatcher
mis-routes `SystemTime.now` to `SysTimeOpsInstant.now()` (a sibling
type with a 1-field layout vs `SystemTime`'s 2-field layout). Symptom:

```
field access out of bounds: field index 1 (offset 8+8 = 16)
exceeds object data size 8 type_id=137 type='SysTimeOpsInstant'
backtrace=[SystemTime.timestamp_millis@pc=16
  <- core.base.snowflake.current_unix_ms@pc=20
  <- Snowflake.next_id@pc=6]
```

* Defect class: task #17/#39 — bare-name first-suffix-wins in
  static-method dispatch. Same root as
  `[[task17_static_method_dispatch_defect_2026-05-24]]`.
* Workaround in tests: use synthetic Snowflake IDs (hand-built via
  `(ts_offset << TIMESTAMP_SHIFT) | (worker << WORKER_SHIFT) | seq`)
  to exercise the bit-packing / parse contract end-to-end without
  going through `next_id`.
* Live `next_id` regression pinned at `regression_test.vr §D` as
  `@ignore`'d. Flips green when the dispatcher becomes mount-scope-
  aware (multi-day VBC codegen fix).

### 2.2 integration_test.vr referenced a hallucinated API

Pre-fix integration_test called:

| Pre-fix call | Status |
|---|---|
| `SnowflakeGenerator` mount | Type does not exist |
| `gen.next_id().unwrap().value()` | `next_id` returns `UInt64`, not a wrapper with `.value()` |
| `id.worker_id()` on a `UInt64` | `UInt64` has no `worker_id` method |
| `Snowflake.from_value(raw)` | Method does not exist |

Fix: rewritten integration_test using `parse(id, epoch_ms)` on
synthetic IDs as the bit-layout extraction path; the `Snowflake`
struct itself only exposes `.worker_id()` / `.epoch_ms()` accessors.

### 2.3 property_test.vr depended on live next_id

Pre-fix property tests all called `next_id()` and asserted
monotonicity. Every test hit the §2.1 defect. Fix: rewritten to use
synthetic IDs for round-trip / bit-field independence / monotonicity-
in-timestamp / monotonicity-in-sequence laws. The live-clock
monotonicity property is pinned at `regression_test.vr §D` as
`@ignore`'d.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.snowflake`:

* `core.action.*` and `core.signal.*` — task IDs / event IDs.
* `core.database.*` — primary keys (cited in source as the canonical
  use-case).
* No other `core/` modules reference this layer at present.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. `core-tests/base/snowflake/unit_test.vr` — rewritten end-to-end
   with 24 tests across 6 sections covering Snowflake.new validation,
   layout constants, SnowflakeError variants + Eq matrix, parse on
   synthetic IDs, SnowflakeParts construction, and Snowflake accessors.
2. `core-tests/base/snowflake/property_test.vr` — rewritten without
   live `next_id()` dependency; 14 laws covering parse · build
   identity (worker / sequence / timestamp), bit-field independence,
   synthetic-ID monotonicity, worker-id validation parametrised, and
   the 41+10+12 = 63-bit layout invariant.
3. `core-tests/base/snowflake/integration_test.vr` — rewritten without
   the hallucinated `SnowflakeGenerator` / `.value()` / `.from_value`
   API; 10 scenarios covering multi-worker construction, parse on
   synthetic-ID corpus, Set<UInt64> + worker-bit no-collision, and
   chronological-byte-order invariant.
4. NEW `core-tests/base/snowflake/regression_test.vr` — 9 pins:
     §A layout-constant stability
     §B WorkerIdOutOfRange at exactly MAX_WORKER+1
     §C parse · build round-trip
     §D `@ignore`'d × 3 — `SystemTime.now()` static-method dispatch
        defect (task #17/#39): live `next_id` panics with
        `SysTimeOpsInstant` receiver
     §E default-epoch carries Twitter 2010
     §F SnowflakeError variants pairwise distinct
5. NEW `core-tests/base/snowflake/audit.md` — documents API surface,
   this branch's findings, deferred items, action plan.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| `SystemTime.now()` static-method dispatch close-out | multi-day VBC codegen work | task #17/#39 |
| Live `next_id` monotonicity / sequence-saturation tests | gated on §2.1 | regression_test.vr §D pins |
| `ClockRegressed` / `ClockBeforeEpoch` live trigger tests | gated on §2.1 | future task |
| `Display` / `Debug` impls for SnowflakeError exercised via `f"{...}"` | 30min, once §2.1 closes | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker (`semver_compare undefined`) | task #7 |
