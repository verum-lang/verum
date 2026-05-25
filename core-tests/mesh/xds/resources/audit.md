# `mesh/xds/resources` audit

Module: `core/mesh/xds/resources.vr` (~184 LOC) — typed views over
Envoy xDS v3 LDS / CDS / RDS / EDS resources.

Tests: 32 unit tests covering 5 ADTs (ParsedResource +
ClusterDiscoveryType + LbPolicy + RouteMatch + RouteAction +
HealthStatus) + per-ADT variant disjointness sets.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.xds.client` | `XdsClient.parse_resource()` returns one of the 4 `ParsedResource` variants |
| `verum_runtime::xds::dispatch` | `RouteAction` variants drive runtime traffic-routing decisions |
| `verum_runtime::health::propagate` | `HealthStatus` updates flow through cluster-load-assignment subscriptions |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_runtime::envoy::types` | mirrors `ClusterDiscoveryType` 5-variant (Static/StrictDns/LogicalDns/Eds/OriginalDst) — the runtime DNS resolver dispatch depends on this set. |
| `verum_runtime::lb::policies` | mirrors `LbPolicy` 6-variant — each variant maps to a Rust-side load-balancer impl. |
| `verum_runtime::router::action` | mirrors `RouteAction` 4-variant — drives the request dispatch. `ForwardCluster` is the hot path. |
| `verum_runtime::health::status` | mirrors `HealthStatus` 5-variant — directly compatible with Envoy HealthStatus enum wire values. |

## 3. Language-implementation gaps

### §3.1 RouteAction variant naming discipline

The source file at `resources.vr:144` declares the forward-to-cluster
variant as **`ForwardCluster(Text)`** rather than the natural
`Cluster(Text)` to avoid colliding with the top-level `Cluster` type
declared in the same module:

```verum
public type RouteAction is
    /// Forward to a named Cluster (by cluster_name). Named `ForwardCluster`
    /// rather than `Cluster` to avoid colliding with the top-level `Cluster`
    /// type export — Verum flattens variant names into the module namespace.
    | ForwardCluster(Text)
    ...
```

This is a workaround for the same **bare-variant cross-module
collision** class as `Scope.Transient` / `core.net.http.Request`
(task #17/#39). The audit doesn't pin this as a new defect — it's a
known workaround — but flags it as evidence the close-out is still
load-bearing.

### §3.2 ParsedResource record-payload variants not exercised

ParsedResource's `L(XdsListener)`, `C(Cluster)`, `R(RouteConfiguration)`,
`E(ClusterLoadAssignment)` variants each carry a complex nested
record. Constructing these at the test site is feasible but heavy
(20+ field literals each). Deferred to a separate integration suite
once the test-helper module pattern (e.g. `make_listener_default()`)
lands.

## Action items landed in this branch

* `core-tests/mesh/xds/resources/unit_test.vr` — 32 unit tests over
  ParsedResource 5-variant (Opaque only — see §3.2) +
  ClusterDiscoveryType 5-variant + LbPolicy 6-variant +
  RouteMatch 4-variant + RouteAction 4-variant +
  HealthStatus 5-variant — every set with pairwise-disjointness pin.
* `core-tests/mesh/xds/resources/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ParsedResource record-payload variants (§3.2) | this folder | 2 h with test helpers |
| Integration test: parse a fixture protobuf → ParsedResource → assert variant + key fields | this folder | 4 h once protobuf round-trip is testable |
| Drift-pinning Rust unit test for HealthStatus wire-value compat with Envoy | crates/verum_runtime/src/health/tests.rs | 30 min |
| Property test: `dual(dual(LbPolicy)) == LbPolicy` (if a `dual` operation is added) | this folder | n/a — depends on future API |
