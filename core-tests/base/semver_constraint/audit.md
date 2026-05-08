# Audit — `core/base/semver_constraint.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/semver_constraint.vr` (665 lines) |
| Tests | NEW — `unit_test.vr` (~140 LOC, caret/tilde/exact/parse), `property_test.vr` (~110 LOC, reflexivity + window bounds + truth tables) |
| Depends on | `core/base/semver.vr` (parser for the underlying SemVer type) |

## §1  Operator semantics

The operators implemented match cargo / npm conventions:

| Operator | Meaning |
|---|---|
| `^1.2.3` | `>=1.2.3 <2.0.0` (caret — same major) |
| `~1.2.3` | `>=1.2.3 <1.3.0` (tilde — same minor) |
| `1.2.3` | exact match |
| `>=1.0` | min bound |
| `<2.0` | max bound |
| `*` / `x` | wildcard |

Reference: cargo's semver crate; npm's semver.

## §2  Pre-release matching is mode-dependent

`MatchOptions { allow_prerelease }` controls whether constraints
match prerelease versions of the matching range. By default, `^1.2.3`
does NOT match `2.0.0-beta` (prereleases excluded); with
`match_options_allow_prerelease()` it does.

**Action item (deferred):** add property tests covering both modes.
Today only `matches_default` is exercised.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/semver_constraint/`
- [x]  `unit_test.vr` — caret / tilde / exact semantics, parse syntax,
       parse-invalid, format round-trip
- [x]  `property_test.vr` — reflexivity (constraint matches its base),
       caret window upper bound, tilde window upper bound,
       @test_case exact-match truth table
- [x]  This audit document

## §4  Action items deferred

1. **Pre-release matching mode tests** — exercise
   `match_options_allow_prerelease()`.
2. **Range-intersection / union semantics** — if Verum supports
   `>=1.0 <2.0` compound constraints, pin the intersection rules.
3. **X-ranges** — `1.x` should equal `^1.0.0` etc.; pin equivalences.
4. **Cargo cross-test corpus** — port cargo's semver test suite for
   exhaustive operator coverage.
