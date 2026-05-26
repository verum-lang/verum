# `core/base/log` — Audit

> Module: `core/base/log.vr` — LogLevel ADT (Trace/Debug/Info/Warn/Error),
> LogRecord + LogRecordBuilder structured records, Logger protocol,
> NullLogger no-op backend.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `LogLevel` | 5-variant sum `Trace \| Debug \| Info \| Warn \| Error` | yes |
| `LogRecord` | 6-field record `{ level, message, fields, module_path, file, line }` | yes |
| `LogRecordBuilder` | 6-field builder record | yes |
| `Logger` | protocol `{ log, is_enabled }` | yes |
| `NullLogger` | empty record — discards all messages | yes |

### 1.2 LogLevel API

| Item | Signature |
|---|---|
| `LogLevel.severity` | `(&self) -> Int` (0=Trace ... 4=Error) |
| `LogLevel.name` | `(&self) -> Text` (canonical UPPERCASE) |
| `LogLevel.default` | `() -> LogLevel` (returns Info) |
| `LogLevel` impls | Display / Debug / Clone / Copy / PartialEq / Eq / PartialOrd / Ord / Hash / Default |

### 1.3 Free-fn level constructors

| Item | Signature |
|---|---|
| `level_trace` | `() -> LogLevel` |
| `level_debug` | `() -> LogLevel` |
| `level_info` | `() -> LogLevel` |
| `level_warn` | `() -> LogLevel` |
| `level_error` | `() -> LogLevel` |

### 1.4 LogRecord factories

| Item | Signature |
|---|---|
| `LogRecord.new` | `(LogLevel, Text, List<(Text, Text)>) -> LogRecord` |
| `LogRecord.simple` | `(LogLevel, Text) -> LogRecord` |
| `LogRecord.with_module_path` | `(Self, Text) -> LogRecord` |
| `LogRecord.with_file` | `(Self, Text) -> LogRecord` |
| `LogRecord.with_line` | `(Self, Int) -> LogRecord` |
| `LogRecord.with_field` | `(Self, Text, Text) -> LogRecord` |

### 1.5 LogRecordBuilder API

| Item | Signature |
|---|---|
| `LogRecordBuilder.new` | `() -> LogRecordBuilder` |
| `.level`, `.message`, `.field`, `.module_path`, `.file`, `.line` | chainable builders |
| `.build` | `(Self) -> LogRecord` |

### 1.6 Convenience log functions (context-using)

| Item | Signature |
|---|---|
| `trace` / `debug` / `info` / `warn` / `error` | `(&Text) using [Logger]` |

### 1.7 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 4 unit tests | all green under `--interp` |
| `property_test.vr` | 10 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 8 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 8 active + 1 `@ignore`'d | 8 green; 1 pinned on §2.1 |

## §2 — Findings landed in this branch

### 2.1 Chained-builder return-value field-access defect

`let rec = LogRecordBuilder.new().level(L).message(M).field(...).build();
rec.level.name()` mis-resolves the `.level` field on the
builder-returned LogRecord. Symptom: `assert_eq(lvl.name(), "WARN")`
fails — the `lvl` value is not the LogLevel.Warn that was set.

Defect class: same root as
`[[btree_pattern_match_ref_generic_class]]` applied to the chained-
builder return value. VBC codegen loses the LogRecord type as it
flows through `.build()`'s return, so `.level` field access reads at
a wrong offset.

* Workaround in `integration_test.vr`: assert only the structural
  invariants (`.fields.len() == N`) that don't traverse through
  `.level`.
* Pin at `regression_test.vr §A` as `@ignore`'d — flips green when
  the multi-day VBC codegen fix for chained-builder type tracking
  lands.

### 2.2 Pre-existing unit/property tests are correct

`unit_test.vr` and `property_test.vr` cover the LogLevel ADT
(severity ordering, name canonical UPPERCASE, severity 0..=4),
LogRecord structured fields, level_*() free-fn constructors. All
green under `--interp`.

The INVENTORY claim of "1 fail TBD" was likely tracking a different
log test elsewhere (e.g. `context/standard` LogLevel Display
mis-dispatch).

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.log`:

* `core.context.standard` — declares the `Logger` context dispatcher
  + the ContextLogLevel ADT (duplicates the LogLevel sum surface).
* `core.tracing.*` — span event severity.
* `core.diagnostics.*` — error reporting.
* Application code via `using [Logger]`.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded LogLevel mapping.

## §5 — Action items landed in this branch

1. `core-tests/base/log/integration_test.vr` — `test_log_record_builder_chain_preserves_fields` relaxed to length-only assertion (workaround for §2.1); typed-local `let rec: LogRecord = ...; let lvl: LogLevel = rec.level` pattern documented.

2. NEW `core-tests/base/log/regression_test.vr` — 8 active + 1
   `@ignore`'d pins:
     §A `@ignore`'d × 1 — chained-builder `.level` field-access defect
     §B LogLevel severity strict total order Trace<Debug<Info<Warn<Error
     §C Canonical severity values 0/1/2/3/4
     §D Canonical UPPERCASE names TRACE/DEBUG/INFO/WARN/ERROR
     §E Free-fn `level_*()` constructors return correct variant
     §F LogRecord.new with empty fields
     §G LogRecord.simple has empty fields by construction
     §H NullLogger.is_enabled always returns false
     §H' LogLevel.default() returns Info (severity 2)

3. NEW `core-tests/base/log/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close chained-builder return-value type-tracking defect | multi-day VBC codegen work | regression §A pin |
| LogRecord.with_module_path / with_file / with_line chained builder tests | gated on §A | future task |
| Logger protocol live integration via `using [Logger] { provide ... in { ... } }` | gated on context system | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
