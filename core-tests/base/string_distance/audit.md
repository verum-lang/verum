# Audit — `core/base/string_distance.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/string_distance.vr` (258 lines) |
| Tests | NEW — `unit_test.vr` (~110 LOC, golden fixtures), `property_test.vr` (~120 LOC, metric axioms + Jaro range + JW≥J) |
| Hardcodes in `crates/` | none — pure stdlib |

## §1  Reference fixtures

- Levenshtein: `kitten/sitting=3`, `saturday/sunday=3` — these are
  textbook fixtures from Levenshtein 1965.
- Jaro: `MARTHA/MARHTA`, `DIXON/DICKSONX` from Jaro 1989.
- Jaro-Winkler: same inputs as Jaro plus prefix-bonus check.

## §2  Metric axioms

Levenshtein satisfies all four metric axioms (zero-iff-equal,
non-negative, symmetric, triangle inequality). Property tests verify
the first three exhaustively; triangle inequality on a representative
sample.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/string_distance/`
- [x]  `unit_test.vr` — Levenshtein golden fixtures (kitten/sitting,
       saturday/sunday, identical, empty), bounded variant, Jaro
       identity / no-overlap, Jaro-Winkler identity
- [x]  `property_test.vr` — Levenshtein zero-iff-equal /
       non-negative / symmetric / triangle inequality, Jaro range
       and symmetry, Jaro-Winkler ≥ Jaro
- [x]  This audit document

## §4  Action items deferred

1. **Unicode handling** — current API takes `&[Byte]`, not `&Text`.
   For Unicode strings, callers must convert via UTF-8. Pin the
   contract: `levenshtein(utf8(s1), utf8(s2))` matches "logical"
   character-distance for ASCII; for non-ASCII, the byte-distance
   may exceed the codepoint-distance.
2. **Performance contract** — Levenshtein is O(n*m); for long strings
   the bounded variant is preferred. Add a benchmark target in
   `vcs/benchmarks/`.
3. **`closest` autocorrect API** — exposed in `core/base/string_distance.vr:229`
   but minimally tested. Add a fixture for typical
   typo-suggestion scenarios.
