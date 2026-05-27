# `core/base/panic` — Audit

> Module: `core/base/panic.vr` — panic / abort / exit primitives,
> the rich `assert*` family (`assert_eq`, `assert_ne`, `assert_some`,
> `assert_none`, `assert_ok`, `assert_err`, `assert_approx_eq`,
> `assert_between`, `assert_is_sorted`, `assert_contains`,
> `assert_panics`), `unreachable` / `unimplemented` / `todo`, plus
> the `Location` + `PanicInfo` record surface for catch_unwind.

## §1 — Public API surface

### 1.1 Records

| Type | Shape | Public? |
|---|---|---|
| `Location` | `{ file: Text, line: Int, column: Int }` | yes |
| `PanicInfo` | `{ message: Text, location: Maybe<Location> }` | yes |
| `PanicHook` | function-typed callback | yes |
| `CatchResult<T>` | `Result<T, PanicInfo>`-like sum | yes |

### 1.2 Panic primitives

| Item | Signature |
|---|---|
| `panic` | `(Text) -> !` |
| `panic_at` | `(Text, Text, Int, Int) -> !` (msg, file, line, column) |
| `panic_fmt` | `(FormatArgs) -> !` |
| `abort` | `() -> !` |
| `exit` | `(Int) -> !` |
| `unreachable` | `() -> !` |
| `unreachable_unchecked` | `() -> !` (release fast-path) |
| `unimplemented` | `() -> !` |
| `todo` | `() -> !` |

### 1.3 Assert family

| Item | Signature |
|---|---|
| `assert` | `(Bool, Text = ...)` |
| `assert_eq<T: Eq + Debug>` | `(T, T, Text = ...)` |
| `assert_ne<T: Eq + Debug>` | `(T, T, Text = ...)` |
| `assert_some<T>` | `(Maybe<T>, Text = ...) -> T` |
| `assert_none<T>` | `(Maybe<T>, Text = ...)` |
| `assert_ok<T, E: Debug>` | `(Result<T, E>, Text = ...) -> T` |
| `assert_err<T: Debug, E>` | `(Result<T, E>, Text = ...) -> E` |
| `assert_approx_eq` | `(Float, Float, Float = 1e-9, Text = ...)` |
| `assert_between<T: Ord>` | `(T, T, T, Text = ...)` |
| `assert_is_sorted<T: Ord>` | `(&List<T>, Text = ...)` |
| `assert_contains<T: Eq>` | `(&List<T>, &T, Text = ...)` |
| `assert_panics` | `(fn() -> Unit, Text = ...)` |
| `debug_assert*` | Debug-only mirror of the assert family |

### 1.4 Catch / hook surface

| Item | Signature |
|---|---|
| `catch_unwind<T>` | `(fn() -> T) -> CatchResult<T>` |
| `resume_unwind` | `(PanicInfo) -> !` |
| `PanicHook` | `fn(&PanicInfo)` typed callback |
| `default_panic_hook` | `() -> PanicHook` |

### 1.5 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 95 tests | most green (2 `@ignore`'d for §2.1) |
| `property_test.vr` | 28 algebraic laws | green |
| `integration_test.vr` | 11 scenarios | green |
| `regression_test.vr` | 7 active + 2 `@ignore`'d | 7 green; 2 pinned on §2.1 |

## §2 — Findings landed in this branch

### 2.1 `Formatter.write_int` lenient-stubbed via private `Text.from_utf8_unchecked`

`Formatter.write_int` is part of the canonical `fmt_debug` /
`Display.fmt` write path that converts integer values to bytes. The
underlying `Text.from_utf8_unchecked` was declared private (`unsafe fn`
without `public`) at `core/text/text.vr:455`, so the cross-module
reference inside `Formatter.write_int` couldn't be resolved at
precompile and the function was lenient-stubbed:

```
[lenient] Formatter.write_int compiled to panic-stub:
undefined function: Text.from_utf8_unchecked
(in function Formatter.write_int)
```

Every `loc.fmt_debug(&mut formatter)` / `loc.fmt(&mut formatter)`
call eventually hits `write_int` (for line / column numbers) and
panics. Same defect class as `core-tests/base/ulid/regression_test.vr §A`.

**Fix landed in this branch** (ulid commit cbd79805f — text.vr:455
`unsafe fn` → `public unsafe fn from_utf8_unchecked`). Activates after
next precompiled-stdlib refresh.

### 2.2 Pre-existing unit/property/integration tests largely green

`unit_test.vr` (95 tests) covers the full assert family + panic
primitives + Location/PanicInfo record surface. Most pass; only the
2 `Formatter`-using tests (`test_location_debug_format` /
`test_location_display_format`) hit §2.1.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.panic`:

* Every Verum module that uses `panic` / `assert*` / `unreachable` /
  `todo` (i.e. effectively all of stdlib and application code).
* `core.diagnostics.*` — panic-report formatting.
* Application code via `mount core.prelude.*` (the family is
  re-exported there).

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/panic/unit_test.vr` — 2 tests `@ignore`'d for §2.1
   (Formatter.write_int panic-stub). Underlying stdlib fix already
   staged via text.vr:455 visibility change (ulid work).

2. NEW `core-tests/base/panic/regression_test.vr` — 7 active + 2
   `@ignore`'d pins:
     §A `@ignore`'d × 2 — Location.fmt_debug / .fmt via Formatter
        (gated on Text.from_utf8_unchecked visibility refresh)
     §B Location 3-field record-layout pin
     §C Location.new preserves field order (line / column non-swap)
     §D PanicInfo with / without location round-trip
     §E PanicInfo 2-field record-layout pin
     §F Location.new with empty file + zero positions

3. NEW `core-tests/base/panic/audit.md` — documents API surface,
   Formatter defect, deferred items.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Text.from_utf8_unchecked visibility refresh (precompiled stdlib) | gated on next stdlib rebuild | regression §A pins |
| `catch_unwind` actual unwinding semantics under `--interp` (currently no-op?) | medium-day VBC runtime work | future task |
| `assert_panics` integration with closure-mutable-state defect | gated on closure defect close (see base/retry §A) | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
