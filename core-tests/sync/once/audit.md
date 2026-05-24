# `sync/once` audit

Module: `core/sync/once.vr` (612 LOC) — `Once` (one-time
initialization primitive, atomic state machine), `OnceState` enum
(New / InProgress / Done / Poisoned), `OnceLock<T>` (lazy
initialization container).

Tests focus on the testable static surface: `OnceState` 4-variant
construction + disjointness. Live `Once` interactions
(call_once / try_call_once / call_once_force / state) require a
concurrent execution context — tested at the language level via
`vcs/specs/L2-standard/sync/once/`.

## 1. Cross-stdlib usage

`Once` consumed by:
| consumer | how |
|---|---|
| `core.security.csprng.*` | one-shot RNG seed initialization |
| `core.runtime.*` | global runtime context bootstrap |
| `core.sync.OnceLock` | layered on top |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/...` AtomicInt state machine constants
(`INCOMPLETE`/`RUNNING`/`COMPLETE`/`POISONED`) must match the
internal numeric values in `core/sync/once.vr:30-34`. Drift here
would break the CAS atomicity contract.

## 3. Language-implementation gaps

### §3.1 Live Once tests need multi-threaded harness

Cannot be tested at the value-shape level. Concurrent test
patterns (multiple threads calling call_once, one panicking,
others observing Poisoned, etc.) need `@test(spawn_n=4)`-style
test fixture — not available at the core-tests level today.
Cross-reference `vcs/specs/L2-standard/sync/once/`.

### §3.2 OnceGuard.drop SOUNDNESS fix pinned in doc-comment

The doc-comment at `once.vr:50-72` documents a load-bearing
soundness fix (CAS-based poisoning to avoid overwriting a
successful COMPLETE with POISONED in a panic-during-publication
race window). This MUST stay — regression here silently
breaks every Once consumer. Pin a regression test at the
language level once the multi-threaded harness lands.

### §3.3 No `OnceState.Eq` / `Display` / `Debug` impls

Pattern shared with other simple enums in stdlib. Add for
ergonomic state-machine assertions: `assert_eq!(once.state(),
OnceState.Done)` is the natural shape.

**Effort:** small (~30 min).

### §3.4 `try_call_once` Result<Bool, E> semantics

Doc-stated: returns Ok(true) on first successful call, Ok(false)
on subsequent observations, Err(e) if the closure returned Err.
The `Bool` discriminator is subtle — could be `Result<OnceCallResult, E>`
where `OnceCallResult = FirstCall | Observed`. Document or refactor.

## Action items landed in this branch

* `core-tests/sync/once/unit_test.vr` — 7 unit tests covering
  OnceState 4-variant + disjointness + state-machine existence.
* `core-tests/sync/once/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `OnceState.Eq` / `Display` / `Debug` impls | `core/sync/once.vr` + tests | 30 min |
| Add language-level multi-threaded once tests | `vcs/specs/L2-standard/sync/once/` | 1 day |
| Pin OnceGuard.drop soundness fix at language level | regression_test.vr in L2 specs | 30 min |
| Sister tests for `core.sync.{atomic,mutex,rwlock,condvar,semaphore,barrier,waitgroup}` | sister folders | 1 day each |
EOF
