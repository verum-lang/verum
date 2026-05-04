# Spindle Postgres Integration Tests

Live integration tests against `postgres:16-alpine` in Docker. Bring up,
run, tear down — one command:

```bash
bash vcs/integration/postgres/scripts/run.sh           # all tests
bash vcs/integration/postgres/scripts/run.sh t01_*.vr  # single glob
```

## Layout

```
vcs/integration/postgres/
├── docker-compose.yml      # postgres:16-alpine on 127.0.0.1:55432
├── conf/pg_hba.conf        # forces scram_sha_256 for every host
├── init/01-init.sql        # schema + roles created on first boot
├── scripts/run.sh          # up + wait + run + down
├── tests/conn_config.vr    # shared fixtures (host, port, roles)
├── tests/t01_*.vr          # individual integration tests
└── results/                # per-test stdout/stderr logs
```

## Container posture

- **Image**: `postgres:16-alpine` (~80 MB).
- **Port**: `127.0.0.1:55432` (loopback only — chosen to avoid
  clashes with a host-level Postgres install).
- **Auth**: `scram_sha_256` for every connection (md5 / trust /
  password fallback — rejected). Matches the production posture
  documented in spec §6.1.2.
- **Storage**: tmpfs — every `up` is a clean DB; `down -v` is
  destructive only on the volume the harness created.
- **Logging**: `log_statement=all` so the per-test log captures
  every statement Postgres saw.

## Roles

| Role | Powers | Used by |
|---|---|---|
| `spindle_admin` | DBA — DDL + everything | admin paths, cleanup |
| `spindle_app` | DML on test schema | typical handler path |
| `spindle_ro` | SELECT only | capability-narrowing tests |

Passwords are hard-coded in `init/01-init.sql` and `conn_config.vr`
— integration-only secrets, never used in production.

## Test naming

`tNN_<scenario>.vr` — sortable two-digit prefix so the runner walks
them in dependency order. Each test:

1. Opens a fresh connection via `conn_config.connect_*`.
2. Asserts pre-conditions via `expect_eq_int` / `expect`.
3. Runs the scenario.
4. Calls `pass("scenario name")` and exits 0; or
5. Calls `expect_*` which prints `FAIL: ...` and exits 1 on the
   first violated invariant.

## Adding a test

```verum
mount conn_config.{connect_app, expect_ok, expect_eq_int, pass};

fn main() {
    let conn = expect_ok(connect_app(), &"connect failed".clone());
    // ... scenario ...
    pass(&"my new scenario".clone());
}
```

## Tear down

`scripts/run.sh` always runs `docker compose down -v` on exit,
even when an individual test fails. Manual:

```bash
docker compose -f vcs/integration/postgres/docker-compose.yml down -v
```

## Future

Test list per `internal/specs/database.md` follow-up:

- `t01_handshake_scram.vr` — SCRAM-SHA-256 startup
- `t02_simple_query.vr` — text protocol round-trip
- `t03_extended_codec.vr` — Phase-2/3 codec round-trip
- `t04_pipeline.vr` — bounded pipeline + abort semantics
- `t05_listen_notify.vr` — pub/sub via NotifyDispatcher
- `t06_cancel.vr` — CancelRequest source-IP-bind
- `t07_transaction.vr` — begin/commit/rollback/savepoint
- `t08_pool.vr` — concurrent acquire + arena reset
- `t09_capability.vr` — Database<PgPool, DbRead> compile-rejects writes
- `t10_arena_simple_query.vr` — `simple_query_arena` returns rows whose
  bytes live in the connection arena; `reset_arena()` invalidates them
  via generation bump (spec §4.2 load-bearing)
- `t11_replication.vr` — logical replication V0 + pgoutput decode:
  IDENTIFY_SYSTEM → CREATE_REPLICATION_SLOT → START_REPLICATION →
  drain XLogData → decode_pgoutput → assert Begin/Relation/Insert/
  Commit observed; relation cache resolves Insert relid (spec §6.1.7)
- `t12_async_pool.vr` — AsyncPgPool fan-out: 32 concurrent tasks
  on 4 connections; wall-clock falsifies the "actor serialises
  callers" failure mode (spec §6.1.8 async surface)
- `t13_async_transaction.vr` — async typed Database<AsyncPgPool, _>:
  capability narrowing, AsyncPgTxScope round-trips (rollback +
  commit) via the actor (spec §5.4 / §5.5 / §6.1.8)
- `t14_physical_replication.vr` — physical-mode replication wire
  + BASE_BACKUP V0: connect_replication(physical) / IDENTIFY_SYSTEM
  / timeline_history / create_physical_slot / drop_slot;
  BASE_BACKUP {fast, manifest} drains every tablespace's CopyData
  chain + the manifest stream + RFQ (spec §6.1.7)
- `t15_async_copy.vr` — async streaming COPY FROM (1000 rows in
  10 chunks of 100 via AsyncCopyInWriter) + COPY TO
  (AsyncCopyOutReader; chunk-by-chunk drain); proves the actor
  sub-loop pattern works without buffering rows in memory
  (spec §6.1.6 / §6.1.8)
- `t16_prepared.vr` — sync + async prepared statements: PgConnection
  prepare / execute_prepared / close_prepared; AsyncPgConnection
  same flow via actor (spec §6.1.4 extended-query / prepared)
- `t17_async_replication.vr` — async logical replication V0:
  AsyncPgReplicationConnection identify_system / create_slot /
  start_logical / next_event / send_standby_status; verifies the
  reader actor uses timeout-bounded reads to alternate with
  pending standby-status writes (spec §6.1.7 / §6.1.8)
- `t18_typed_prepared.vr` — binary-format prepared params + result
  decode + TypedRow protocol; sync int4/int8/text/bytea round-trip
  + Maybe<Int> NULL handling + async int8 SUM (spec §6.1.4)
- `t19_typed_aggr.vr` — typed array + range round-trip: int4[]
  + text[] bound + decoded back; int4range / tstzrange decoded
  from server literals (spec §6.1.4 / §6.1.5 aggregate codecs)
- `t20_async_base_backup.vr` — async BASE_BACKUP V0:
  connect_and_start_async + tablespaces() pre-flight + drain
  BackupStreamEvent (BseTsChunk / BseTsEnd / BseManifestChunk /
  BseManifestEnd / BseFinished). Closes the LAST PG sub-protocol
  without async surface (spec §6.1.7 / §6.1.8)
- `t21_prepared_session.vr` — sync + async PreparedSession LRU:
  prepare_cached double-call asserts cache hit; query_cached
  round-trip; drain_cache_on_close issues Close(S, name) for
  every cached slot (spec §6.1.4 prepared cache)
- `t22_composite.vr` — composite/record binary round-trip:
  SELECT ROW(42, 'spindle', 1.5)::record decoded as TvComposite
  with 3 typed fields; CREATE TYPE smoke_pair (n int4, s text)
  + SELECT ROW(99, 'beta')::smoke_pair exercises user-defined
  composite path (spec §6.1.4 composite codec)

- `t23_async_pgoutput.vr` — pgoutput async consumer with relation-
  cache lifecycle: connect_replication_async + create slot +
  start_logical_pgoutput → drain CdcInsert events with their
  RelationMessage attached + assert insert_count ≥ 3 + relation
  cache populated (spec §6.1.7 / §6.1.8)
- `t24_async_pgoutput_buffered.vr` — streamed-tx grouping
  consumer V0: AsyncPgoutputBufferedConsumer wraps the raw
  consumer; verifies BcRegular pass-through path delivers
  CdcRelationOnly + 3× CdcInsert at proto_version=1 (no
  streaming). The full streamed-tx path (BcStreamedTx /
  BcStreamedAbort) is decoder-tested via the state machine;
  triggering a real streamed transaction requires
  `logical_decoding_work_mem` tuning out of V0 scope.
- `t25_pgoutput_typed.vr` — typed-tuple lift over async pgoutput:
  lift_change converts CdcInsert/Update/Delete TupleData into
  TpcInsert/Update/Delete with List<TupleValue> via per-column
  decode_typed_from_bytes against RelationMessage column OIDs;
  asserts (Int, Text, Int) tuples decoded correctly through
  TvLifted(TvInt4/TvText/TvInt8) (spec §6.1.7 + §6.1.4)
- `t26_async_listener.vr` — async LISTEN/NOTIFY end-to-end:
  AsyncPgListener actor on its own dedicated PgConnection;
  listen + subscribe; side-channel NOTIFY arrives on the
  subscriber's Receiver<Notification> with channel + payload +
  sender_pid populated (spec §6.1.6 + §6.1.8)
- `t27_async_copy_metrics.vr` — async COPY metrics + cancellation
  token: chunks_sent/bytes_sent/last_write_us populated after
  5 chunks; copy_in_async_with_token short-circuits subsequent
  writes after token.cancel() + bumps cancellations counter
  (spec §6.1.6 metrics + cancellation)
- `t28_async_cancel.vr` — server-side CancelRequest from async
  PG: AsyncPgConnection caches BackendKeyData at connect; spawn
  a task running `SELECT pg_sleep(10)` on the actor; from a
  separate task call cancel_running_query() → fresh-socket
  CancelRequest aborts the query within seconds (spec §6.1.2 +
  §6.1.8)
- `t29_async_auto_cancel.vr` — auto-fire CancelRequest on
  CancellationToken flip: attach_cancellation_token spawns a
  watcher; token.cancel() fires CancelRequest without caller
  intervention (spec §6.1.2 + §6.1.8 auto-wire)
- `t30_composite_registry.vr` — CompositeRegistry + named-field
  accessors: register schema for ROW(int4, text, int8); decode
  ROW(99, 'beta', 1234567890)::record; field_value_by_name
  pulls each cell by its declared name + asserts exact-value
  match (spec §6.1.4 composite registry)

Note on pgoutput streaming-tx variants ('S'/'E'/'c'/'A',
proto_version 2+): the decoder now recognises these tags
end-to-end (exercised through t11_replication.vr's pump if the
server emits them). Triggering a real streamed transaction
requires `logical_decoding_work_mem` lower than the test
transaction's size; that fixture isn't wired today so the
streaming path is decoder-tested via the existing pump.
