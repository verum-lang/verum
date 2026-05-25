# `mesh/k8s/gateway` audit

Module: `core/mesh/k8s/gateway.vr` (~93 LOC) — Gateway API v1
Gateway + GatewayClass ADTs.

Tests: 18 unit tests over ListenerProtocol 6-variant + TlsMode
2-variant + AddressType 3-variant + FromNamespaces 3-variant.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.k8s.client` | reads/writes Gateway resources via the K8s API |
| `core.mesh.k8s.httproute` | HTTPRoute.parent_refs point to Gateway via ParentReference (shared type) |
| `core.mesh.k8s.tlsroute` | TLSRoute likewise references Gateway listeners |

## 2. Crate-side hardcodes

| site | hardcode |
|---|---|
| `verum_runtime::k8s::gateway::v1::types` | ListenerProtocol 6-variant — directly maps to k8s.io/api `gateway.networking.k8s.io/v1` ProtocolType. Drift breaks API decode. |
| `verum_runtime::k8s::gateway::tls` | TlsMode 2-variant — Terminate / Passthrough wire values per Gateway API §2.4. |
| `verum_runtime::k8s::address_type` | AddressType 3-variant — IPAddress / Hostname / NamedAddress per Gateway API §2.7. |

## 3. Language-implementation gaps

### §3.1 Record-typed ADTs deferred

Gateway / GatewayClass / K8sListener / ListenerTls / FilterChain
etc. are record types (no enum variants) — testing requires either
direct record-literal construction or builder ctors. Both hit the
cross-module record-return defect class. Deferred until the
cross-module fix lands.

## Action items landed in this branch

* `core-tests/mesh/k8s/gateway/unit_test.vr` — 18 unit tests:
  ListenerProtocol 6-variant + 6-way disjointness + TlsMode
  2-variant + disjointness + AddressType 3-variant + disjointness +
  FromNamespaces 3-variant + disjointness.
* `core-tests/mesh/k8s/gateway/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Gateway / K8sListener record-construction tests (§3.1) | this folder | 1 h after cross-module fix |
| Integration test: round-trip Gateway → YAML → Gateway | this folder | 4 h |
| Drift-pinning Rust unit test for ListenerProtocol wire-value table | crates/verum_runtime/src/k8s/gateway.rs | 30 min |
