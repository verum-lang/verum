# `net/proxy` audit

Module: `core/net/proxy/` (~7 files) — HTTP/HTTPS forward +
reverse proxy middleware umbrella:

* `circuit_breaker.vr` — closed / open / half-open state machine
  with consecutive-failure threshold + cooldown timer.
* `health_check.vr` — periodic probe + sliding window status.
* `loadbalancer.vr` — RoundRobin / Random / WeightedLeastConn
  strategies + healthy-only scan.
* `rate_limit.vr` — TokenBucket / LeakyBucket / SlidingWindow
  algorithms + KeyedRateLimiter.
* `retry.vr` — exponential backoff with jitter.
* `upstream_pool.vr` — connection-pool maintenance.

Tests cover the algebraic data-surface: LoadBalancer 3-variant
+ disjointness lattice, rate-limit cost constants.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` reverse-proxy middleware | wraps inner handler chain. |
| Forward-proxy clients | `Upstream` pool + retry. |
| Service mesh + sidecar | `proxy.*` is the building block. |

## 2. Crate-side hardcodes

None — pure Verum.

## 3. Language-implementation gaps

### §3.1 PROXY-1 — `LbPool.pick` / `TokenBucket.try_acquire` /
       `CircuitBreaker.allow` functional surface

Subject to precompile-cascade SIGSEGV class. Data-surface
algebra compiles; the picker / circuit-breaker / rate-limiter
state machines covered at L2 specs.

### §3.2 RFC 1928 SOCKS5 — currently absent

The umbrella documents HTTP CONNECT-tunnelling but lacks SOCKS5
client/server. Roadmap: add `socks5.vr` under proxy/.

### §3.3 RFC 2616 §14.31 `Proxy-Authorization` Negotiate scheme

Standard proxy authentication. Currently absent; deferred.

## 4. Action items landed in this branch

* `core-tests/net/proxy/unit_test.vr` — 8 unit tests covering
  LoadBalancer 3-variant + 2 disjointness checks +
  CHEAP_COST/HEAVY_COST cost constants + relative-order
  invariant.
* `core-tests/net/proxy/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| CircuitBreakerLayer record + state-machine variants | this folder | 2h |
| UpstreamEntry + LbPool round-trip with mock upstreams | this folder | 4h, gated on §3.1 |
| RateDecision variant Eq + disjointness | this folder | 1h |
| TokenBucket / LeakyBucket / SlidingWindow algorithm semantic tests | this folder | 1 day |
| HealthCheck periodic probe + sliding-window status | this folder + harness | 4h |
| Retry exponential-backoff jitter property | this folder | 2h |
| RFC 1928 SOCKS5 client/server | stdlib + tests | 1 week |
