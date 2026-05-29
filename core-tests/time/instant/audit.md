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

1. **Codegen (registration ordering):** full-core compile emits legacy
   `MakeVariant` (synthetic id) for `Maybe.Some` because `Maybe`'s
   descriptor isn't yet populated when `time` compiles.
2. **Runtime (dispatch):** a value built by legacy `MakeVariant` for
   `Maybe` does not dispatch `.is_some`/`.unwrap_or` — the panic reports
   `runtime kind Int` (and `alloc_variant` returns a heap *ptr*, so the
   "Int" classification is itself suspect: either the archived body
   returns the unboxed payload, or the synthetic-id receiver is
   mis-classified). `MakeVariantTyped` Maybe values dispatch fine
   (small-module mirrors pass).

A fresh small-module mirror passes because it gets the typed form; the
archive (full-core) body gets the legacy form and fails. This is the
same archive/codegen Maybe-variant family memory flags as multi-day
and previously-reverted — it is **NOT** a bare-vs-qualified source
issue.

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

### Fix direction (fundamental) — two independent fixes, either closes it

1. **Codegen (preferred, fixes the whole class):** populate the core
   ADT descriptors (`Maybe`, `Result`, `Ordering`, `Poll`) with their
   real `variants` BEFORE any stdlib module body compiles in the
   full-core precompile, so `emit_make_variant`'s typed-form gate
   (`codegen/expressions.rs:473`, `desc.variants.is_empty()`) passes
   and `Maybe.Some` emits `MakeVariantTyped` everywhere — not just in
   single-module compiles. This fixes every Maybe/Result-returning
   stdlib function, not only `time`.
2. **Runtime (defence-in-depth):** make CallM dispatch recognise a
   legacy synthetic-id variant receiver (`0x8000+tag`) as the canonical
   `Maybe`/`Result` type when the method is Maybe/Result-specific
   (`is_some`/`is_none`/`unwrap`/`unwrap_or` → Maybe; `is_ok`/`is_err`
   → Result), routing instead of panicking with `runtime kind Int`.
   The runtime already has synthetic-variant handling for `eq`/`hash`/
   `debug`/`arith` (`comparison.rs`, `debug.rs`, `arith_extended.rs`);
   method dispatch is the missing surface.

**Verification is environment-blocked:** each cycle is a ~25-min
contended `cargo build` (full archive precompile + relink), and a
concurrent session was observed editing `core/` + relinking
`target/release/verum` mid-test (test killed, exit 144). Use a copied
stable binary (`cp target/release/verum /tmp/verum-stable`) to run
tests immune to relinks.

**Effort:** multi-day VBC archive/codegen work (gated). Do **not**
mark instant stable until §D closes and the 8 tests are GREEN under
both tiers.
