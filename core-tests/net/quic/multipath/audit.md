# core-tests/net/quic/multipath — audit
`core/net/quic/multipath.vr` — QUIC multipath extension path-id error surface.
| § | What | Tests |
|---|------|-------|
| 1 | unit variants (Eq) + 3 PathId-payload variant discrimination (match) | 5 |
| 2 | PathId payload value preservation | 1 |
| 3 | unit-variant disjointness | 1 |
7 `@test`. PathId = { value: UInt64 }. Qualified Type.Variant. Live
PathManager/scheduler deferred to L2.
