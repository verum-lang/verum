# `mesh/xds/error` audit

Module: `core/mesh/xds/error.vr` (~100 LOC) — error ADT for the
xDS v3 (Aggregated Discovery Service) control-plane client.

Tests: 26 unit tests covering XdsError 8-variant + variant
disjointness (4 pairs) + Eq reflexivity (6 cases) + Eq
payload-sensitivity (7 cases) + cross-variant inequality (3 cases).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.xds.client` | every `XdsClient.recv_response()` returns `Result<XdsResource, XdsError>` |
| `core.mesh.xds.resources` | parser produces `XdsError.InvalidResource` on protobuf decode failure |
| `core.security.spiffe` | mTLS handshake failure surfaces as `XdsError.AuthFailed` |
| `verum_runtime::xds_dispatch` | runtime side maps gRPC `Status` codes to XdsError variants |

## 2. Crate-side hardcodes

* `verum_runtime::xds::error_codes` mirrors the 8-variant set.
  Adding a variant requires changes in 3 places (this file, the
  runtime mapping, the gRPC code translator).
* `verum_compiler::auth_oracle_discipline` lists `AuthFailed` as
  one of the "no payload detail" surface candidates — the
  `Display` impl SHOULD NOT surface mTLS / SPIFFE check details
  to avoid oracle-attack vectors. Pinned by the Display impl in
  `error.vr:43` (`f.write_str("xDS authentication failed")`)
  which contains NO payload Text.

## 3. Language-implementation gaps

### §3.1 Cross-module record-return defect (low impact here)

The 6 Eq-reflexivity tests construct variant values **directly at
the test site** via `XdsError.NotFound(Text.from("X"))` — same
discipline as `core-tests/meta/span` (see meta/span audit §3.1).
Avoids the cross-module `.new(...)` factory path entirely.

### §3.2 Display / Debug not tested at this layer

Display + Debug impls in `error.vr:38-82` produce Text via `f"…"`
format strings, which currently fall through to Text construction
methods that hit the cross-module record-return defect on Text
returns. Pinned for a later integration test layer once
[`core.text`] / `f"…"` paths stabilise.

## Action items landed in this branch

* `core-tests/mesh/xds/error/unit_test.vr` — 26 unit tests over
  XdsError 8-variant + Eq reflexivity + payload-sensitivity +
  cross-variant inequality + AuthFailed-no-detail discipline.
* `core-tests/mesh/xds/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Display / Debug rendering tests (§3.2) | this folder | 1 h after cross-module fix |
| Integration test: gRPC `Status.Code` ↔ XdsError variant table | this folder + verum_runtime/tests | 2 h |
| Drift-pinning Rust unit test mirroring the 8-variant set | crates/verum_runtime/src/xds/tests.rs | 30 min |
| Property test: Eq is reflexive / symmetric / transitive across all 8 variants × representative payloads | this folder | 1 h |
