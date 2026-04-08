# core/time Test Suite

Test coverage for Verum's time module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `duration_test.vr` | `time/duration` | Duration construction, accessors, arithmetic, comparison, checked/saturating ops, protocols | 98 |
| `instant_test.vr` | `time/instant` | Instant construction, elapsed, arithmetic, comparison, benchmarking patterns | 52 |
| `system_time_test.vr` | `time/system_time` | SystemTime construction, accessors, duration calculations, arithmetic, comparison, precision, errors | 66 |
| `time_namespace_test.vr` | `time/mod` | Time.now(), Time.monotonic(), Time.sleep() | 19 |
| `time_improvements_test.vr` | `time/*` | Duration.max_value, Display for Instant/SystemTime, arithmetic edge cases | 44 |

## Key Types Tested

### Duration
A span of time stored as nanoseconds (always non-negative).

**Constructors:**
- `Duration.zero()` - Zero duration
- `Duration.nanos(n)`, `Duration.from_nanos(n)` - From nanoseconds
- `Duration.micros(n)`, `Duration.from_micros(n)` - From microseconds
- `Duration.millis(n)`, `Duration.from_millis(n)` - From milliseconds
- `Duration.secs(n)`, `Duration.from_secs(n)` - From seconds
- `Duration.mins(n)` - From minutes
- `Duration.hours(n)` - From hours
- `Duration.new(secs, nanos)` - From seconds + nanoseconds

**Accessors:**
- `as_nanos()`, `as_micros()`, `as_millis()`, `as_secs()`, `as_secs_f64()`
- `subsec_nanos()`, `subsec_micros()`, `subsec_millis()`
- `is_zero()`

**Arithmetic:**
- `checked_add()`, `saturating_add()`
- `checked_sub()`, `saturating_sub()`
- `checked_mul()`, `saturating_mul()`
- `checked_div()`
- Operators: `+`, `-`, `*`, `/`

### Instant
A point in monotonic time (never goes backwards).

**Methods:**
- `Instant.now()` - Current instant
- `elapsed()` - Duration since this instant
- `duration_since(earlier)` - Duration from earlier instant
- `saturating_duration_since(earlier)` - Same, saturates at zero
- `checked_add(duration)`, `checked_sub(duration)`
- Operators: `+ Duration`, `- Duration`, `- Instant`

### SystemTime
Wall-clock time (UNIX epoch based, can go backwards).

**Constructors:**
- `SystemTime.UNIX_EPOCH()` - January 1, 1970
- `SystemTime.now()` - Current system time
- `SystemTime.from_timestamp(secs)` - From Unix timestamp
- `SystemTime.from_timestamp_millis(ms)` - From milliseconds

**Accessors:**
- `timestamp()` - Unix timestamp in seconds
- `timestamp_millis()` - In milliseconds
- `timestamp_nanos()` - In nanoseconds
- `duration_since_epoch()` - Duration since epoch

**Duration calculations:**
- `duration_since(&earlier)` - Result<Duration, SystemTimeError>
- `elapsed()` - Result<Duration, SystemTimeError>
- `checked_add(duration)`, `checked_sub(duration)`

### SystemTimeError
Error when time appears to go backwards.

- `WentBackwards(Duration)` - Contains the backwards duration
- `duration()` - Extract the backwards duration

### Time Namespace
Static methods for general time operations.

- `Time.now()` - Current monotonic time as Duration
- `Time.monotonic()` - Raw monotonic nanoseconds (Int)
- `Time.sleep(duration)` - Sleep for duration
- `Time.sleep_ms(ms)` - Sleep for milliseconds
- `Time.sleep_secs(secs)` - Sleep for seconds

## Tests by Category

### Duration Tests (98 tests)
- **Constructors**: zero, from_nanos, from_micros, from_millis, from_secs, from_mins, from_hours, new
- **Constructor aliases**: from_nanos/nanos, from_micros/micros, from_millis/millis, from_secs/secs equivalence
- **Accessors**: as_nanos, as_micros, as_millis, as_secs, subsec_nanos, subsec_micros, subsec_millis
- **Float conversion**: as_secs_f64 (whole, fractional, zero, millis-only, large), from_secs_f64 (whole, fractional, zero, small, large)
- **Arithmetic**: add, add_different_units, sub, sub_saturating, mul, div
- **Arithmetic edge cases**: add zero (left/right), sub zero, mul by 0/1, div by 1
- **Checked operations**: checked_add (nonzero+nonzero, zero+nonzero, commutative), checked_sub (to zero, larger-smaller), checked_mul (by 0/1/large), checked_div (by 1, even, truncation, by zero)
- **Saturating operations**: saturating_add (basic, with zero), saturating_sub (larger from smaller, zero-something, equal), saturating_mul (basic, by zero)
- **Comparison**: equality (same/different construction), ordering (zero vs nonzero, nanos vs micros, transitivity), neq, le/ge equal
- **Mixed unit construction**: new with small nanos, large secs+nanos, zero secs+nanos, both zero
- **is_zero**: after sub to zero, not for 1 nano/micro/milli/sec
- **Minutes/hours**: from_mins to millis/multiple, from_hours to millis/multiple
- **Subsecond accessors**: exact millis, odd values, with remainder, zero subsec
- **Protocol behavior**: default is zero, eq reflexive, ord consistent with eq
- **Edge cases**: negative input, large values, precision, fractional seconds

### Instant Tests (52 tests)
- **Construction**: now()
- **Elapsed time**: elapsed, duration_since, saturating_duration_since
- **Arithmetic**: add_duration, sub_duration, sub_instant
- **Comparison**: ordering, equality
- **Checked operations**: checked_add, checked_sub
- **Benchmarking patterns**: benchmark timing, lap timing
- **Edge cases**: zero duration, self duration

### SystemTime Tests (66 tests)
- **Constructors**: UNIX_EPOCH, now, from_timestamp, from_timestamp_millis, from_timestamp_zero
- **From timestamp patterns**: one_day (86400s), one_million, year_2000 (946684800)
- **From timestamp millis patterns**: one_second, half_second, zero
- **Accessors**: timestamp, timestamp_millis, timestamp_nanos
- **Timestamp roundtrips**: seconds identity, millis identity, secs/millis consistency
- **Duration since epoch**: basic, millis, zero, now_positive, specific (1 hour)
- **Duration since other**: earlier, later (error), same, equal_times, commutative_error, millis_precision
- **Elapsed**: basic, now, epoch
- **Arithmetic**: checked_add, checked_add_zero, checked_add_millis, checked_add_large_duration, checked_sub, checked_sub_to_epoch, checked_sub_exact_to_epoch, checked_sub_underflow, checked_sub_zero_identity, checked_add_then_sub_roundtrip, add_operator, sub_operator
- **Operator +/-**: add_then_sub_roundtrip, add_zero_duration, sub_zero_duration, add_millis
- **Comparison**: equality, ordering, ordering_millis, epoch_less_than_now, reflexive, transitivity
- **Precision**: millis_precision_roundtrip, nanos_from_millis
- **Error handling**: SystemTimeError duration extraction, small_difference, large_difference
- **Constants/identity**: epoch_constants, two_epoch_calls_equal
- **Edge cases**: large timestamp, millisecond precision, nanosecond precision, consecutive now calls

### Time Namespace Tests (~17 tests)
- **Time.now()**: positive, monotonic, returns_duration
- **Time.monotonic()**: positive, increasing, vs_now consistency
- **Time.sleep()**: basic, zero, multiple
- **Time.sleep_ms()**: basic, zero
- **Time.sleep_secs()**: signature verification
- **Integration**: sleep_vs_instant_elapsed, monotonic_timing, duration_from_time_now
- **Edge cases**: rapid_monotonic_calls, rapid_now_calls

## Protocols Implemented

| Type | Eq | Ord | PartialOrd | Hash | Debug | Display | Clone | Copy | Default |
|------|----|----|------------|------|-------|---------|-------|------|---------|
| Duration | Y | Y | Y | Y | Y | Y | Y | Y | Y |
| Instant | Y | Y | Y | Y | Y | N | Y | Y | N |
| SystemTime | Y | Y | Y | Y | Y | N | Y | Y | N |
| SystemTimeError | N | N | N | N | Y | Y | N | N | N |

## Constants

```verum
NANOS_PER_MICRO: Int = 1_000
NANOS_PER_MILLI: Int = 1_000_000
NANOS_PER_SEC: Int = 1_000_000_000
NANOS_PER_MIN: Int = 60_000_000_000
NANOS_PER_HOUR: Int = 3_600_000_000_000
```

## Known Limitations

- Sleep precision depends on OS scheduler (typically ~1ms on most systems)
- SystemTime can go backwards if system clock is adjusted
- Very long sleep durations not tested (would slow test suite)
- Platform-specific behavior not separately verified
- Concurrent timing tests not included

## Test Count: 235 tests total (4 test files)

## Architecture Notes

### Monotonic vs Wall-Clock Time
- `Instant` and `Time.monotonic()` use monotonic time that never goes backwards
- `SystemTime` uses wall-clock time that can be adjusted (NTP, manual changes)
- Use `Instant` for measuring elapsed time (benchmarks, timeouts)
- Use `SystemTime` for calendar time (timestamps, dates)

### Platform Integration
Uses V-LLSI (Verum Low-Level System Interface):
- Linux: `clock_gettime(CLOCK_MONOTONIC)`, `clock_gettime(CLOCK_REALTIME)`
- macOS: `mach_absolute_time()`, `gettimeofday()`
- Windows: `QueryPerformanceCounter()`, `GetSystemTimeAsFileTime()`

### Performance Targets
- `Time.monotonic()` / `Instant.now()`: ~15ns on Linux
- `SystemTime.now()`: ~30ns on Linux
