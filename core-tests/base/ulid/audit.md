# Audit — `core/base/ulid.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/ulid.vr` (315 lines) |
| Tests | NEW — `unit_test.vr` (~80 LOC), `property_test.vr` (~70 LOC, round-trip + lex-order + uniqueness + alphabet) |
| Hardcodes in `crates/` | clock + RNG via runtime intrinsics |

## §1  Format

ULID = 48-bit Unix-ms timestamp + 80-bit randomness, encoded as
26-char Crockford Base32. Crockford alphabet excludes I, L, O, U
(visual-confusion characters); the spec says encoded text is
case-insensitive on parse but canonical-cased on emit.

Reference: https://github.com/ulid/spec.

## §2  Lexicographic time-order property

ULID's selling point: lexicographic sort = time sort. Property tested
in `property_test.vr`. Note that within the same ms, the order is
determined by the random tail — so we assert `<=` not `<`.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/ulid/`
- [x]  `unit_test.vr` — construction, from_parts, parse round-trip,
       parse-invalid, parse-wrong-length, Crockford alphabet,
       sequential ordering
- [x]  `property_test.vr` — parse round-trip on 100 samples,
       canonical length, lex-order non-decreasing, 1000-sample
       uniqueness, from_parts timestamp round-trip @property
- [x]  This audit document

## §4  Action items deferred

1. **Case-insensitive parse normalisation** — `parse("01H8...")`
   and `parse("01h8...")` must produce equal ULIDs. Pin this.
2. **Cross-platform timestamp consistency** — v7-style spec relies
   on Unix-ms; verify across platforms.
3. **Edge: timestamp = 2^48 - 1** — boundary of representable range.
4. **Crockford check digit** — standard ULIDs don't include a check
   digit; some libraries add one. If Verum's `parse` accepts
   non-standard variants, document.
