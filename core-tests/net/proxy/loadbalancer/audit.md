# `net/proxy/loadbalancer` audit

Module: `core/net/proxy/loadbalancer.vr` (~164 LOC) — request
distribution strategies. Three variants:

* `RoundRobin` — fair under uniform load (shared atomic cursor).
* `Random` — caller-salted modulus pick.
* `WeightedLeastConn` — `in_flight × 1000 / weight` score with
  lowest-score wins. Mimics Envoy/HAProxy WLCDN.

The umbrella `core-tests/net/proxy/unit_test.vr` already covers
the LoadBalancer 3-variant + pairwise disjointness (8 tests).
This per-submodule suite extends with `UpstreamEntry` /
`LbPool` data-surface tests and pins the algebraic laws
(empty-pool → None, cursor monotonicity, WLC picks the lowest
in_flight).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.proxy` reverse proxy | `LbPool.pick(req_salt)` per request. |
| `core.net.weft` upstream selector | wraps `LbPool` behind a `Layer`. |
| Service mesh sidecar | per-cluster LbPool keyed on (service, zone). |

## 2. Crate-side hardcodes

None — pure Verum. `AtomicInt` / `MemoryOrdering` stdlib-side.

## 3. Language-implementation gaps

### §3.1 LB-1 — Empty pool returns None

Line 110: `if n == 0 { return Maybe.None; }`. Pinned in unit
tests by constructing `LbPool.new(_, List.new())` and asserting
`pick(0).is_none()`.

### §3.2 LB-2 — Functional surface gated on UpstreamEntry mocks

Real `pick` against a heterogeneous pool (mixed healthy /
unhealthy) requires constructing `Upstream` records — which
needs `TcpStream.connect` at acquire time. Functional surface
@ignore'd until a `MockUpstream` harness lands.

### §3.3 LB-3 — WeightedLeastConn picks lowest-score

Lines 142-163: `score = (in_flight × 1000) / weight`. Caller
of `with_weight(0)` would divide-by-zero, guarded by `weight <
1 -> 1` clamp at line 153. Algebraic-law @ignore'd pin.

### §3.4 LB-4 — RoundRobin cursor monotonicity

Line 119: `cursor.fetch_add(1, AcqRel) % n`. Caller-visible
invariant — successive `pick` calls on the same Lb cycle
through indices in order. @ignore'd until UpstreamEntry mocks.

## 4. Action items landed in this branch

* `core-tests/net/proxy/loadbalancer/unit_test.vr` — 15 unit
  tests: 3-variant construction (already covered in umbrella —
  re-pinned here for module-local granularity) + pairwise
  disjointness + Upstream / UpstreamEntry data-surface +
  LbPool.new + LbPool.pick(empty) → None.
* `core-tests/net/proxy/loadbalancer/regression_test.vr` — 7
  regression pins (3 active LOCK-IN + 4 @ignore'd algebraic).
* `core-tests/net/proxy/loadbalancer/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| RoundRobin cycle invariant | this folder + UpstreamEntry mock | 4h |
| Random pick within bounds | this folder + UpstreamEntry mock | 2h |
| WeightedLeastConn picks lowest in_flight | this folder + atomics | 4h |
| Maglev consistent-hash (phase 3) | stdlib + this folder | 1 week |
| power-of-two-choices variant | stdlib + this folder | 4h |
