# `net/http` audit

Module: `core/net/http.vr` (1273 LOC) — HTTP/1.x primitives.
Defines Method 9-variant + Version 4-variant + StatusCode record
+ Request/Response records + HttpError + the HttpClient/
HttpServer protocols.

Tests focus on `Method`: variant construction, as_str canonical
naming, from_str case-insensitive parser, is_safe / is_idempotent
/ has_body RFC 7231/7230 semantic predicates.

Full Request/Response/StatusCode/Version coverage deferred — these
involve more complex records with header maps + body buffers.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2` | HTTP/2 reuses the same Method/StatusCode/Header types. |
| `core.net.http3` | HTTP/3 reuses ditto. |
| `core.mesh.xds` | Envoy xDS routes specify HTTP Method matching. |
| `core.cli` (HTTP demo) | uses Method.Get for `verum get-url`. |
| Application HTTP services | every endpoint declares Method. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/...` HTTP client (when implemented)
emits wire-format method names from Method.as_str() — drift here
would produce non-RFC-compliant HTTP. Pinned by `test_as_str_*`.

## 3. Language-implementation gaps

### §3.1 Method.as_str / from_str / is_safe / is_idempotent / has_body
       use BARE-NAME match arms

Pre-fix `Get / Head / Post / ...` patterns vulnerable to task #17/#39
first-wins collision (likely with `Request.Get` ctor or any sibling
type's matching variant). Not fixed in this commit because the test
surface already exercises the runtime behaviour correctly — but
flagged for the same qualified-arm sweep applied to compress/archive/
protobuf in this session.

**Effort:** trivial (~20 min) — same find/replace template.

### §3.2 Method has no `Hash` impl

`Map<Method, EndpointHandler>` lookups need Hash. Add following the
@derive(Eq, Hash) pattern.

**Effort:** trivial.

### §3.3 Method.as_str returns owned Text (allocates)

Hot-path consumers (HTTP serializer emitting Method per request)
allocate a fresh Text each call. Switch to `&'static Text` like
`compress.Algorithm.content_encoding` does, OR cache as constants.

**Effort:** small (~30 min).

### §3.4 No `Method.is_cacheable()` per RFC 7234 §3

GET + HEAD are cacheable by default; POST is conditionally
cacheable. Add the predicate following the is_safe pattern.

**Effort:** trivial (~10 min).

## Action items landed in this branch

* `core-tests/net/http/unit_test.vr` — 44 unit tests over Method
  9-variant construction + 9 as_str canonical names + 5 from_str
  parser cases (uppercase / lowercase / mixed-case / unknown /
  empty) + 9 is_safe + 9 is_idempotent + 8 has_body predicates.
* `core-tests/net/http/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Qualify Method.as_str / from_str / is_* match arms | `core/net/http.vr` | 20 min |
| Add Hash impl for Method | `core/net/http.vr` + test | 10 min |
| Switch Method.as_str to &'static Text | `core/net/http.vr` + caller migration | 30 min |
| Add Method.is_cacheable() per RFC 7234 §3 | `core/net/http.vr` + 9 tests | 30 min |
| Sister tests for StatusCode / Version / Request / Response / HttpError | this folder | 1 day |
| Cover http2/http3/h3/dns/cidr/addr/proxy/parser/range/cache/link_header | sister folders | 1 week total |
EOF
