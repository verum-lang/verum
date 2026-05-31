# core-tests/net/quic/path — audit
`core/net/quic/path.vr` — RFC 9000 §8/§9 path validation + amplification limit.
| § | What | Tests |
|---|------|-------|
| 1 | `PathError` 3-variant ctors + Eq + pairwise disjointness | 4 |
| 2 | `TimerOutcome` 3-variant (NoAction/ShouldReChallenge/Failed), match | 3 |
7 `@test`. Qualified Type.Variant. Deferred: `PathState` record-payload
variants (`Validating { challenge: [Byte;8], sent_at: Instant, … }`) +
`QuicPath` state machine — fixed-array/Instant surface, L2 spec level.
