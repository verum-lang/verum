# Audit — `core/base/mod.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/mod.vr` (377 lines — re-export hub for `core.base.*`) |
| Tests | `core-tests/base/mod/` — `prelude_test.vr` (97 LOC, migrated), `unit_test.vr` (NEW, ~190 LOC, prelude re-exports + aliases + VERSION) |
| Hardcodes in `crates/` | re-export resolution paths — historically a hot defect surface |

## §1  Re-export propagation — known defect surface

Project memory documents multiple re-export bugs:
- `project_reexport_propagation_2026-05-08.md` (commit 094a64bb)
- `project_inline_short_circuit_coverage_gate_2026-05-08.md` (commit 79dd86d9)
- `project_inline_mount_reexport_2026-05-08.md` (commit 67694183)

The re-export propagation has been fragile. Tests in this folder
probe whether stdlib consumers can still see the canonical names
(Maybe, Result, Ordering, Heap, …) through the prelude. If any test
fails, it's likely a propagation regression.

## §2  Submodule visibility

`mod.vr` declares 24 public submodules:
```
ordering, ops, maybe, result, protocols, primitives, memory, iterator,
panic, env, data, cell, uuid, snowflake, nanoid, semver, glob, ulid,
string_distance, coercion, coinductive, error, log, serde
```

Each is a separate test target in `core-tests/base/<name>/`. The
audit-progress matrix is the natural inventory of which submodules
have full coverage.

## §3  Type aliases — drift surface

`mod.vr` defines:
- `Byte = UInt8`
- `Bytes = List<Byte>`
- `TextResult<T> = Result<T, Text>`
- `StdResult<T> = Result<T, Error>`

If `Result` is renamed or its parameter list changes, these aliases
silently break. Today they're tested in `unit_test.vr §2` with usage
witnesses — concrete typecheck-pass evidence is the contract.

## §4  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/prelude_types_test.vr` →
       `core-tests/base/mod/prelude_test.vr`
- [x]  Add `unit_test.vr` covering prelude re-exports (Maybe, Result,
       Ordering, collections, protocols, memory, iterator, panic),
       type aliases (Byte, Bytes, TextResult), VERSION constants
- [x]  Add this audit document

## §5  Action items deferred (not landed)

1. **Re-export propagation regression test suite** — the project memory
   shows recurring bugs in this area; we'd benefit from a focused test
   harness that probes every public name in the prelude. Currently
   covered only by usage-witness tests in `unit_test.vr §1-7`.
2. **Submodule visibility integration test** — for each submodule
   listed in `mod.vr`, verify it's reachable via `mount core.base.<x>`.
   Mechanical; ~30 LOC.
3. **VERSION constant validation** — currently we assert the constants
   exist and are non-empty. A more rigorous test would assert
   semver-format compliance (we have `semver.vr` parser; once
   `core-tests/base/semver/` lands, parse the VERSION string and
   verify it parses cleanly).
