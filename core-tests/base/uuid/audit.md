# Audit — `core/base/uuid.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/uuid.vr` (277 lines) |
| Tests | NEW — `unit_test.vr` (~80 LOC), `property_test.vr` (~80 LOC, round-trip + version + uniqueness) |
| Hardcodes in `crates/` | RNG (CSPRNG) for v4; clock for v7 |

## §1  RFC compliance

Reference: RFC 4122 (UUIDv1-5) + RFC 9562 (UUIDv6-8). Verum
implements v4 (random) and v7 (time-ordered).

Canonical form: `xxxxxxxx-xxxx-Mxxx-Nxxx-xxxxxxxxxxxx` where M is
the version (1-8) and N's high bits are 10 (variant 1, RFC 4122).

## §2  v7 monotonicity

v7 prepends a 48-bit Unix-ms timestamp, making lexicographic sort
match time order. Property test pinned. **Caveat:** within the same
ms, v7 IDs may not be strictly monotone (random tail differs); the
test asserts `<=` not `<`.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/uuid/`
- [x]  `unit_test.vr` — nil format, v4/v7 version, canonical form
       length, parse/text round-trip, parse-invalid, bytes round-trip
- [x]  `property_test.vr` — round-trip on 50 v4 + 50 v7 samples,
       version pins, 1000-sample uniqueness, canonical-length law
- [x]  This audit document

## §4  Action items deferred

1. **Variant bits validation** — pin RFC 4122 variant (high bits 10).
2. **MAC-based v1 / namespace v3/v5** — not implemented in `uuid.vr`;
   would be a feature-add.
3. **Cross-platform timestamp consistency** — v7 timestamp must be
   Unix-ms; verify across platforms.
4. **Edge case: ms-boundary ordering** — when two v7 IDs span an
   ms boundary, ordering should hold. Specific test deferred.
