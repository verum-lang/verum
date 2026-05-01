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
