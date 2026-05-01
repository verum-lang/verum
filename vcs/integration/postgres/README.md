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
