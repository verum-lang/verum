# `proof/pcc` audit

Module: `core/proof/pcc.vr` (210 LOC) — Proof-Carrying Code
certificate bundle API. Defines GoalHash + ProofCertificate +
BundleMetadata + ProofBundle records plus the bundle_add /
bundle_lookup / bundle_size operations.

Tests: `unit_test.vr` (~23 unit tests covering constructors,
bundle accumulation, idempotent re-add of same hash, lookup
behaviour, size invariants).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.verify` | constructs proof bundles for each verified module. |
| `verum_compiler` | emits ProofBundle as a side-output for incremental verification caching. |
| Application audit tools | consume bundles to render verification reports. |

## 2. Crate-side hardcodes

`crates/verum_verification/src/...` likely emits ProofBundle from
the SMT/Z3 backend. The (compiler_version, source_path) tuple in
BundleMetadata is the audit-trail key; drift here invalidates
incremental verification cache.

## 3. Language-implementation gaps

### §3.1 No `bundle_remove(b, h)` operation

ProofBundle currently grows monotonically (add/lookup/size only).
For incremental verification (re-checking a module after edit),
the old certificates should be removed. Add:
* `bundle_remove(b: ProofBundle, h: GoalHash) -> ProofBundle`
* `bundle_clear(b: ProofBundle) -> ProofBundle`

**Effort:** small (~30 min) + 4 tests.

### §3.2 `BundleMetadata.total_duration_ms` may overflow

Sum of all certificate duration_ms across thousands of obligations
could exceed Int range (~292M years at 1ms each, but for batched
parallel verification the wall-clock-equivalent could be much
larger). Document the cap OR switch to BigInt. Likely fine for
typical workloads.

### §3.3 No `Display` / `Debug` for ProofBundle

Bundle debugging requires walking the certificates map manually.
Add Display showing `"bundle:{compiler}:{source} ({N} certs, {D}ms)"`
+ Debug rendering the full structure.

**Effort:** small (~30 min).

### §3.4 `bundle_add` mutates argument by value-return shape

The signature `bundle_add(b: ProofBundle, h, c) -> ProofBundle`
takes ownership and returns a new bundle. A mutating
`bundle_add_mut(&mut b, h, c)` variant would avoid the shallow
copy of bundle's map. Today the map insert is in-place so the
visible difference is mostly the metadata-record construction.

**Effort:** small (~30 min).

## Action items landed in this branch

* `core-tests/proof/pcc/unit_test.vr` — 23 unit tests covering
  the public surface including idempotent-replace semantics
  (the load-bearing invariant pinned at `pcc.vr:106-111`).
* `core-tests/proof/pcc/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `bundle_remove` + `bundle_clear` operations | `core/proof/pcc.vr` + tests | 30 min |
| Add `Display` / `Debug` for ProofBundle | `core/proof/pcc.vr` + tests | 30 min |
| Add `bundle_add_mut` variant | `core/proof/pcc.vr` + tests | 30 min |
| Switch `total_duration_ms` to BigInt OR document cap | decide first | 1h |
| Property test for replace-idempotence with multi-step sequences | this folder + property_test.vr | 30 min |
| Sister tests for `core.proof.{kernel_bridge,reflection,tactics}` | sister folders | 1 day each |
