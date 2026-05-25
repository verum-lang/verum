# `mesh/xds/types` audit

Module: `core/mesh/xds/types.vr` (~172 LOC) — canonical xDS v3
TypeUrl constants + XdsNode identity + AdsAuth + AdsConfig.

Tests: 13 unit tests covering all 8 canonical Envoy v3 TypeUrl
constants (LISTENER, CLUSTER, ROUTE_CONFIGURATION,
CLUSTER_LOAD_ASSIGNMENT, SECRET, SCOPED_ROUTE_CONFIGURATION,
VIRTUAL_HOST, RUNTIME) + pairwise-distinctness pin +
envoy-namespace pin + AdsAuth 3-variant + variant disjointness.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.xds.client` | sends DiscoveryRequest with one of these type_urls |
| `core.mesh.xds.resources` | parsed_resource's `type_url` field comes back as one of these constants |
| `verum_runtime::envoy::ads` | wire-protocol mirror — these MUST match Envoy's protobuf descriptor canonical names |

## 2. Crate-side hardcodes

* Envoy's protobuf `Any.type_url` field uses these exact strings.
  Drift here breaks wire compat. The 8-test `test_type_url_<x>`
  series in `unit_test.vr` is the canonical drift pin.
* `verum_runtime::envoy::v3_typeurls` (planned) should mirror the
  set as a single source-of-truth `&[(&str, ResourceKind)]` table.

## 3. Language-implementation gaps

### §3.1 XdsNode / AdsConfig builder methods not tested

`XdsNode.new(id, cluster)` is a cross-module factory ctor — same
defect class as `meta/span` audit §3.1. The builder methods
`.with_locality()`, `.with_user_agent()`, `.with_metadata_json()`,
`.with_auth()`, `.with_use_delta()`, `.with_keepalive()` all hit
this path.

Workaround would be direct record literal at the test site, but
XdsNode has 8 fields with mixed `Maybe<Text>` — heavyweight.
Deferred until the cross-module fix lands.

### §3.2 ResourceName.xdstp ctor not tested

`ResourceName.xdstp(authority, type_, path)` constructs an
`xdstp://authority/type/path` resource name. Tested at the
language-level integration layer (vcs/specs/L2/mesh/) where the
Text builder fully works.

## Action items landed in this branch

* `core-tests/mesh/xds/types/unit_test.vr` — 13 unit tests over the
  8 canonical TypeUrl constants (each pinned to exact Envoy v3
  string) + pairwise-distinctness + envoy-namespace prefix pin +
  AdsAuth 3-variant + variant disjointness.
* `core-tests/mesh/xds/types/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| XdsNode builder methods + AdsConfig (§3.1) | this folder | 1 h after cross-module fix |
| ResourceName.xdstp round-trip | this folder | 30 min after cross-module fix |
| Drift-pinning Rust unit test for the 8 TypeUrl constants (§ crate-side) | crates/verum_runtime/src/xds/typeurls.rs | 30 min |
