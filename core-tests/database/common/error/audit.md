# `database/common/error` audit

Module: `core/database/common/error.vr` (~887 LOC) — Spindle DbError
taxonomy. One error type for every Postgres / MySQL / SQLite adapter
with portable classifiers (`is_retryable`, `is_constraint_violation`,
`category`) and SQLSTATE-driven retry routing.

Tests: 47 unit tests over pure-data ADTs (AdapterId 3 + ConstraintKind
7 + ErrorCategory 5 + ConnectionErrorKind 6 + AuthErrorKind 5 +
TimeoutPhase 4 + PoolErrorKind 4) + variant-disjointness sets.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.database.postgres.adapter` | every Postgres error funnels through `DbError.QueryFailed` / `Adapter(AdapterSpecific)` |
| `core.database.mysql.adapter` | maps MySQL server errno → SQLSTATE + DbError variant |
| `core.database.sqlite.adapter` | maps SQLITE_* result codes → DbError |
| `core.database.common.retry` | `run_with_retry` dispatches on `DbError.is_retryable` |
| `core.database.common.into_response` | maps DbError → HTTP 4xx / 5xx via ErrorCategory |
| `verum_runtime::db::pool` | PoolErrorKind drives connection pool state machine |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_runtime::db::pg_error_codes` | mirrors SQLSTATE 23xxx → ConstraintKind table (lines 127-134). Drift here misroutes constraint violations to wrong HTTP statuses. |
| `verum_runtime::db::retry_predicate` | mirrors SqlState.is_retryable class-prefix check (`40` / `08`). Drift breaks safe-retry guarantee. |
| `verum_runtime::pool::supervisor` | mirrors PoolErrorKind 4-variant for state-machine dispatch. |

## 3. Language-implementation gaps

### §3.1 SqlState parse + classifiers not tested at this layer

`SqlState.parse(s)` returns `Maybe<SqlState>` after validating the
5-char ASCII shape. The successful case constructs `SqlState { code: Text }`
which is a record return from a cross-module fn — same defect class
as `meta/span` audit §3.1.

Once the cross-module record-return fix lands, add tests for:
* `SqlState.parse("23505")` returns Some, .class() == "23",
  .is_integrity() == true, .integrity_kind() == Some(CkUnique)
* `SqlState.parse("40001")` returns Some, .is_retryable() == true
* `SqlState.parse("08006")` returns Some, .is_retryable() == true
* `SqlState.parse("XX000")` returns None (parser rejects non-ASCII-alphanumeric)
* `SqlState.parse("foo")` returns None (wrong length)

### §3.2 DbError 10-variant integration tests deferred

DbError variants `ConnectionFailed`, `QueryFailed`, `DecodeFailed`,
`EncodeFailed`, `PoolFailure`, `AuthFailed`, `Adapter` all carry
mixed record payloads (DecodeFault, EncodeFault, AdapterSpecific,
etc) with Span / Text fields. Direct record literal at the test
site works for the data-only enum surface but constructing a full
DbError chain that exercises `.message()` / `.is_retryable()` /
`.category()` requires the cross-module fix.

### §3.3 AdapterId.name string mapping not tested

`AdapterId.name()` returns one of "postgres" / "sqlite" / "mysql".
Cross-module fn return defect blocks `.name()` field access.
Pinned for integration tier once cross-module fix lands.

## Action items landed in this branch

* `core-tests/database/common/error/unit_test.vr` — 47 unit tests
  covering 7 pure-data ADTs + variant disjointness + ConstraintKind
  Eq matrix via `.equals()` (which works as method dispatch, sidesteps
  record-return defect).
* `core-tests/database/common/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| SqlState parse + classifiers (§3.1) | this folder | 1 h after cross-module fix |
| DbError 10-variant integration (§3.2) | this folder | 2-3 h |
| AdapterId.name() (§3.3) | this folder | 15 min after cross-module fix |
| Property test: SQLSTATE class-prefix retry table | this folder | 30 min |
| Drift-pinning Rust unit test for SQLSTATE 23xxx → ConstraintKind table | crates/verum_runtime/src/db/pg_error_codes.rs | 30 min |
