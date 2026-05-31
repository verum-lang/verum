# core-tests/net/quic/cid_pool — audit

`core/net/quic/cid_pool.vr` — RFC 9000 §5.1 connection-ID pool / issuer.

## Coverage (unit_test.vr)

| § | What | Tests |
|---|------|-------|
| 1 | `CidPoolError` 5-variant constructors + Eq reflexivity | 5 |
| 2 | payload preservation (LimitExceeded/DuplicateSequence) + distinct-payload | 4 |
| 3 | pairwise disjointness (anti-diagonal cycle) | 1 |

10 `@test`. Qualified `Type.Variant` form (BAREVAR-ADT-1).

## Deferred

- `CidPool` / `CidIssuer` / `CidEntry` record state machines
  (on_new_connection_id / on_retire_connection_id / pick_next_for_migration)
  — depend on `ConnectionId` + `[Byte; 16]` reset-token fixed arrays;
  belong at the L2 connection-lifecycle spec level.
