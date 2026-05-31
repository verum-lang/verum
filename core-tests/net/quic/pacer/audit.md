# core-tests/net/quic/pacer — audit

`core/net/quic/pacer.vr` — RFC 9002 §7.7 token-bucket pacer.

## Coverage (unit_test.vr)

| § | What | Tests |
|---|------|-------|
| 1 | `PacerDecision` Send / NotYet(Duration) variant discrimination (match-based) | 3 |

3 `@test`. PacerDecision has no Eq impl → match-based. Duration payload
constructed via `Duration.from_micros` / `from_secs` (pacer's own idiom).

## Deferred

- `Pacer.check()` token-accounting over `Instant` time — live behaviour;
  Duration payload-value extraction deferred (Duration single-field-record
  unboxing §G defect). Belongs at L2 timing spec level.
