# Audit — `core/base/log.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/log.vr` (332 lines) |
| Tests | NEW — `unit_test.vr` (~50 LOC), `property_test.vr` (~70 LOC, total order + name + truth tables) |
| Hardcodes in `crates/` | none — pure stdlib facade |

## §1  LogLevel total order

The 5-variant LogLevel forms a total order via `severity()`. This is
the canonical filter mechanism: a logger configured at level `Info`
emits records with severity ≥ 2 (Info / Warn / Error).

## §2  Logger as Context

`Logger` is a protocol type designed to be provided through Verum's
context system: `fn foo() using [Logger] { Logger.info("hi") }`.
Tests in this folder don't exercise the context-system integration
because that requires runtime support that's at the language level —
test those in language-level conformance suites.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/log/`
- [x]  `unit_test.vr` — severity ordering, name values, LogRecord
       construction and builder chain
- [x]  `property_test.vr` — total-order law, non-empty names,
       @test_case truth tables for severity and name
- [x]  This audit document

## §4  Action items deferred

1. **Context-system integration tests** — requires a test mock for
   the Logger context. Once `@mock(Logger)` lands per
   CAPABILITY_GAPS §2.2.
2. **Zero-cost-when-off contract** — verify that
   `Logger.trace("expensive_thing()")` does NOT evaluate
   `expensive_thing()` when the level filter excludes Trace.
   Requires lazy-eval inspection.
3. **Structured-field encoding** — LogRecord carries fields; verify
   they're preserved through serde encoding.
