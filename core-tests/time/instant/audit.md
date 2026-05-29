# `core.time.instant` — audit findings

> Module under test: `core/time/instant.vr` (310 LOC; the `Instant`
> record type holding a single `nanos: Int` field for monotonic
> nanoseconds since an arbitrary epoch, plus
> `Instant.now`/`elapsed`/`duration_since`/`saturating_duration_since`/
> `checked_add`/`checked_sub`/`as_nanos` + Add/Sub/Eq/Ord/PartialOrd/
> Clone/Copy/Hash/Debug/Display protocol impls).
>
> Test surfaces (this branch):
> `unit_test.vr` (141 LOC, 13 `@test`s),
> `property_test.vr` (106 LOC, 9 `@test`s),
> `integration_test.vr` (104 LOC, 7 `@test`s).

## 1. Cross-stdlib usage

`Instant` is the canonical monotonic point-in-time. It is the
reference clock for every elapsed-time measurement.

| Consumer | Use |
|---|---|
| `core.time.interval.{Interval,AsyncInterval}` | `Interval.new` captures `Time.monotonic()` (raw `Int` form of the same monotonic counter) and computes the next-tick deadline from it; `AsyncInterval.poll_next` reads `Time.monotonic()` to test deadline expiry. |
| `core.async.executor` / `core.async.timer` | `sleep_until(deadline: Instant)`, `Deadline { instant }` |
| `core.cog.manifest` (build-time profiling) | Elapsed-time measurement around compilation phases |
| `core-tests/base/memory/cbgr_test.vr` | Benchmark CBGR-validation latency vs. the 15ns production-target via `Instant.now()` deltas |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `monotonic_nanos()` intrinsic | Per-platform `CLOCK_MONOTONIC` / `mach_absolute_time` / `QueryPerformanceCounter` lowering | The intrinsic is implemented in `core/intrinsics/runtime/time.vr` and ultimately calls `sys/{linux,darwin,windows}/time.monotonic_nanos`. Drift between the platform syscall numbers and the intrinsic body would silently de-monotonise the clock. |
| Single `nanos: Int` field (8 bytes) | Layout invariant | Codegen treats `Instant` as a 1-field record; LLVM lowering emits an i64 with a 1-field type-info table. Drift = wrong-offset reads. |
| `implement IntCoercible for Instant` / `implement SizedNumeric for Instant` | Cross-coercion lattice membership alongside `Duration` and `Int64` | Drift breaks `Instant - Instant → Duration` (the canonical elapsed-time idiom). |

## 3. Language-implementation gaps

### §3.1 `Instant.elapsed()` bug-fix carry-over verification

`Instant.elapsed()` carries an inline comment at
`core/time/instant.vr:138-146` flagging a historical bug: the previous
implementation was `self.duration_since(now)` which always computed
`self - now` (always negative for a past instant, returning None,
collapsing to `Duration.zero()` via `unwrap_or`). The current shape
is `now.duration_since(*self).unwrap_or(Duration.zero())`. The
current `test_elapsed_is_non_negative` pin asserts the post-fix
contract (`elapsed >= 0`), but does NOT pin the directional flip:
calling `start.elapsed()` strictly after `start = Instant.now()`
should yield a Duration strictly larger than zero (assuming the
monotonic clock advances at all between the two calls — which it
does on every supported platform).

Tighter pin: add `test_elapsed_after_sleep_is_positive` that calls
`Time.sleep_ms(1)` between `start = Instant.now()` and `start.elapsed()`,
then asserts `elapsed.as_millis() >= 1`. Pinned in
`integration_test.vr` Section 3 implicitly (the timer-sleep test
sequence covers it indirectly), but an explicit `elapsed`-named
pin would harden against future re-introductions of the inverted
direction.

**Effort:** trivial (~10 min).

### §3.2 `checked_add` overflow detection sign-flip relies on Int wrap

`Instant.checked_add(d)` computes `let res = self.nanos + d.as_nanos();`
and tests `res < self.nanos` as the overflow signal. This relies on
2's-complement signed overflow wrapping below the starting value.
For `Int = Int64` this is safe. For `Int.max_value()` + any positive
duration, the wrap is mandated by Verum's signed-integer semantics
(see `core.math.checked.CheckedResult`). But the test surface does
not exercise the `Int.max_value()` boundary (would require a
constructed `Instant { nanos: Int.max_value() }`, which is private —
the only way to construct an `Instant` is `Instant.now()`).

**Workaround:** the bench in `core-tests/base/memory/cbgr_test.vr`
exercises `Instant.now() + Duration.secs(very-large)` and would
surface a regression. No dedicated overflow pin needed in the
`time/instant/` suite.

### §3.3 `Display` rendering format coverage

`implement Display for Instant` at `core/time/instant.vr:303-309`
renders as `f"{secs}.{subsec_ms:03}s"` — a human-readable
seconds-with-ms-padded form. No test currently asserts the rendered
text. Add `test_display_formats_as_seconds_dot_millis` pin.

**Effort:** trivial (~5 min).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.instant` | `core-tests/time/instant/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/instant/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | `elapsed_after_sleep_is_positive` directional pin | 10 min | **CLOSED 2026-05-27** — `test_elapsed_after_sleep_is_positive` |
| §B | Display rendering text assertion | 5 min | **CLOSED 2026-05-27** — `test_display_formats_as_seconds_dot_millis` + `test_display_zero_padded_millis` |
| §C | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |

## 6. Status

**REGRESSED — NOT stable.** See §7. The pre-2026-05-29 "stable under
`--interp`" claim no longer holds: 8 of 19 `duration_since`-path tests
fail under `--interp` against the binary built 2026-05-29 11:29.

## 7. §D — `Instant.duration_since` returns `Int` instead of `Maybe<Duration>` (precompiled-archive collapse)

**Severity: critical (kernel-soundness — Tier-0 archive body returns
wrong runtime type).** Surfaced 2026-05-29.

### Manifestation

Every test that reads the result of `Instant.duration_since(...)` or
`Instant.saturating_duration_since(...)` (which calls `duration_since`
internally) panics at the *use* site:

```
Panic: method 'Maybe.unwrap_or' not found on receiver of runtime kind `Int`.
   - Maybe.unwrap_or (arity 2) …
```

i.e. `duration_since` — declared `-> Maybe<Duration>` — returns a bare
`Int` at runtime. The `Maybe.Some(Duration.from_nanos(self.nanos -
earlier.nanos))` branch collapses to its (intrinsic-elided) `Int`
payload; the `Maybe` variant wrapper is lost.

### Confirmed failing tests (8/19 on the `duration_since` path)

`unit_test::test_duration_since_self_is_zero`,
`test_duration_since_later_is_some`,
`test_saturating_duration_since_floors_at_zero`,
`test_duration_since_epoch_consistency`;
`property_test::law_duration_since_none_when_earlier_is_later`,
`law_duration_since_some_when_later`,
`law_sub_operator_equals_duration_since`,
`law_duration_since_epoch_consistency`.

(The SystemTime `duration_since` — which returns `Result<Duration,
SystemTimeError>` and is *statically* dispatched — passes; the defect
is specific to the `Maybe`-returning Instant path.)

### Root-cause narrowing (verified)

| Probe | Result |
|---|---|
| Real `Instant.duration_since(a).unwrap_or(z)` via standalone `verum run --interp` | **FAIL** (`unwrap_or on Int`) — uses the **precompiled stdlib archive** body |
| User-code mirror `Pt.dsince` (two single-field-record params, `Some(Duration.from_nanos(self.nanos - earlier.nanos))`, `verum run --interp`) | **PASS** — fresh-compiled body |
| User-code `Sfr.via_bare` / `Sfr.via_qual` (`Some` vs `Maybe.Some`, `self.field` payload, branch, test harness) | **both PASS** — fresh-compiled |

⇒ The collapse is **not** in the source pattern (a freshly-compiled,
structurally-identical body returns `Maybe` correctly). It is in the
**precompiled-archive body of `duration_since`** (the
`target/precompiled-stdlib/runtime.vbca` regenerated by the
2026-05-29 11:29 build), or in how that archived body's
`Maybe<Duration>` return is generated/loaded. The `--build`-path VBC
dump of a fresh `duration_since` is correct (`MakeVariantTyped tag=1,
field_count=1` + `SET_VDATA`), so this is an **archive-generation /
archive-load** defect, not a source-shape defect.

### Cross-tier note (AOT)

The `--aot` test path is independently non-functional for these
tests: (a) `verum_types` rejects qualified `Maybe.Some(payload)` as
"no method named `Some`" (the canonical bare `Some(...)` is accepted),
and (b) the `--aot` test runner recompiles the whole stdlib from
source and aborts on unrelated codegen errors (`undefined function:
pointer_parse`, `wrong number of arguments for raw_read` /
`create_thread_tls`, `undefined variable: alg`). So "must pass in
interp AND aot" is currently structurally unmeetable for `time` at the
infrastructure level. Tracked separately.

### Fix direction (fundamental)

1. **Archive path** — ensure `Maybe.Some(<intrinsic-elided payload>)`
   in a precompiled `core` body emits + survives as a proper
   `MakeVariantTyped`, and that the archived body's `Maybe<Duration>`
   return type is preserved through `merge_archive_function_bodies` /
   `type_name_to_id` propagation (same family as the repeatedly-gated
   cross-module record-return work; cf. `585728904` revert).
2. **Regression check** — the binary built 2026-05-29 11:29 is the
   first to expose this; bisect across the 2026-05-29 codegen commits
   (`130b2fca4` shadow-filter, `49eb73b5c`) vs. a stale-archive
   false-green to determine regression-vs-latent before forward-fixing.

**Effort:** multi-day VBC archive-codegen work (gated). Do **not**
mark instant stable until §D closes and the 8 tests are GREEN under
both tiers.
