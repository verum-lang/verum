# core-tests/net/quic/address_token — audit

`core/net/quic/address_token.vr` — RFC 9000 §8.1 address-validation tokens
(Retry token + NEW_TOKEN), AEAD-sealed with a rotating key.

## Coverage (unit_test.vr)

| § | What | Tests |
|---|------|-------|
| 1 | `TokenError` 7-variant constructors + Eq reflexivity | 7 |
| 2 | `TokenError` pairwise disjointness (anti-diagonal cycle + seal-vs-all) | 2 |
| 3 | `QuicTokenKind` 2-variant (Retry / NewToken), match-based | 2 |
| 4 | `WIRE_VERSION` constant pin (= 1) | 1 |

12 `@test` total. Variants use the qualified `Type.Variant` form
(BAREVAR-ADT-1 discipline) since `TokenError`'s `Malformed` / `Expired`
names are not globally unique across the stdlib.

## Deferred

- `AddressTokenKey.generate()` + seal/open round-trip — depends on the
  AEAD key-material path (`[Byte; 16]` / `[Byte; 8]` fixed arrays) and
  `Instant`-based expiry; belongs at the L2 crypto-protocol spec level,
  not pure-data conformance.
- `TokenPlaintext` / `QuicTokenVerifyOptions` record round-trips — record
  field-access surface; pending the same fixed-byte-array handling.
