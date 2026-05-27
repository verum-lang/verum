# `sync/once` audit

Module: `core/sync/once.vr` (650 LOC) —
* `Once` (one-time initialization synchronization primitive with
  CAS-driven state machine over `AtomicInt`),
* `OnceState` enum (New / InProgress / Done / Poisoned) with
  canonical `.name()` + Display + Debug + Eq (pre-existing in
  this branch),
* `OnceLock<T>` (lazy initialization container layered over Once).

Tests focus on the testable static surface:
* `OnceState` 4-variant construction + pairwise disjointness +
  `name()` canonical-token + Eq reflexivity / inequality matrix.

Live `Once.call_once` / `try_call_once` / `call_once_force` /
`state()` interactions require concurrent execution + need a
multi-threaded test harness — tested at the language level in
`vcs/specs/L2-standard/sync/once/`.

## 1. Cross-stdlib usage

`Once` consumed by:

| consumer | how |
|---|---|
| `core.security.csprng.*`  | one-shot RNG seed initialization |
| `core.runtime.*`          | global runtime context bootstrap |
| `core.sync.OnceLock`      | layered on top |
| `core.cache.lru`          | per-process singleton stats counters |
| `core.tracing.span_registry` | one-shot tracer/exporter init |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/...` AtomicInt state machine constants
(`INCOMPLETE` = 0 / `RUNNING` = 1 / `COMPLETE` = 2 / `POISONED` = 3)
MUST match the internal numeric values in `core/sync/once.vr:30-34`.
Drift here would break the CAS atomicity contract.  Pin the
state-tag invariant via the `OnceState` 4-variant disjointness
+ name() canonical tokens.

## 3. Language-implementation gaps

### §3.1 Live Once tests need multi-threaded harness

Cannot be tested at the value-shape level.  Concurrent test
patterns (multiple threads calling call_once, one panicking,
others observing Poisoned, etc.) need `@test(spawn_n=4)`-style
test fixtures — not yet available at the core-tests level.
Cross-reference `vcs/specs/L2-standard/sync/once/`.

### §3.2 OnceGuard.drop SOUNDNESS fix pinned in doc-comment

The doc-comment at `once.vr:50-72` documents a load-bearing
soundness fix (CAS-based poisoning to avoid overwriting a
successful COMPLETE with POISONED in a panic-during-publication
race window).  This MUST stay — regression here silently breaks
every Once consumer.  Pin a regression test at the language
level once the multi-threaded harness lands.

### §3.3 Closed — OnceState Eq / Display / Debug / name impls

`OnceState` carries:
* `.name() -> Text` (canonical "New" / "InProgress" / "Done" /
  "Poisoned" tokens) at `once.vr:46-53`,
* `Display for OnceState` at `once.vr:56-60`,
* `Debug for OnceState` at `once.vr:62-66`,
* `Eq for OnceState` (tag-only equality) at `once.vr:68-79`.

Pinned by `property_test.vr` §B / §C + `regression_test.vr`.

### §3.4 `try_call_once` Result<Bool, E> semantics

Doc-stated: returns `Ok(true)` on first successful call, `Ok(false)`
on subsequent observations, `Err(e)` if the closure returned Err.
The `Bool` discriminator is subtle — could be `Result<OnceCallResult, E>`
where `OnceCallResult = FirstCall | Observed`.  Document or
refactor.  Deferred.

## Action items landed in this branch

* `core-tests/sync/once/unit_test.vr` — 7 unit tests (pre-existing)
  covering OnceState 4-variant + disjointness + state-machine
  existence.
* `core-tests/sync/once/property_test.vr` — pairwise disjointness
  matrix + name() injectivity + Eq laws.
* `core-tests/sync/once/regression_test.vr` — LOCK-IN for §3.3
  (Eq / Display / Debug / name impls) + §3.1 four-variant ADT
  shape pin.
* `core-tests/sync/once/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Live Once.call_once + multi-threaded harness | `vcs/specs/L2-standard/sync/once/` | 1 day |
| OnceGuard.drop SOUNDNESS regression pin (multi-thread) | language-level | 30 min once harness lands |
| `OnceCallResult` refactor of `try_call_once` Result<Bool, E> | refactor + ripple through consumers | 1 day |
| OnceLock<T> single-threaded get_or_init / get / set / take coverage | `core-tests/sync/once/integration_test.vr` | 1 day |
