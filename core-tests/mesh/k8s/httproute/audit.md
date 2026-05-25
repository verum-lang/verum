# `mesh/k8s/httproute` audit

Module: `core/mesh/k8s/httproute.vr` (~120 LOC) — Gateway API v1
HTTPRoute ADTs.

Tests: 14 unit tests over PathMatchType 3-variant + HeaderMatchType
2-variant + HttpPathModifier 2-variant + SessionType 2-variant +
per-ADT disjointness.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.k8s.client` | parses HTTPRoute resources from K8s API |
| `verum_runtime::k8s::route_match` | dispatches incoming requests via PathMatchType + HeaderMatchType |
| `verum_runtime::k8s::session` | SessionPersistence-typed.session_type drives the session-affinity mode |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_runtime::k8s::httproute::v1::types` | PathMatchType 3-variant (Exact / PathPrefix / RegularExpression) — Gateway API §2.6.3 ProtocolType. |
| `verum_runtime::k8s::httproute::filters` | HttpRouteFilter 6-variant (RequestHeaderModifier / ResponseHeaderModifier / RequestRedirect / RequestMirror / URLRewrite / ExtensionRef). Each is one of the 6 standard HTTPRoute filter types. |

## 3. Language-implementation gaps

### §3.1 HeaderMatchType variant-naming workaround

The source declares variants as `HExact` / `HRegularExpression`
prefixed with `H` to avoid colliding with `PathMatchType.Exact` /
`.RegularExpression` (same module). This is the same
**bare-variant cross-module collision** workaround as `Scope.Transient`
and `RouteAction.ForwardCluster` — pinned by task #17/#39.

The audit flags this as evidence the task #17/#39 close-out is
still load-bearing: every new module ships with deliberate name
disambiguation to dodge the dispatch defect.

### §3.2 HttpRouteFilter 6-variant record-payload tests deferred

Each variant carries a complex record payload (4-5 fields with
`List<(Text, Text)>` etc). Direct record construction at the test
site is feasible but heavyweight; deferred.

## Action items landed in this branch

* `core-tests/mesh/k8s/httproute/unit_test.vr` — 14 unit tests:
  PathMatchType 3-variant + disjointness + HeaderMatchType 2-variant
  + disjointness + HttpPathModifier 2-variant + disjointness +
  SessionType 2-variant + disjointness.
* `core-tests/mesh/k8s/httproute/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| HttpRouteFilter 6-variant payload tests (§3.2) | this folder | 1 h |
| Integration test: HTTPRoute YAML → HTTPRoute → dispatch | this folder | 4 h |
| Drift-pinning Rust unit test for PathMatchType / HeaderMatchType wire values | crates/verum_runtime/src/k8s/httproute.rs | 30 min |
