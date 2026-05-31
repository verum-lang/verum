# core-tests/net/quic/lb_cid — audit
`core/net/quic/lb_cid.vr` — draft-ietf-quic-load-balancers routable CIDs.
| § | What | Tests |
|---|------|-------|
| 1 | `LbCidProfile` 4-variant (8_1/12_2/16_3/20_3), match | 4 |
| 2 | `LbCidError` unit variants (InvalidConfigByte/NoMatchingKey), Eq | 2 |
| 3 | `LbCidError` record-payload variants (InvalidServerIdLen/LengthMismatch) — ctor Eq + field extraction + distinct-payload (exercises CLASS-9/D2b) | 4 |
| 4 | unit-variant disjointness | 1 |
11 `@test`. Qualified Type.Variant. Encode/decode CID routing (AES key
schedule) deferred to L2 crypto specs.
