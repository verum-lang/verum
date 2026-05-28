# `net/proxy/health_check` audit

Module: `core/net/proxy/health_check.vr` (~334 LOC) — active +
passive upstream health checks for the reverse proxy. Three
algebraic surfaces:

* `ProxyHealthStatus` — record holding three atomics (healthy +
  consecutive_failures + consecutive_successes). Constructed
  by `ProxyHealthStatus.new()`; default state is healthy.
* `HealthCheck` — 3-variant config sum:
  `Disabled` | `Active { period_ms, path, timeout_ms,
  healthy_threshold, unhealthy_threshold }` | `Passive {
  unhealthy_threshold, recovery_interval_ms }`.
* `HealthCheckError` — 3-variant error: `Connect(Text)` |
  `Timeout` | `InvalidResponse`. `Eq` is impl'd structurally.

`ActiveHealthChecker` runs the active probe loop; its
functional surface (`tick` / `run` / `record_outcome`) requires
a `HealthProbe` impl and is gated on async harness availability.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.proxy.loadbalancer` | reads `ProxyHealthStatus.is_healthy()` per pick. |
| `core.net.weft` proxy middleware | spawns `ActiveHealthChecker.run` into a nursery. |
| `core.net.proxy.upstream_pool` | release path consults health to drop unhealthy conns. |

## 2. Crate-side hardcodes

None — pure Verum. `AtomicBool` / `AtomicInt` / `MemoryOrdering`
are stdlib-side.

## 3. Language-implementation gaps

### §3.1 HC-1 — Active HTTP probe loop gated on `HealthProbe` impl

`HealthProbe.probe(&self, upstream, path, timeout_ms)` is a
protocol declared at lines 287-297. There's no in-tree default
HTTP-1.1 impl shipped with `health_check.vr` (the comment at
line 289 says "Real implementations live in callers so we don't
pull http_client into the proxy crate"). Functional surface
@ignore'd until a test-side `MockHealthProbe` lands.

### §3.2 HC-2 — `record_outcome` threshold semantics

Lines 203-234 implement the threshold-based mark-healthy /
mark-unhealthy transitions. Direct unit-testable but mutates
shared atomics — pinned via `regression_test.vr` LOCK-IN at
the construction surface only; mid-loop pin requires harness.

### §3.3 HC-3 — `passive_default()` constants

Lines 120-125: `Passive { unhealthy_threshold: 5,
recovery_interval_ms: 30_000 }`. LOCK-IN pinned in unit_test.

### §3.4 HC-4 — `HealthCheck.active(period_ms, path)` builder

Lines 110-118 fills `timeout_ms=1000`, `healthy_threshold=2`,
`unhealthy_threshold=3`. Default-constants pinned in unit_test.

## 4. Action items landed in this branch

* `core-tests/net/proxy/health_check/unit_test.vr` — 17 unit
  tests covering ProxyHealthStatus.new + is_healthy, HealthCheck
  3-variant disjointness, HealthCheck.active / passive_default
  field preservation, HealthCheckError Eq + disjointness.
* `core-tests/net/proxy/health_check/regression_test.vr` — 6
  regression pins (3 active LOCK-IN + 3 @ignore'd functional).
* `core-tests/net/proxy/health_check/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ProxyHealthStatus.mark_healthy / mark_unhealthy round-trip | this folder | 1h |
| ActiveHealthChecker.record_outcome threshold semantics | this folder | 2h |
| ActiveHealthChecker.tick with MockHealthProbe | this folder + mock probe | 4h |
| ActiveHealthChecker.run cancellation via stop() | this folder + async harness | 4h |
| Live HTTP probe against in-process upstream | language level | 1 day |
