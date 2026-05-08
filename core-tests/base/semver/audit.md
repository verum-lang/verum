# Audit — `core/base/semver.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/semver.vr` (496 lines) |
| Tests | NEW — `unit_test.vr` (~150 LOC, parse/cmp/format), `property_test.vr` (~140 LOC, RFC compliance + total order) |
| Hardcodes in `crates/` | none — pure stdlib |

## §1  Known defect — cmp returns Int instead of Ordering

Documented in `core-tests/base/ordering/audit.md §1.1`:

```
core/base/semver.vr:327
public fn cmp(a: &SemVer, b: &SemVer) -> Int {
    // returns -1 / 0 / +1
}
```

This is a typing-discipline regression — should return `Ordering`.
Property tests still verify the contract via `< 0` / `> 0` / `== 0`
checks, but consumers can't chain via `.then()` / `.reverse()`.

**Action item (deferred):** change return type to `Ordering`.
Cross-cutting (call sites must update).

## §2  RFC 2.0.0 conformance

The semver.org §11 ordering examples are pinned in
`property_test.vr §F` as `@test_case` parametrised tests. If the
parser ever drifts from RFC, these break.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/semver/`
- [x]  `unit_test.vr` — canonical parse, prerelease, build metadata,
       invalid inputs (empty / missing components / non-numeric /
       leading zeros), cmp ordering, format round-trip
- [x]  `property_test.vr` — round-trip, cmp reflexivity / anti-symmetry /
       transitivity, component monotonicity, @test_case prerelease
       fixtures
- [x]  This audit document

## §4  Action items deferred

1. **cmp → Ordering migration** — typing-discipline fix; ~30 call sites.
2. **Build-metadata-equivalence rule** — RFC says build metadata
   MUST be ignored when determining precedence. Today's tests don't
   directly assert this; pin it.
3. **All 11 reference fixtures** from semver.org §11 — currently 7
   are pinned; complete the set.
4. **VERSION constant validation** — `core/base/mod.vr` defines
   VERSION; verify `parse(VERSION).is_ok()`.
