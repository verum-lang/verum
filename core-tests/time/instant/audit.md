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

**PARTIAL — §D core CLOSED, §E residual open.** The `duration_since`
`Maybe<Duration>` collapse (§D) is fixed (commit `9f64335a5`):
`duration_since` suite **13/19 GREEN** under `--interp` (was 11/19).
The remaining 6 are blocked by §E (`Instant + Duration` operator on the
unboxed-`Int` Instant receiver — a distinct single-field-record defect),
not by the Maybe collapse. See §7. (AOT remains structurally
non-functional — see Cross-tier note.)

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
**precompiled-archive body of `duration_since`**.

### Disproven hypothesis: bare `Some` vs qualified `Maybe.Some` (tested 2026-05-29)

Commit `e8e993dc9` had rewritten `duration_since`'s bare `Some(...)`/
`None` to qualified `Maybe.Some(...)`/`Maybe.None`. Hypothesis: the
qualified form triggers a legacy `MakeVariant` fallback. **Tested and
REJECTED:**

1. Reverted `instant.vr` qualified `Maybe` → bare; full `cargo build`
   refresh of `runtime.vbca`; re-ran the suite → **still 8/19 FAIL**
   (identical set).
2. `verum build --emit-vbc` full-core dump of the bare `duration_since`
   emits **identical bytecode** to the qualified one:
   `MK_VAR tag=1, fields=1` (the *legacy* `MakeVariant`, synthetic
   `0x8000+tag` id) — **NOT** `MakeVariantTyped`.

So in the **full-core** compile both spellings emit legacy
`MakeVariant`; only **small single-module** compiles emit
`MakeVariantTyped` (e.g. a user `Sfr` mirror got `type_id=515`). Per
`emit_make_variant` (`codegen/expressions.rs:473`), the typed form is
gated on the parent descriptor having **non-empty `variants`**; in the
full-core precompile the `Maybe` descriptor is still a Pass-1.5
placeholder (empty variants) when the `time` module body compiles, so
it demotes to legacy.

### The actual two-part defect

**Earlier theories (legacy-`MakeVariant` emit; archive return-unboxing)
were both refuted by `main()` bytecode disassembly** — see Resolution.

### CONFIRMED ROOT CAUSE + RESOLUTION (2026-05-29, commit `9f64335a5`)

The `main()`/caller disassembly (`verum build --emit-vbc`) showed
`a.duration_since(a)` is **not called at all** — it was lowered inline
to a bare `Sub` (integer subtraction → `Int`). Cause: `Instant.
duration_since` is registered as an intrinsic (`register_stdlib_intrinsics`,
`codegen/mod.rs`) backed by `time_instant_duration_since` →
`InlineSequence(InstantDurationSince)`, which emitted **only**
`self.nanos - earlier.nanos`. That drops both the `Maybe` wrapper and
the `earlier > self → None` semantics, so the result is a raw `Int`
("`unwrap_or` not found on receiver of runtime kind Int"; `match` reads
it as `None`). `Instant` is a single-field `{nanos:Int}` record carried
**unboxed** (the generic `add`/`sub` intercepts make `Instant ± Duration`
integer arithmetic), so the intercept can't simply be removed.

**Fix (landed):** rewrote the `InstantDurationSince` inline sequence to
build the canonical typed `Maybe<Duration>`:
`diff = self.nanos - earlier.nanos; if diff >= 0 →
MakeVariantTyped(MAYBE, Some=tag1, 1 field=diff) else
MakeVariantTyped(MAYBE, None=tag0)`. Plus a defence-in-depth
`emit_make_variant` fast-path that force-emits `MakeVariantTyped` for the
canonical core ADTs (Maybe=515, Result=516) on their exact variant
shapes (closes the legacy-synthetic-`MakeVariant` demotion class for
called Maybe/Result-returning fns). Verified regression-free (22/23
`unwrap_or` tests across maybe/result/poll GREEN; the 1 fail is a
pre-existing `@property`-discovery harness bug, not Maybe emission).

**Result:** `duration_since` suite **11/19 → 13/19**.
`test_duration_since_self_is_zero` + `test_duration_since_later_is_some`
now pass; `match`/`unwrap`/`unwrap_or`/`is_some`/`is_none` on the result
all work.

### §E — RESIDUAL: `Instant + Duration` operator on unboxed Instant (6 tests still RED)

The remaining 6 failures (`law_duration_since_some_when_later`,
`law_duration_since_none_when_earlier_is_later`,
`law_sub_operator_equals_duration_since`,
`test_saturating_duration_since_floors_at_zero`,
`law_/test_duration_since_epoch_consistency`) all construct Instants via
`b = a + Duration.…` and then fail BEFORE reaching `duration_since` with
`method 'Instant.add' not found on receiver of runtime kind Int`.
`compile_binary` classifies `Instant` as non-primitive → routes `+`/`-`
through Add/Sub-protocol dispatch (`CallM Instant.add`), which can't
resolve on the unboxed-`Int` receiver. This is a **distinct §G
single-field-record arithmetic-consistency defect**, not the Maybe
collapse. Fix direction: classify the unboxed time types
(`Instant`/`Duration`/`Stopwatch`/`PerfCounter`/`DeadlineTimer`) as
integer-arithmetic in `compile_binary` so `+`/`-`/comparisons lower to
`BinaryI`/`CmpI` — but this bypasses Duration's existing `add`/`sub`
intercepts and needs a full Duration+Instant suite verification (hours).
Tracked as a separate task.

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

### Fix status

§D core (`duration_since` returning `Int`) is **CLOSED** — see
*Confirmed root cause + resolution* above (commit `9f64335a5`). The
`emit_make_variant` canonical-core-ADT fast-path also closes the
legacy-synthetic-`MakeVariant` demotion class for *called*
Maybe/Result-returning functions (defence-in-depth). §E (`Instant +
Duration` operator on the unboxed-`Int` receiver) is the open residual
blocking the last 6 tests — tracked separately.

**Verification note:** each cycle is a ~12–22 min `cargo build` (archive
precompile + relink), and a concurrent session was observed editing
`core/` + relinking `target/release/verum` mid-test (killed a run, exit
144). Use a copied stable binary (`cp target/release/verum
/tmp/verum-fixN`) to run tests immune to relinks.

**Effort (residual §E):** multi-day VBC single-field-record
arithmetic-consistency work (gated). Do **not**
mark instant stable until §D closes and the 8 tests are GREEN under
both tiers.
