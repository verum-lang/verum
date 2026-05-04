# Spindle MySQL Integration Tests

Live integration tests against `mysql:8.4-oracle` in Docker. Bring up,
run, tear down — one command:

```bash
bash vcs/integration/mysql/scripts/run.sh           # all tests
bash vcs/integration/mysql/scripts/run.sh t01_*.vr  # single glob
```

## Layout

```
vcs/integration/mysql/
├── docker-compose.yml      # mysql:8.4-oracle on 127.0.0.1:53306
├── init/01-init.sql        # roles + tables created on first boot
├── scripts/run.sh          # up + wait + run + down
├── tests/conn_config.vr    # shared fixtures (host, port, roles)
├── tests/t01_*.vr          # individual integration tests
└── results/                # per-test stdout/stderr logs
```

## Container posture

- **Image**: `mysql:8.4-oracle` (~600 MB).
- **Port**: `127.0.0.1:53306` (loopback only — avoids clashing with
  a host-level MySQL install).
- **Auth**: `caching_sha2_password` forced as the default plugin —
  matches the production posture documented in spec §6.3.
  `mysql_native_password` is rejected unless the connection carries
  `@allow_legacy_auth`.
- **Storage**: tmpfs — every `up` is a clean DB.
- **Logging**: general-log enabled (`/tmp/general.log` inside the
  container) so per-test failures can be inspected by exec-ing in.

## Roles

| Role | Powers | Used by |
|---|---|---|
| `spindle_admin` | DDL + DML | `connect_admin()` (capability tests) |
| `spindle_app` | DML on `spindle_test.*` | `connect_app()` (default app role) |
| `spindle_ro` | SELECT-only | `connect_ro()` (capability tests) |

All three authenticate via `caching_sha2_password` (the only
plugin Spindle accepts in the safe path).

## Tests

| File | Coverage |
|---|---|
| `t01_handshake.vr` | caching_sha2 fast-path; state, connection_id, server_version |
| `t02_simple_query.vr` | text-mode SELECT/INSERT/UPDATE/DELETE; affected_rows |
| `t03_prepared_codec.vr` | binary-protocol round-trip via PreparedSession LRU |
| `t04_transactions.vr` | begin / commit / rollback / savepoint |
| `t05_capability.vr` | `Database<MysqlPool, DbReadWriteTx>.into_readonly()` |
| `t06_pool.vr` | acquire/release accounting; total_arena_resets |
| `t07_async_transaction.vr` | async typed Database<AsyncMysqlPool, _> + AsyncMysqlTxScope rollback / commit (spec §6.3.6 async surface) |
| `t08_binlog.vr` | COM_BINLOG_DUMP_GTID + binlog event header + canonical event decoders (FORMAT_DESCRIPTION / QUERY / XID / GTID / ROTATE / HEARTBEAT) (spec §6.3.7) |
| `t09_binlog_rows.vr` | TABLE_MAP_EVENT + WRITE_ROWS_V2 type-aware tuple decoder; pairs RowsEvent with cached TABLE_MAP via `decode_with_table_map`; verifies INSERTs propagate as decoded RowValues (spec §6.3.7 row events) |
| `t10_async_binlog.vr` | AsyncMysqlBinlogStream actor — connect_binlog_async + next_event_message drains canonical events from a side-channel INSERT (spec §6.3.6 + §6.3.7 async surface) |
| `t11_typed_row.vr` | MysqlTypedRow protocol + NEWDECIMAL canonical-text decoder; rows_typed_my::<(Int, Text, Text)> lifts INSERT'd rows including DECIMAL(12,4) values "1234.5678" / "-9.0001" (spec §6.3.7 typed lift) |
| `t12_binlog_json.vr` | MySQL binary-JSON decoder via binlog ROW event: INSERT JSON_OBJECT with scalars/array/nested object/null/true/false/100000 → decoded canonical JSON text contains every expected key+value (spec §6.3.7 JSON decode) |
| `t13_async_load_data.vr` | async LOAD DATA LOCAL INFILE: stream 1000 TSV rows in 10 chunks of 100 via AsyncMysqlInfileWriter; assert affected_rows == 1000 + verify persistence via COUNT(*) (spec §6.3.6 + §6.3.8) |
| `t14_binlog_enum_set.vr` | TABLE_MAP optional metadata (binlog_row_metadata=FULL) + ENUM/SET decoders: ENUM('small','medium','large') decoded as RvText("small"); SET('alpha','beta','gamma','delta') with bits {alpha,gamma} decoded as RvText("alpha,gamma"); column_names + enum_values + set_values populated on TableMapEvent (spec §6.3.7.1) |
| `t15_multi_query.vr` | sync + async multi-statement COM_QUERY: INSERT + SELECT LAST_INSERT_ID() + UPDATE in one round-trip; 3 result-sets in declaration order with correct affected_rows / last_insert_id / row count (spec §6.3.3 multi-statement) |
| `t16_binlog_geometry.vr` | WKB → GeoJSON decoder: ST_GeomFromText('POINT(1 2)') decoded as `{"type":"Point","coordinates":[…]}` JSON text via binlog ROW event; SRID surfaced via GeoJSON `crs` member when non-zero (spec §6.3.7 GEOMETRY decode) |
| `t17_async_cancel.vr` | server-side KILL QUERY from async MySQL: AsyncMysqlConnection caches connection_id at connect; spawn `SELECT SLEEP(10)` on the actor; from a separate task call cancel_running_query() → fresh side conn issues `KILL QUERY <id>` → query aborts within seconds with ER_QUERY_INTERRUPTED (spec §6.3.6 + §6.3.9) |
| `t18_async_prepared.vr` | async prepared statements + PreparedSession LRU: direct prepare/execute_prepared/close_prepared round-trip; AsyncMysqlPreparedSession.prepare_cached double-call asserts cache hit + len stays 1; query_cached round-trip; drain_cache_on_close releases slot (spec §6.3.4) |

## Adding tests

1. Drop a `tests/tNN_<name>.vr` whose `main()` exits 0 on success,
   non-zero on failure (use `expect`/`expect_ok`/`expect_eq_int`
   from `conn_config.vr`).
2. End with `pass(&"tNN_name".clone())`.
3. To skip on a missing prerequisite, write `eprint("@skip: <reason>")`
   then exit non-zero — `run.sh` recognises the `@skip:` marker and
   reports it without failing the suite.

## Cleanup

```bash
docker compose -f vcs/integration/mysql/docker-compose.yml down -v
```

`scripts/run.sh` already runs this on exit (success or failure).
