# `net/weft` audit

Module: `core/net/weft/` (~25 files) — Verum's typed HTTP
framework. The largest sub-tree under `core/net/`: handlers +
extractors + middleware + routing + arena-pooled buffer /
connection management + HTTP/1.1 + HTTP/2 + HTTP/3 +
observability hooks + RPC + WebSocket adapter.

Tests cover the algebraic data-surface that is reachable from a
USER test module without triggering the precompile-cascade
SIGSEGV class:

* `WeftErrorCategory` 5-variant (ErrTransient /  ErrPermanent /
  ErrSecurity / ErrClient / ErrUpstream) + default_status
  StatusCode mapping (503/500/403/400/502) + is_retryable
  (Transient + Upstream only) + Eq + variant disjointness.

The full framework surface (Router, Handler, Middleware,
FromRequest, IntoResponse, WeftRequest extractors, http2/http3
adapters, arena pool, connection lifecycle) is at L2 specs
end-to-end harness because every entry point requires the
framework runtime + accept loop + arena.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http` | base types (Method/StatusCode/Headers/Request/Response). |
| `core.net.http_parser` | per-request HTTP/1.1 parse. |
| `core.net.{http2,h3,quic}` | HTTP/2 + HTTP/3 transport adapters. |
| `core.net.proxy.*` | reverse-proxy middleware composition. |
| `core.metrics.*` | request metrics + OTLP export. |
| `core.tracing.*` | per-request span. |
| Application services | every Verum HTTP server. |

## 2. Crate-side hardcodes

None — Weft is pure Verum, sitting on top of the V-LLSI
syscall layer + the `core.net.*` primitives.

## 3. Language-implementation gaps

### §3.1 WEFT-1 — Router / Handler dispatch + extractor pipeline

Subject to precompile-cascade SIGSEGV class. The framework
implements multi-protocol HTTP serving (HTTP/1.1 + HTTP/2 +
HTTP/3 unified) — each adapter has its own connection state
machine.

### §3.2 Refined routing (`refined_routes.vr`)

Verum's refined-type guarantees flow through to route
parameters via `PathParam<T>` extractor + `T: FromStr`. Tested
at language level (`vcs/specs/L3-extended/net/weft/`).

### §3.3 RPC (`rpc.vr`)

OpenAPI / gRPC-Web translation layer. Tested at L2 specs.

### §3.4 Arena-pooled buffer management

`arena_pool.vr` + `bufpool.vr` provide per-request memory
arenas + buffer pools. Critical for the < 150 ns/request
latency target.

## 4. Action items landed in this branch

* `core-tests/net/weft/unit_test.vr` — 20 unit tests covering
  WeftErrorCategory 5-variant + default_status (5 status code
  mappings) + is_retryable (Transient + Upstream true; others
  false — 5 tests) + Eq + variant disjointness (5).
* `core-tests/net/weft/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| WeftError 8+ variant Eq + Display rendering | this folder | 2h |
| ExtractRejection variant disjointness | this folder | 1h |
| Router.add + .match construction (no real request) | this folder | 4h, gated on §3.1 |
| PathParam / QueryParam / BodyBytes / BodyText extractor shape | this folder | 4h, gated on §3.1 |
| Middleware composition (Next + chain) | this folder | 4h, gated on §3.1 |
| End-to-end request/response cycle | language level | 1 week |
| HTTP/2 + HTTP/3 adapter coverage | language level | 1 week each |
| RPC OpenAPI translation | language level | 1 week |
