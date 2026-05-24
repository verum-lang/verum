# `cog/manifest` audit

Module: `core/cog/manifest.vr` (~635 LOC) — Cog (package)
manifest parsing + types, mirroring Cargo.toml shape.

Tests: 33 unit tests covering LanguageProfile 3-variant +
language_profile_tag round-trip + parse_language_profile
case-sensitive matching + GitRevision 4-variant +
DependencySource 4-variant + ManifestError 9-variant.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.cog.resolve` | parses dependency tree from manifests. |
| `core.cog.archive` | reads manifest from .car archive. |
| `vrm build` CLI | drives compilation from CogManifest. |

## 2. Crate-side hardcodes

`crates/verum_compiler/src/manifest/` Rust-side mirror types
MUST agree with the variant set + field ordering here. Schema-
drift surfaces as cog-load failures.

## 3. Language-implementation gaps

### §3.1 Round-trip parse_text + format_text test

Encode → decode → encode invariant test for a small canonical
manifest. Requires `core.configuration.toml.parse` to be wired
in --interp mode (currently AOT-gated by toml format adapter
registration).

**Effort:** moderate (~1h once toml adapter is interp-safe).

### §3.2 Property test on cog name refinement

CogName has a refinement `self.matches("^(@[a-z0-9]…)$")` —
property test should generate names and verify accept/reject.
Requires panic_contains test attribute (task #34) for the
reject side.

### §3.3 Sister tests for cog.resolve / cog.archive / cog.sign

Full cog distribution suite. Multi-day.

## Action items landed in this branch

* `core-tests/cog/manifest/unit_test.vr` — 33 unit tests over
  LanguageProfile + GitRevision + DependencySource +
  ManifestError.
* `core-tests/cog/manifest/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Round-trip parse/format test | this folder | 1h (gated on toml interp) |
| CogName refinement property test | this folder | gated on #34 |
| Sister tests for `core.cog.{resolve,archive,sign}` | sister folders | 1 week total |
