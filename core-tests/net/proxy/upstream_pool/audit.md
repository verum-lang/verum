# `net/proxy/upstream_pool` audit

Module: `core/net/proxy/upstream_pool.vr` (~310 LOC) — per-origin
TCP connection pool with reuse, bounded capacity, idle-timeout
expiration, and per-host limit.

* `Upstream` — record `(scheme, host, port, name, weight)`.
  Constructed by `Upstream.new(scheme, host, port)` which sets
  the human label to `f"{scheme}://{host}:{port}"` and weight=1.
  Builders `.with_weight(w)` / `.with_name(n)`.
* `UpstreamPool` — record holding `Shared<PoolState>` (private
  `PoolState { config: HttpPoolConfig, idle: Mutex<Map<...>> }`).
  `UpstreamPool.new(config)` + `UpstreamPool.default()`.
* `ConnectionLease` — RAII guard with Maybe<TcpStream> + owning
  upstream key + weak pool reference. Drop returns the conn iff
  still healthy.

Functional surface (`acquire` / `idle_count` / Drop release)
requires `TcpStream.connect` — gated on harness availability.
The data-surface tests here pin the record shape + the Eq /
Clone behavior + the `addr()` / `is_tls()` accessors.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.proxy.loadbalancer` | `Upstream` instances backing `UpstreamEntry`. |
| `core.net.proxy` reverse proxy | `UpstreamPool.acquire(&upstream).await`. |
| `core.net.weft` upstream client | per-process pool sharing connections. |

## 2. Crate-side hardcodes

`crates/verum_runtime` — `TcpStream.connect` syscalls (no libc).
The `HttpPoolConfig` default values (max idle 100, max per host
10, idle timeout 90s) live in `core/net/http/` — not in this
module. Roundtripped indirectly via `UpstreamPool.default()`.

## 3. Language-implementation gaps

### §3.1 UP-1 — `acquire` requires `TcpStream.connect` socket fixture

Lines 232-247 of upstream_pool.vr — `acquire` opens a fresh
TcpStream on cache miss. Without an in-process loopback fixture
or a UNIX socketpair shim, every acquire would attempt a real
DNS+TCP. @ignore'd in regression_test.

### §3.2 UP-2 — `Drop for ConnectionLease` release path

Lines 282-310 — drop returns the connection to the bucket via
`pool.idle.lock()`. Real release requires acquire + drop chain;
data-surface tests cover the lease record shape only.

### §3.3 UP-3 — `Upstream.addr()` format

Line 93 — `f"{self.host}:{self.port}"`. Pinned via direct
equality check in unit test. Drift here would surface as
`TcpStream.connect("h:port")` parse failures.

### §3.4 UP-4 — `Upstream` Eq ignores `name` and `weight`

Line 116-122 — equality compares only `(scheme, host, port)`.
`name` is decorative (logged / metric label); `weight` is
load-balancer specific. Pinned in unit test.

### §3.5 UP-5 — `UpstreamPool.default()` calls `HttpPoolConfig.new()`

Line 226-228 — `UpstreamPool.default()` delegates to
`HttpPoolConfig.new()`. The defaults live in
`core/net/http/`; we pin only that `.default()` constructs
successfully without panic.

## 4. Action items landed in this branch

* `core-tests/net/proxy/upstream_pool/unit_test.vr` — 18 unit
  tests: Upstream.new field preservation + default weight=1 +
  with_weight / with_name builder overrides + addr() format +
  is_tls() http/https + Clone preservation + Eq ignores
  name/weight + UpstreamPool.default() construction.
* `core-tests/net/proxy/upstream_pool/regression_test.vr` — 7
  regression pins (3 active LOCK-IN + 4 @ignore'd functional).
* `core-tests/net/proxy/upstream_pool/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| UpstreamPool.acquire against loopback fixture | this folder + socket harness | 1 day |
| ConnectionLease drop returns connection to pool | this folder + harness | 4h |
| ConnectionLease.close drops without re-pooling | this folder + harness | 2h |
| idle_count after acquire+release cycle | this folder + harness | 4h |
| Per-host bound (max_per_host) enforced on release | this folder + harness | 4h |
| Idle-timeout expiry drops stale conns on next acquire | this folder + clock fixture | 4h |
