# `runtime/supervisor` audit

Module: `core/runtime/supervisor.vr` (1679 LOC, the largest in
`core/runtime/`) — Erlang/OTP-style supervision tree for fault-tolerant
async tasks.

Tests: 41 unit tests over the data-only ADT subset.  Live RestartIntensity
(atomic-counter sliding window) + Supervisor protocol + SupervisorHandle
+ root_supervisor() require atomic intrinsics + spawn — deferred to
`vcs/specs/L2-standard/runtime/supervisor/`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.runtime.spawn` | `SupervisorSpawnConfig.supervisor: Maybe<&SupervisorHandle>` attaches a spawn to a supervisor. |
| `core.runtime.recovery.RuntimeRecoveryStrategy.Supervision` | indicates "supervisor handles recovery" — delegated to this surface. |
| `core.async.runtime` | `root_supervisor()` is the OTP-style application root for all spawned async tasks. |
| `core.cli.error.ExitCode` | maps `EscalationPolicy.Terminate` to a non-zero process exit. |

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `@repr(UInt8)` on SupervisionStrategy / SupervisorRestartStrategy / ChildStatus / EscalationPolicy | wire-format ordinals | Drift in variant ORDER silently re-tags every persisted restart event. |
| `@repr(C) @size(48)` on RestartEvent | exact 48-byte layout for the inline circular buffer | Layout drift fragments the buffer. |
| 100-entry circular RestartEvent buffer | hardcoded ring size | High-restart-frequency services lose history past 100 events; not exposed as tuning. |
| `ChildSpec.new` defaults: `Transient` restart, 5000ms shutdown, 10 max_restarts, 60s window, 100 priority | hardcoded magic | Production tuning requires explicit override. |
| `SupervisorConfig.one_for_all` lowers max_restarts to 5 (vs 10 for one_for_one) | per-strategy heuristic | Source comment explains the reasoning; pin it via a test that asserts the difference. |
| `SUPERVISOR_ID_COUNTER` / `CHILD_ID_COUNTER` start at 1 (0 reserved for root) | global atomic counters | Without the 0-reserved discipline, `SupervisorId.new() == SupervisorId.root()` is possible. |

## 3. Language-implementation gaps

### §A — `FailureReason.Manual` treated as abnormal exit

Source contract (`supervisor.vr:217-222`): `is_abnormal()` returns
true for everything except `NormalExit` + `SupervisorShutdown`.
Manual restarts therefore trigger Transient restart strategy
re-execution.  Documented but counter-intuitive: a "manual restart"
is being treated like a crash.  Recommend: add `is_manual()`
predicate so Transient can opt-out.

### §B — RestartIntensity uses approximate sliding window

Source contract (`supervisor.vr:395-399`): "exponential decay rather
than storing individual timestamps".  This is a deliberate
allocation-free trade-off; the timestamp-precise window would
require a `List<UInt64>` per intensity tracker.  Acceptable, but
should be documented as a known approximation.

### §C — `ChildId.new` + `SupervisorId.new` use global atomic counters

Two process-wide AtomicU64 counters.  Soundness pin: the increment
must be atomic (uses `fetch_add(1, SeqCst)`).  Hazard: counter
wraparound at 2^64 is theoretically reachable; without explicit
overflow handling the counter wraps to 0 = root.  Recommend:
document the wraparound, OR use a saturating counter.

### §D — EscalationReason.SubSupervisorEscalation recursion via `Heap<EscalationReason>`

Recursive variant via `Heap` — infinite recursion hazard if a
supervisor's escalation triggers another supervisor's escalation
indefinitely.  Recommend: depth bound at construction OR cycle
detection in the description() formatter.

### §E — RestartEvent 48-byte layout + UInt8 reason_code mapping

`@size(48)` enforces the exact layout.  The `reason_code: UInt8`
field encodes FailureReason (a 7-variant ADT).  Mapping (FailureReason
→ reason_code) lives elsewhere in the file; without an explicit
constants table, drift between encode + decode is invisible.
Recommend: pin a constants table in `core/runtime/supervisor.vr`
and a property test that exhaustively round-trips the 7 variants.

### §F — SupervisorError.TooManyRestarts loses the most-recent FailureReason

Source surface: `TooManyRestarts { count, window_secs }` — no field
carrying the failure that finally triggered the limit.  Diagnostic
loss: an operator handling the escalation can't see what crashed.
Recommend: add `last_failure: FailureReason` to the payload.

## Action items landed in this branch

* `core-tests/runtime/supervisor/unit_test.vr` — 41 unit tests over
  SupervisorId/ChildId + SupervisionStrategy + SupervisorRestartStrategy +
  FailureReason + ChildStatus + EscalationPolicy + ShutdownStrategy +
  SupervisorError.
* `core-tests/runtime/supervisor/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A `FailureReason.is_manual()` predicate | `core/runtime/supervisor.vr` | 15 min |
| §B Sliding-window approximation documentation | `core/runtime/supervisor.vr` docstring | 15 min |
| §C `ChildId` / `SupervisorId` wraparound handling | `core/runtime/supervisor.vr` | 1 h |
| §D EscalationReason cycle-detection in description() | `core/runtime/supervisor.vr` | 1 h |
| §E RestartEvent reason_code constants table + round-trip property test | `core/runtime/supervisor.vr` + this folder | 2 h |
| §F `TooManyRestarts` payload extension | `core/runtime/supervisor.vr` + callers | 2 h |
| ChildSpec.new field-default test | this folder | 30 min (gated on cross-module ctor return defect [[spawn §F]]) |
| Live RestartIntensity.record_restart sliding window | `vcs/specs/L2-standard/runtime/supervisor/` | gated on atomics |
| OneForAll supervision strategy live test | sister | gated on spawn |
| Supervisor.start_child + propagation + escalation flow | sister | gated on root_supervisor() intrinsic |
| Display/Debug rendering tests for all 6 ADTs | this folder | 1 h |
