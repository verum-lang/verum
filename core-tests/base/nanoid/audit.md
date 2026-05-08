# Audit — `core/base/nanoid.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/nanoid.vr` (171 lines) |
| Tests | NEW — `unit_test.vr` (~70 LOC), `property_test.vr` (~70 LOC, length + uniqueness laws + @property + @test_case) |
| Hardcodes in `crates/` | RNG source (CSPRNG via verum_codegen runtime intrinsic) |

## §1  RNG source

`generate()` requires CSPRNG-grade entropy. Source goes through
`core.intrinsics.runtime.crypto.random_bytes` which is implemented as
a platform shim: `getrandom(2)` on Linux, `arc4random_buf` on macOS
via libSystem, `RtlGenRandom` on Windows.

**Critical:** if the runtime intrinsic ever falls back to a non-CSPRNG
PRNG (e.g. seeded by a constant), nanoid's collision-resistance claim
silently breaks. Worth a periodic spot-check of the platform shim.

## §2  Spec compliance

Reference: https://github.com/ai/nanoid (the canonical spec). Verum's
default alphabet (A-Za-z0-9_-) and default length 21 match the spec.
Collision probability: ~1% chance after generating 10^21 ids
(per nanoid.dev calculator).

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/nanoid/`
- [x]  `unit_test.vr` — default length, custom length, alphabet membership,
       distinct calls
- [x]  `property_test.vr` — length invariant @property, uniqueness over 1000
       samples, default-length witnesses
- [x]  This audit document

## §4  Action items deferred

1. Cross-platform RNG-source verification — periodic CI check that
   the runtime CSPRNG intrinsic is wired correctly.
2. Custom-alphabet tests — generate_with_alphabet exposed but minimal
   coverage; pin alphabet-membership invariant for non-default
   alphabets.
3. Statistical entropy check — sample N IDs, verify byte-distribution
   is roughly uniform (chi-squared test). Loose because nanoid is
   not a PRNG benchmark.
