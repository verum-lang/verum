# Spindle integration test rig

Brings up Postgres 14 + 16 and MySQL 8 in containers so the Spindle
adapters can run end-to-end integration tests. SQLite is in-process
and needs no rig.

## Prerequisites

* Docker 24.x or newer (uses Compose v2 syntax).
* macOS / Linux. Container ports bind to `127.0.0.1` so they don't
  leak to the LAN, and they are off the standard ports so a developer's
  local Postgres / MySQL service is not stomped on.

## Ports

| Service | Internal | Host | Auth |
|---|---|---|---|
| pg16   | 5432 | 55432 | scram-sha-256 |
| pg14   | 5432 | 55434 | scram-sha-256 |
| mysql8 | 3306 | 53306 | caching_sha2_password |

User: `spindle` / password: `spindle_pw_local_only` (development only,
*never* used outside this rig).

## Targets

```sh
make up      # start, block until all healthchecks pass
make ps      # current state
make logs    # tail logs
make test    # set env vars, run vcs/specs/L2-standard/database/{postgres,mysql}/
make down    # stop + remove (volumes are anonymous so state goes too)
```

`make test` exports the connection-string env vars below before invoking
`vtest`. The adapter test files gate on `VERUM_DB_INTEGRATION=1`; without
the rig running, they skip with a `@requires` directive.

| Variable | Used by |
|---|---|
| `VERUM_DB_INTEGRATION=1` | gate on the rig being live |
| `PGHOST` / `PGPORT` / `PGUSER` / `PGPASSWORD` / `PGDATABASE` | postgres adapter |
| `MYSQL_HOST` / `MYSQL_PORT` / `MYSQL_USER` / `MYSQL_PASSWORD` / `MYSQL_DATABASE` | mysql adapter |

## CI

CI is expected to:

1. Cache the Docker layer for the three images.
2. `make up` (the `--wait` on `compose up` blocks until healthchecks
   pass — no application-side polling needed).
3. `make test` to run the adapter VCS suite.
4. `make down` on teardown.

The rig is idempotent — `make up` after a partial state will reconcile.
