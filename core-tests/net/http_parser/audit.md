# `net/http_parser` audit

Module: `core/net/http_parser.vr` (~931 LOC) — zero-copy,
resumable, SIMD-accelerated HTTP/1.1 wire-parser. The hot-path
of HTTP servers and clients. Target latency budget < 150 ns
per request on modern x86_64.

Tests cover the algebraic data-surface:

* `HttpParseError` 15-variant disjointness + Eq matrix for
  every variant carrying payload (Text / Int / record fields).
* `ParseProgress` 3-variant: NeedMore / Done {consumed, body_len,
  body_start} / Error(HttpParseError) + variant disjointness.
* `HeaderView` 4-field record (key_start/key_len/value_start/
  value_len) + zero-field construction.
* `HttpParser` construction via `.request()` / `.response()` +
  initial-state assertions (method=None, status=None,
  content_length=None, chunked=false, headers=[]).
* `ChunkedDecoder.new()` factory + initial-state.
* `ChunkProgress` 4-variant: ChunkNeedMore / ChunkOutput /
  ChunkEnd / ChunkErr(HttpParseError).

The `feed(&mut self, buf: &[Byte])` runtime path on both
`HttpParser` and `ChunkedDecoder` is locked-in behind
HTTPPARSE-1 (precompile-cascade SIGSEGV class shared with
CIDR-1 family) — see §3.1 below.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` | per-request HTTP parsing on accept-loop. |
| `core.net.http` clients | response parsing. |
| `core.net.websocket` handshake | HTTP/1.1 Upgrade-request parse. |
| `core.net.proxy` CONNECT | HTTP/1.1 method-only proxy parsing. |

## 2. Crate-side hardcodes

The `core.simd.bytes.find_byte` intrinsic (CRLF / colon scans)
is dispatched here through `@multiversion` annotation —
SSE2 / AVX2 / NEON / scalar fallback. Pinned externally in
`core.simd.bytes` tests.

## 3. Language-implementation gaps

### §3.1 HTTPPARSE-1 — `HttpParser.feed` / `ChunkedDecoder.feed` SIGSEGV

**Stable trigger**: invoking `parser.feed(buf.as_slice())` from a
USER test module triggers the precompile-cascade SIGSEGV class
shared with CIDR-1 / URL-1 / URITPL-1 / HTTPRNG-1 / CONNEG-1 /
LINKHDR-1 / HTTPCACHE-1.

The construction surface (HttpParser.request/.response,
ChunkedDecoder.new, accessor methods method/status/version/
content_length/is_chunked/headers/trailers) compiles and tests
pass. The wire-parsing functional surface (feed) is gated.

Same likely root cause as CIDR-1 family.

### §3.2 Private DoS-guard constants

`MAX_REQUEST_LINE`, `MAX_HEADER_LINE`, `MAX_HEADERS_TOTAL`,
`MAX_HEADER_COUNT`, `MAX_CONTENT_LENGTH` are private `const`
(no `public` keyword) and not accessible to tests. The values
are referenced in the corresponding `HttpParseError` variant
payloads (`HeaderTooLong { limit }`, `TooManyHeaders { limit }`,
`RequestLineTooLong { limit }`, `ContentLengthTooLarge { limit
}`) but cannot be cross-validated.

**Effort to expose**: 5 min (`pub const`). Recommend exposing as
public for caller-side instrumentation.

### §3.3 Zero-copy `HeaderView` lifecycle

`HeaderView.key(&self, buf: &'a [Byte])` returns `&'a [Byte]`
borrowed from the parser's input buffer. The buffer must
outlive the parsed Request — Weft enforces this through a
per-request arena. Tested at construction-shape only here;
end-to-end lifecycle tested at language level
(vcs/specs/L2-standard/net/http_parser/).

## 4. Action items landed in this branch

* `core-tests/net/http_parser/unit_test.vr` — 32 unit tests
  covering HttpParseError 15-variant construction + Eq + 3
  pairwise-disjointness checks, ParseProgress 3-variant
  construction + variant disjointness, HeaderView 4-field
  construction + zero-fields, HttpParser request/response
  factory + 4 initial-state assertions, ChunkProgress
  4-variant construction, ChunkedDecoder.new factory.
* `core-tests/net/http_parser/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close HTTPPARSE-1 (batched with CIDR-1 family) | VBC codegen | 3-5 days |
| Expose MAX_* constants as `public` | stdlib | 5 min |
| Per-request HttpParser.feed() round-trip tests (GET / POST / chunked) | this folder | 4h, gated on §3.1 |
| Property test: ∀req. parse(serialize(req)) ≡ req on Method/Version/Path/Headers/body-framing | this folder | 1 day, gated on §3.1 |
| HeaderView lifecycle / aliasing tests | language level | gated on arena instrumentation |
| Trailer handling (post-chunked-end) | stdlib + tests | 4h |
