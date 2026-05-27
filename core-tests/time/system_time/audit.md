# `core.time.system_time` — audit findings

> Module under test: `core/time/system_time.vr` (363 LOC; the
> `SystemTime { secs: Int, nanos: Int }` record + 1-variant
> `SystemTimeError.WentBackwards(Duration)` ADT + 4 module-level
> free functions `now_unix_{ns,us,ms,s}` / `epoch_seconds`).
>
> Test surfaces (this branch):
> `unit_test.vr` (203 LOC, 23 `@test`s),
> `property_test.vr` (109 LOC, 9 `@test`s + 1 `@test_case` 4-case truth table),
> `integration_test.vr` (124 LOC, 10 `@test`s).

## 1. Cross-stdlib usage

`SystemTime` is the canonical wall-clock time. It is the reference
clock for every absolute / timestamp / log-line / certificate-validity
API:

| Consumer | Use |
|---|---|
| `core.time.rfc3339` | `now_utc()` snapshots `SystemTime.now()` into `Rfc3339Time { unix_seconds, nanos, offset_minutes: 0 }`. |
| `core.time.cron.CronExpr.next_after_unix` | Takes a unix-second `Int`; callers convert via `SystemTime.now().timestamp()`. |
| `core.security.x509` / `core.security.sigstore` / `core.storage.s3` | `epoch_seconds()` for certificate-validity windows + presign-URL expiry. |
| `core.tracing.id.generate_*` | W3C trace IDs include a millisecond timestamp (consumes `now_unix_ms()`). |
| `core.cache.types.CacheTtl` | `Seconds(N)` / `Millis(N)` expiry computations consume `now_unix_*` to derive deadlines. |
| `core.cog.manifest` (artifact mtime) | `SystemTime` compared against `core.io.fs::Metadata::modified()`. |

## 2. Crate-side hardcodes

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `realtime_nanos()` intrinsic | Per-platform `CLOCK_REALTIME` / `clock_gettime` / `GetSystemTimePreciseAsFileTime` lowering | Drift between intrinsic and platform syscall = wrong wall-clock; every TLS certificate validity check would mis-classify expiry. |
| Two-field `{ secs: Int, nanos: Int }` layout | 16-byte record; codegen + LLVM lowering | Drift = wrong-offset reads (same defect class as `[[btree_pattern_match_ref_generic_class]]` if reads cross module boundaries). |
| `nanos: Int` invariant `[0, 999_999_999]` | Sub-second precision and `subsec_*` accessor correctness | Constructors like `from_timestamp_millis` MUST decompose ms→(secs, nanos) maintaining the invariant; codegen would emit out-of-range values if the modulo op were lost. |
| `SystemTimeError.WentBackwards(Duration)` carries a Duration payload | Caller can inspect the gap | Drift between `core/time/system_time.vr` and `core/time/duration.vr` (Duration record layout drift) would mis-deserialise the payload on cross-module variant unpack. |

## 3. Language-implementation gaps

### §3.1 `duration_since` arithmetic uses `secs * NANOS_PER_SEC + nanos`

The computation at `core/time/system_time.vr:129-130` reads:

```verum
let self_nanos = self.secs * NANOS_PER_SEC + self.nanos;
let earlier_nanos = earlier.secs * NANOS_PER_SEC + earlier.nanos;
```

For `Int = Int64`, multiplying `secs` by `1_000_000_000` overflows
when `secs > Int64.MAX / 10^9 ≈ 9.2e9`. This corresponds to a
year ≈ 292 (in years post 1970 → year 2262). For practical wall-clock
timestamps this is safely outside the operational window. But the
test surface does not pin the overflow boundary explicitly — a
malicious or adversarial timestamp (e.g., a corrupted file mtime
read as `secs: Int.max_value()`) could exercise this. Add a
property pin that `secs <= some_safe_upper_bound` (e.g.,
`SystemTime { secs: 2^60, nanos: 0 }.duration_since(...)` returns
either Ok with a sensible Duration or a new `Overflow` error variant).

**Effort:** small (~30 min) — add overflow guard to `duration_since`
+ new `SystemTimeError.Overflow` variant + property test.

### §3.2 `checked_add` carry handling has off-by-one in `extra_secs`

The computation at `core/time/system_time.vr:148-160`:

```verum
let total_nanos = self.nanos + duration.subsec_nanos();
let extra_secs = total_nanos / NANOS_PER_SEC;
let remaining_nanos = total_nanos % NANOS_PER_SEC;
let new_secs = self.secs + duration.as_secs() + extra_secs;
```

The `total_nanos / NANOS_PER_SEC` is at most 1 when both inputs
satisfy the `[0, 999_999_999]` invariant (max sum = 1_999_999_998
ns < 2 × NANOS_PER_SEC). The current shape is correct. Property
test `law_add_sub_round_trip` covers this. No defect.

### §3.3 `now_unix_ns` returns Int (signed) — potential year-2262 trap

`now_unix_ns() -> Int` returns `self.secs * NANOS_PER_SEC + self.nanos`
without overflow guard. For `secs ≈ 9.2e9` this overflows. Same
class as §3.1. Document in source as "valid until year 2262".

**Effort:** trivial (~5 min) — add docstring note.

### §3.4 `WentBackwards` is the only `SystemTimeError` variant

`SystemTimeError` is a 1-variant enum carrying `Duration`. Future
variants (e.g., `Overflow`, `Underflow`, `InvalidSubsecond`) would
require extending the enum + Display/Debug match arms + every
`unwrap_err()` consumer. The current API surface is minimal and
correct for the documented use case.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| — | Per-submodule conformance suite for `core.time.system_time` | `core-tests/time/system_time/{unit,property,integration}_test.vr` | Pre-existing in this branch; this audit pins the coverage map. |
| — | Missing `audit.md` for `core-tests/time/system_time/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Year-2262 overflow guard + Overflow error variant | 30 min | open |
| §B | now_unix_ns docstring note re. year-2262 trap | 5 min | open |
| §C | Cross-tier (`--aot` vs `--interp`) divergence sweep | ~10 min wall-clock | open |
| §D | `Display` / `Debug` rendering text assertions | 15 min | open |
| §E | `WentBackwards.duration()` method round-trip test (currently covered by `test_went_backwards_duration_method`) — extend to assert payload identity after Clone | 5 min | open |

## 6. Status

**stable** under `--interp`. The 23 unit + 9 property + 10 integration
tests cover every public method on a constructed (`from_timestamp` /
`from_timestamp_millis`) `SystemTime` value + the `SystemTimeError`
variant + the module-level `now_unix_*` free functions.

Two `now()`-dependent integration tests (`test_now_unix_s_is_after_epoch`
+ `test_now_unix_ms_consistent_with_seconds`) assert that the
wall-clock realtime intrinsic returns sane values (post-2020 epoch
seconds); these would fail on a test environment with a misconfigured
clock — which is correct: the test environment IS the integration
surface for the realtime intrinsic.

6 sampled tests confirmed green 2026-05-27 — 1 baseline
(`test_unix_epoch_secs_is_zero`, 44.8s) + 5 batch
(`test_unix_epoch_*`, all green in 146.5s wall).
