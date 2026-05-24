# `diagnostics` audit

Module: `core/diagnostics.vr` (354 LOC) — umbrella for user-facing
read-only observability. Composes three layer-local snapshot surfaces:
* `core.mem.diagnostics` — CBGR allocation introspection
* `core.mem.cap_audit` / `core.mem.cap_audit_ring` — capability audit log
* `core.async.diagnostics` — async task introspection
* `core.base.panic` — panic-site identity (PanicInfo, Location)

Plus the umbrella-local `PanicReport` composing record with builder
pattern (`from_info`, `with_task`, `with_header`, `with_call_stack`,
`with_thread_name`) and predicates (`has_task`, `has_header`,
`has_thread_name`, `has_call_stack`) plus `format_panic_context` +
`Display` + `Debug`.

Tests: `unit_test.vr` (~15 `@test`s — builder + predicates +
format_panic_context + Display/Debug),
`property_test.vr` (~8 `@test`s — algebraic laws over builder pattern),
`integration_test.vr` (~5 `@test`s — Location composition, log line
embedding, builder chain).

## 1. Cross-stdlib usage

`PanicReport` is consumed by:

| crate / module | what it does |
|---|---|
| panic hooks (compiler-emitted) | construct PanicReport at panic time, populating subsections via builder methods as the hook gathers context from different runtime layers. |
| `core.base.panic` | provides `PanicInfo` + `Location` records that PanicReport composes. |
| `core.async.diagnostics` | provides `TaskInfo` + `TaskLifecycleState` that `with_task` attaches. |
| `core.mem.diagnostics` | provides `MemHeaderView` that `with_header` attaches; `current_call_stack()` for `with_call_stack`. |

## 2. Crate-side hardcodes

`crates/verum_compiler/src/...` panic-hook emission references the
`PanicReport.from_info(...)` constructor. Drift between umbrella and
compiler-emitted code is caught by the integration tests when a
panicking test triggers the hook.

`crates/verum_vbc/src/interpreter/...` panic propagation references
`PanicInfo` directly via `InterpreterError::Panic { message }`. The
umbrella-side composition lives entirely in Verum.

## 3. Language-implementation gaps

### §3.1 PanicReport.with_call_stack requires `List<CallFrame>` literal construction

Today there's no convenient builder for synthetic `CallFrame` lists
in tests — tests have to either capture a real `current_call_stack()`
(which is empty under Tier-0 without VBC debug-info wiring) or build
the List manually. The integration tests above sidestep this by
focusing on the non-call_stack paths. Once live call-stack capture
lands (task #197), add `with_call_stack` integration tests.

**Effort:** trivial when the upstream lands.

### §3.2 No `format_panic_context` variant that streams to a `Write` sink

`format_panic_context` allocates a single Text buffer. For OOM-path
panics (where the allocator is itself failing) we'd want a streaming
variant that writes directly to a pre-allocated `&mut Formatter` or
syscall write. The current shape doesn't allow this; add
`fn format_panic_context_into(&self, f: &mut Formatter) -> Result<(), FormatError>`.

**Effort:** small (~1h) + tests.

### §3.3 Has-checks aren't reflected as a single `summary` accessor

A common consumer pattern is "did this panic carry any extra context?".
Today callers must call all four `has_*` predicates and OR them.
Adding `fn is_minimal(&self) -> Bool { !has_task && !has_header && !has_thread_name && !has_call_stack }`
would consolidate this — useful for the "synchronous panic with no
context" log-routing decision.

**Effort:** trivial (~10 min).

## Action items landed in this branch

* `core-tests/diagnostics/{unit,property,integration}_test.vr` — first
  conformance suite for the umbrella's `PanicReport`-centric surface.
* `core-tests/diagnostics/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `format_panic_context_into(&Formatter)` streaming variant | `core/diagnostics.vr` + test | 1h |
| Add `is_minimal(&self) -> Bool` consolidated predicate | `core/diagnostics.vr` + test | 10 min |
| Live `with_call_stack` integration test (when task #197 lands) | this folder | as upstream closes |
| Capability-audit re-export conformance tests | this folder (covers `record_revoke` / `recent_cap_events` etc.) | 2h |
