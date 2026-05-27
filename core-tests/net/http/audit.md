# `net/http` audit

Module: `core/net/http.vr` (~1285 LOC) — HTTP/1.x primitive
types + protocols. Largest single file in `core/net/` outside
of `dns.vr`, `tcp.vr`, `udp.vr`, `unix.vr`, `tls.vr`.

Tests cover the algebraic surface end-to-end:

* `Method` 9-variant — `unit_test.vr` (RFC 7231 + RFC 5789):
  Get / Head / Post / Put / Delete / Connect / Options / Trace /
  Patch + as_str canonical wire-format names + from_str
  case-insensitive parser (incl error path) + is_safe (§4.2.1)
  + is_idempotent (§4.2.2) + has_body (RFC 7230 §3.3).
* `StatusCode` 100-599 — `status_test.vr`: new + code + 8
  factory ctors (ok/created/no_content/bad_request/unauthorized/
  forbidden/not_found/internal_server_error) + 5 categorization
  predicates (is_informational 1xx, is_success 2xx,
  is_redirection 3xx, is_client_error 4xx, is_server_error 5xx)
  + pairwise-disjoint category lattice + Eq.
* `Version` 4-variant — Http10/Http11/Http2/Http3 + as_str +
  variant disjointness + Eq.
* `Headers` — new / with_capacity / set / get / get_all / append
  / remove / contains / len / is_empty + case-insensitive
  lookup (RFC 7230 §3.2) + set-replaces-vs-append-accumulates
  Set-Cookie semantics.
* `Request` + `Response` — `.new` factory ctors.
* `SameSite` 3-variant — Strict / Lax / None + disjointness.
* `Cookie` — `.new` factory.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` | core handler / extractor types. |
| `core.net.http2` / `http3` | request semantics shared (StatusCode + Headers + Method). |
| `core.net.websocket` | HTTP/1.1 Upgrade handshake. |
| `core.net.proxy` | CONNECT method semantics + status-code routing. |
| `core.mesh.xds` | Envoy xDS routes specify HTTP Method matching. |
| Application servers + clients | every HTTP-typed surface. |

## 2. Crate-side hardcodes

None in this module. Pure-Verum byte arithmetic; the
`@intrinsic` bridge for wire parsing is in `http_parser.vr`
(separate module).

`crates/verum_runtime/src/...` HTTP client (when implemented)
emits wire-format method names from `Method.as_str()` — drift
here would produce non-RFC-compliant HTTP. Pinned by
`test_as_str_*` tests.

## 3. Language-implementation gaps

### §3.1 `Method.as_str` / `from_str` — bare-name match arms (CLOSED via test coverage)

Pre-fix `Get / Head / Post / ...` patterns are vulnerable to
task #17/#39 first-wins collision (likely with `Request.Get`
ctor or any sibling type's matching variant). The test surface
exercises the runtime behaviour correctly through all 9
variants and the from_str case-insensitive parser; tests pass
under `--interp`. Source-side qualified-arm sweep deferred.

**Effort to qualify**: trivial (~20 min) — same find/replace
template applied to compress/archive/protobuf in prior sessions.

### §3.2 Method has no `Hash` impl

`Map<Method, EndpointHandler>` lookups need Hash. Add following
the `@derive(Eq, Hash)` pattern.

**Effort**: trivial.

### §3.3 `Method.as_str` returns owned Text (allocates)

Hot-path consumers (HTTP serializer emitting Method per request)
allocate a fresh Text each call. Switch to `&'static Text` like
`compress.Algorithm.content_encoding` does, OR cache as
constants.

**Effort**: small (~30 min).

### §3.4 No `Method.is_cacheable()` per RFC 7234 §3

GET + HEAD are cacheable by default; POST is conditionally
cacheable. Add the predicate following the is_safe pattern.

**Effort**: trivial (~10 min).

### §3.5 `Headers.get` case-insensitive via `eq_ignore_case`

Documented at `http.vr:382-389` as the ASCII-fast comparator
that runs once per header lookup per request. Tested via
`test_headers_get_case_insensitive_lower` /
`test_headers_get_case_insensitive_upper`. **No defect class
surfaced**.

### §3.6 `Headers.append` vs `Headers.set` — multi-value semantics

RFC 7230 §3.2.2 allows multi-value headers via either
comma-joined values or repeated header lines. Set-Cookie is
the canonical multi-value-via-repeat example. Tested via
`test_headers_append_does_not_replace`. **No defect class
surfaced**.

## 4. Action items landed in this branch

* `core-tests/net/http/unit_test.vr` — 44 unit tests over Method
  9-variant construction + 9 as_str canonical names + 5
  from_str parser cases + 9 is_safe + 9 is_idempotent + 8
  has_body predicates (existing, Method-focused).
* `core-tests/net/http/status_test.vr` — 65 new unit tests
  covering StatusCode (3 ctor + 8 factory + 18 categorization +
  2 pairwise-exclusivity + 2 Eq), Version (4 variant + 4
  as_str + 3 Eq/disjoint), Headers (3 ctor + 2 get + 2
  case-insensitive + 2 contains + 2 set-vs-append + 2 remove),
  Request/Response (4), SameSite (4 variant + disjoint), Cookie
  (1).
* `core-tests/net/http/audit.md` — this file (refreshed).

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Qualify Method.as_str / from_str / is_* match arms | `core/net/http.vr` | 20 min |
| Add Hash impl for Method | `core/net/http.vr` + test | 10 min |
| Switch Method.as_str to &'static Text | `core/net/http.vr` + caller migration | 30 min |
| Add Method.is_cacheable() per RFC 7234 §3 | `core/net/http.vr` + 9 tests | 30 min |
| HttpError 12-variant Eq + Display rendering coverage | this folder | 1h |
| HttpPoolConfig / ClientConfig record-field preservation | this folder | 1h |
| Cookie builder chain (with_path / with_domain / with_max_age / secure / http_only / with_same_site) | this folder | 2h |
| Headers iteration order preservation property | this folder | 1h |
| Property test: ∀req. Request.new(m, u).method == m | this folder | 30 min |
| HttpClient + HttpHandler protocol implementor tests | language level — vcs/specs/L2-standard/net/http/ | gated on protocol-resolver work |
