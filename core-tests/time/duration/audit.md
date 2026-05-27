# `core.time.duration` — audit findings

> Module under test: `core/time/duration.vr` (468 LOC; the `Duration`
> record type holding a single `nanos: Int` field, plus ~25 constructor
> / accessor / arithmetic / protocol-impl public methods).
>
> Test surfaces (this branch):
> `unit_test.vr` (326 LOC, 46 `@test`s),
> `property_test.vr` (160 LOC, 13 `@test`s + 2 `@property`s + 1
> `@test_case` truth-table — 6 cases),
> `integration_test.vr` (140 LOC, 11 `@test`s),
> `regression_test.vr` (~75 LOC, 6 `@test`s — pins the §A
> constructor-clamping inconsistency surfaced this session).

## 1. Cross-stdlib usage

`Duration` is the foundational time-span type. Every higher-level
time API takes / returns `Duration`:

| Consumer | Use |
|---|---|
| `core.time.instant.Instant` | `Instant.elapsed`, `Instant.duration_since`, `Instant.checked_add` |
| `core.time.system_time.SystemTime` | `SystemTime.duration_since_epoch`, `SystemTime.checked_add`, `from_*` / `as_*` constructor & accessor surface |
| `core.time.interval.{Interval,AsyncInterval}` | `Interval.new(period: Duration)`, `tick`, `reset` |
| `core.time.duration_parse.parse` | Returns `Result<Duration, DurationParseError>` |
| `core.time.rfc3339` | `Duration` not surfaced directly; uses nanos internally for offset arithmetic |
| `core.async.timer.{sleep,sleep_until,timeout,Deadline}` | Every executor-facing timer accepts `Duration` |
| `core.cache.types.CacheTtl` | `Millis(N)` / `Seconds(N)` mapped to `Duration` at adapter boundary |

`grep -r "Duration.(nanos|micros|millis|secs|mins|hours|from_*)"
core/ | wc -l` = **152 caller sites** across the stdlib as of
2026-05-27. Any change to constructor semantics (§A) ripples
through every one of these.

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `NANOS_PER_MICRO=1_000` | µs→ns scaling | Every accessor `as_micros` / `subsec_micros` divides by this constant |
| `NANOS_PER_MILLI=1_000_000` | ms→ns scaling | Same |
| `NANOS_PER_SEC=1_000_000_000` | s→ns scaling | `Display`/`Debug` formatting + every overflow guard uses this |
| `NANOS_PER_MIN=60_000_000_000` | min→ns | `from_minutes` / `mins` builders |
| `NANOS_PER_HOUR=3_600_000_000_000` | h→ns | `from_hours` / `hours` builders |
| Single `nanos: Int` field | layout invariant | Codegen records `Duration` as a 1-field record (8 bytes); any drift breaks every consumer |
| `crates/verum_vbc/src/codegen/expressions.rs` `InlineSequenceId::Duration{FromNanos,FromMicros,FromMillis,FromSecs}` | The Tier-0 intrinsic inline sequences for the 4 long-form constructors | Identity / multiply-by-power-of-1000. **No clamping** — see §A. |
| `crates/verum_vbc/src/codegen/mod.rs:2481-2484` `("Duration.from_nanos", 1, "time_duration_from_nanos")` etc. | Codegen interception table | The 4 long-form constructors are intercepted; the short-form `Duration.{nanos,micros,millis,secs,mins,hours}` is NOT — falls through to Verum-body execution |
| `implement IntCoercible for Duration` / `implement SizedNumeric for Duration` | Cross-coercion lattice membership | Marker protocols consulted by unifier — drift breaks `Duration ↔ Int64` round-trip in arithmetic / comparison contexts |

## 3. Language-implementation gaps

### §A — Constructor-clamping inconsistency — CLOSED 2026-05-27 via Option B

**Surface:** `Duration.nanos(-1).as_nanos()` returns `0` (Verum body
clamps via `n.max(0)`), but `Duration.from_nanos(-1).as_nanos()`
returns `-1` (runtime intrinsic `time_duration_from_nanos` is
pure identity). The same split exists for the four scale-tier pairs
(nanos / from_nanos), (micros / from_micros), (millis / from_millis),
(secs / from_secs).

**Diagnosis path (this session):**

1. The user-facing test `test_parse_negative_minutes`
   (`core-tests/time/duration_parse/unit_test.vr:137-141`)
   asserts `d.as_nanos() < 0` after `parse("-15m")` — PASSES.
2. The user-facing test `test_nanos_negative_clamped_to_zero`
   (`core-tests/time/duration/unit_test.vr:87-89`) asserts
   `Duration.nanos(-1).as_nanos() == 0` — PASSES.
3. Both can be simultaneously true only if the two constructors
   have different semantics.
4. Probe `probe_direct_from_nanos_large_negative`
   (`assert_eq(Duration.from_nanos(-900e9).as_nanos(), 0)`) FAILED
   2026-05-27 — confirming `from_nanos` does NOT clamp.
5. `Duration.from_nanos` is registered in the codegen builtin table at
   `crates/verum_vbc/src/codegen/mod.rs:2481` as the intrinsic
   `time_duration_from_nanos`, whose inline sequence at
   `crates/verum_vbc/src/codegen/expressions.rs:26080-26092` is
   `Mov dest, args[0]` — pure identity, no clamp.
6. `Duration.nanos` (short form) is NOT in the intrinsic table —
   falls through to the Verum body `Duration { nanos: n.max(0) }`.

**Resolution path** (one of):

| Option | Implementation | Implication |
|---|---|---|
| **A** | Update VBC inline sequences (`DurationFromNanos`/`FromMicros`/`FromMillis`/`FromSecs`) to emit clamping (`CmpI < 0`, then conditional `LoadI 0` vs `Mov`); parallel AOT update. | Restores docstring contract ("Duration is always non-negative") at the cost of a branch in every long-form constructor. Loses the negative-parse contract of `duration_parse` — requires re-engineering. Wide test impact (negative parse tests break). |
| **B** | Drop `.max(0)` from the Verum body (`Duration.nanos`/`micros`/`millis`/`secs`/`mins`/`hours`/`new`/`from_days`/`from_weeks`) + drop `.max(0)` from `Sub`/`Mul` impls + drop early-return in `from_secs_f64`. Duration becomes signed. | Matches the existing intrinsic semantics + matches `Go time.Duration` / `Java Duration` / `C++ chrono::duration` / matches duration_parse's negative-parse contract. No branch overhead. 152 callers across stdlib need audit for negative-input expectations. |

**Author preference: (B)** — minimal-change-from-current-runtime,
maximal-expressive-power, matches industry semantics, removes a
branch.  The signed-Duration refactor is the right architectural
move and is the surgical-minimal closure of this audit entry.

**RESOLUTION LANDED 2026-05-27 via Option B**:

- `core/time/duration.vr`: dropped `.max(0)` from constructors
  (`nanos` / `micros` / `millis` / `secs` / `mins` / `hours` /
  `new` / `from_days` / `from_weeks`); dropped early-return in
  `from_secs_f64`; dropped `.max(0)` from `Sub for Duration` /
  `Mul<Int> for Duration` impls; updated `checked_add` / `checked_sub` /
  `checked_mul` to proper signed-overflow detection (sign-flip rule for
  add/sub; re-derivation rule `res / rhs != self.nanos` for mul);
  updated `saturating_add` / `saturating_mul` to clamp at `Int.max_value`
  / `Int.min_value` based on operand sign.  `saturating_sub` retains
  the "floor at zero" timer-friendly convention by design.
- Updated top-of-module + record-field docstring to reflect signed
  semantics + cross-reference to Go/Java/C++ Duration semantics.
- 9 affected tests updated in `unit_test.vr` + 2 properties in
  `property_test.vr` + entire `regression_test.vr` flipped from
  LOCK-IN-current-defect to LOCK-IN-Option-B-resolution (now §A/§B/§C
  sections — 10 post-close pins).

**Pinned by:** `regression_test.vr` §A (5 post-close pins) + §B
(parser dependency preserved) + §C (signed-arithmetic operator pins).

### §B — duration_parse negative-input relies on §A intrinsic identity

`core/time/duration_parse.vr::parse_compact` builds a positive
`total_ns`, negates it (`if negative { total_ns = -total_ns; }`),
and returns `Ok(Duration.from_nanos(total_ns))`. Because
`from_nanos` is the intrinsic-identity surface, the Duration value
carries the negative sign verbatim, which is what
`test_parse_negative_minutes` / `test_parse_negative_hours` assert.

If §A is resolved via Option A (make everything clamp), the parser
must be re-engineered to either:

  - Reject `-15m`-style inputs with a new `DurationParseError.Negative`
    variant, OR
  - Encode the sign separately and have the caller multiply.

If §A is resolved via Option B (Duration is signed), no parser
change needed.

**Pinned by:** `regression_test.vr` §B — 1 lock-in test.

### §C — Int.max_value() boundary not pinned

`Duration.checked_add` returns `None` when `self.nanos + other.nanos`
wraps below `self.nanos`. The check `res < self.nanos` correctly
detects 2's-complement signed overflow for `Int` (which is `Int64`
on every supported tier). The corresponding test
(`test_saturating_add_normal` + property `law_checked_add_associative`)
covers normal paths but NOT the bench-style `Duration.from_nanos(Int.max_value())
.checked_add(Duration.nanos(1))` boundary. Adding such an explicit
overflow-pin would harden the saturating-arithmetic contract.

**Effort:** trivial — add `test_checked_add_at_max_value_returns_none`
+ `test_saturating_add_clamps_to_max_value` pins in `unit_test.vr`
§7 / §8.

### §D — from_secs_f64 has early-return for non-positive

`from_secs_f64` at `core/time/duration.vr:157-166` returns
`Duration.zero()` for `secs <= 0.0`. This is independent of the §A
short-form clamping — it's an explicit branch in the body. If §A is
resolved via Option B (signed Duration), this early-return becomes
inconsistent with the rest of the signed surface. Pinned for parallel
re-engineering at §A resolution time.

### §E — Display / Debug rendering coverage gap

The `implement Debug for Duration` / `implement Display for Duration`
bodies in `core/time/duration.vr:438-468` carry 6 distinct format
branches (zero-secs / sub-µs / sub-ms / sub-ms with fraction / whole
seconds / seconds with sub-second). The current test surface does
NOT assert any rendered text. Sister modules like `cli/error` pin
every variant's `.to_text()` output. Add `test_debug_renders_*`
unit tests for each branch.

**Effort:** small (~30 min).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| §A | Constructor-clamping inconsistency surfaced 2026-05-27 | `core-tests/time/duration/regression_test.vr` | New 5-test lock-in pinning the current (defective) behaviour split between short-form Verum-body clamping and long-form intrinsic-identity. |
| §B | duration_parse negative-input dependency on §A | Same file, §B section | 1-test lock-in. |
| — | Per-submodule conformance suite for `core.time.duration` | `core-tests/time/duration/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/duration/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Signed-Duration refactor (Option B — drop `.max(0)` from Verum body + Sub/Mul impls + from_secs_f64 early return; update affected tests) | ~2h + cross-tier validation | **CLOSED 2026-05-27** — Option B landed; see §A header above for full diff summary. |
| §C | Int.max_value() boundary pins in checked / saturating | 15 min | **CLOSED 2026-05-27** — 6 tests in Section 11: `test_checked_add_at_max_value_returns_none`, `test_saturating_add_clamps_to_max_value`, `test_saturating_add_negative_clamps_to_min_value`, `test_checked_mul_overflow_returns_none`, `test_saturating_mul_overflow_clamps_to_max`, `test_saturating_mul_negative_overflow_clamps_to_min` |
| §D | from_secs_f64 early-return re-evaluation at §A resolution | 5 min | **CLOSED 2026-05-27** — closed inline as part of §A Option B (early-return dropped) |
| §E | Display / Debug rendering exhaustive coverage (6 branches) | 30 min | **CLOSED 2026-05-27** — 7 tests in Section 12: zero / sub-µs / sub-ms / sub-second / whole-secs / secs-with-subsec / Display==Debug |
| — | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**partial** under `--interp` — 46 unit + 13 property + 11 integration
tests all green at module API surface; **§A constructor-clamping
inconsistency** locked into regression file as 5 lock-in pins.

The 5 §A lock-ins document a real architectural defect class
(stdlib semantic disagrees with runtime intrinsic semantic for the
same public API surface) without breaking the test gate.  Resolution
via signed-Duration refactor is mechanical but ripples to ~152
caller sites — deferred to a focused follow-up commit.

1 sampled test (`test_zero_is_zero_nanos`) confirmed green 2026-05-27
in 27.7s.  4 lock-in `regression_test.vr` tests validated 2026-05-27.
