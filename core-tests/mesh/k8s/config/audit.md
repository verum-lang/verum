# `mesh/k8s/config` audit

Module: `core/mesh/k8s/config.vr` (~120 LOC) — kubeconfig +
in-cluster auth resolution ADTs.

Tests: 18 unit tests over KubeConfigError 5-variant + Eq matrix +
AuthInfo 4-variant.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.mesh.k8s.client` | every K8s API call resolves auth via `AuthInfo` |
| `verum_runtime::k8s::resolver` | the `@intrinsic("verum.k8s.load_default_kubeconfig")` impl reads ~/.kube/config + KUBECONFIG env var + in-cluster service-account token |

## 2. Crate-side hardcodes

* `verum_runtime::k8s::error` mirrors KubeConfigError 5-variant.
* `verum_runtime::k8s::auth` mirrors AuthInfo 4-variant. The
  `ExecPlugin` variant runs an external command for kubectl-style
  cred-helper integration (aws-iam-authenticator, gke-gcloud-auth-plugin).

## 3. Language-implementation gaps

### §3.1 Live load paths are intrinsics

`KubeConfig.load_default()` / `.load_context(path, context)` are
both `@intrinsic("verum.k8s.*")` with `async` semantics. At
runtime they read the FS + env vars; at the test layer they fall
through to stub responses. Live testing lives at the
`vcs/specs/L2-standard/mesh/k8s/` level.

### §3.2 KubeConfig record-builder paths

`KubeConfig.from_endpoint(cluster, auth)` is a cross-module
factory — same defect class as `meta/span` audit §3.1. The
`with_namespace` builder method on the returned record + the
`.cluster()` / `.auth()` / `.namespace()` accessors would all
hit field-access OOB. Deferred until the cross-module fix lands.

## Action items landed in this branch

* `core-tests/mesh/k8s/config/unit_test.vr` — 18 unit tests:
  KubeConfigError 5-variant ctor + 5-way disjointness + Eq
  reflexivity (NotFound) + payload-sensitivity (InvalidYaml,
  NoAuth, ContextNotFound) + cross-variant inequality + AuthInfo
  4-variant + 4-way disjointness.
* `core-tests/mesh/k8s/config/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| KubeConfig record-builder tests (§3.2) | this folder | 30 min after cross-module fix |
| Integration test: load_default with mock FS + env (§3.1) | vcs/specs/L2-standard/mesh/k8s/ | 4 h |
| Display / Debug rendering tests | this folder | 30 min after cross-module fix |
| Property test: AuthInfo dispatch table — every variant has a known credential-resolution path | this folder | 1 h |
