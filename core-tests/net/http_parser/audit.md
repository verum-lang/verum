# `net/http_parser` audit

Module: `core/net/http_parser.vr` (~931 LOC) — zero-copy,
resumable HTTP/1.1 wire-parser. The hot-path of HTTP servers and
clients. Target latency budget < 150 ns per request on modern
x86_64.

NOT SIMD-accelerated, despite the byte scans living in
`core/simd/bytes.vr`: `find_byte` is a scalar loop (T0184, §3.1) and
no primitive in that file vectorises today.

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
`HttpParser` and `ChunkedDecoder` is LIVE and covered by the 7
property laws — the HTTPPARSE-1 gate and the SIMD-dispatch gate that
succeeded it are both closed. See §3.1.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` | per-request HTTP parsing on accept-loop. |
| `core.net.http` clients | response parsing. |
| `core.net.websocket` handshake | HTTP/1.1 Upgrade-request parse. |
| `core.net.proxy` CONNECT | HTTP/1.1 method-only proxy parsing. |

## 2. Crate-side hardcodes

`core.simd.bytes.find_byte` (CRLF / colon scans) carries a
`@multiversion` annotation, but NOTHING DISPATCHES ON IT: there is no
SSE2 / AVX2 / NEON selection and no vector path to select between. The
bare no-argument `@multiversion` form is itself irregular —
`verum_ast`'s attribute conversion says it "requires at least one
variant" — and the function is a plain scalar loop (T0184, §3.1).
Pinned externally in `core.simd.bytes` tests.

## 3. Language-implementation gaps

### §3.1 HTTPPARSE-1 — CLOSED (both gates)

**Status 2026-07-27: no gate. `feed` is live and tested** — the module
runs 43 passed / 0 failed / 0 ignored under `--interp`, including all
7 property laws over the wire-parsing path.

Two DIFFERENT blockers held this surface in sequence, and neither is
the one this section used to name. Recorded so nobody re-opens a
closed bug:

1. The original **precompile-cascade SIGSEGV** (the CIDR-1 / URL-1 /
   URITPL-1 / HTTPRNG-1 / CONNEG-1 / LINKHDR-1 / HTTPCACHE-1 class)
   closed under T0149. `feed()` compiled and ran after it.
2. The laws then stayed `@ignore`'d on a narrower, unrelated blocker:
   `feed()` calls `core.simd.bytes.find_byte` /
   `find_header_terminator`, whose SIMD block-scan dispatched
   `Mask.any` to a method not found at runtime. Measured with a needle
   at index 3: `Some(3)` for an 8-byte haystack (below the 16-byte
   lane, so only the scalar tail ran) and an **abort** at 16 and 32
   bytes — i.e. every real header line.

Blocker 2 closed under **T0184**, and not by making SIMD dispatch
work. `find_byte`'s SIMD loop was REMOVED, because it could never have
worked: `Vec16b.load_unaligned` answers `dst = ptr` (the address as
data, never a dereference) on both tiers, and `first_set_lane` — which
the loop called to turn a match into an index — was declared nowhere
in `core/`. The function now runs, for its whole length, the scalar
scan it previously used only for the trailing `n % 16` bytes.

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

## 4. Action items landed — net-conformance-20260705

* `property_test.vr` (+7 laws) — the resumable-`feed` functional suite:
  one-shot completion + body framing, prefix safety (every strict prefix
  → NeedMore), split invariance (resume ≡ one-shot), Done idempotence,
  zero-copy view decoding, response-mode status line, garbage→Error.
  Authored `@ignore`'d on HTTPPARSE-1 (then a compile-time VBC codegen
  crash), and **live since 2026-07-27** — all 7 pass. They were dark for
  the whole interval, latterly on the SIMD-dispatch gate rather than the
  one they were filed against; see §3.1.

## Legacy action items — original landing branch

* `core-tests/net/http_parser/unit_test.vr` — 36 unit tests
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
| ~~Close HTTPPARSE-1 (batched with CIDR-1 family)~~ | — | **DONE** — closed T0149; the SIMD gate that succeeded it closed T0184 |
| Expose MAX_* constants as `public` | stdlib | 5 min |
| Per-request HttpParser.feed() round-trip tests (GET / POST / chunked) | this folder | 4h — NO LONGER GATED, §3.1 is closed |
| Property test: ∀req. parse(serialize(req)) ≡ req on Method/Version/Path/Headers/body-framing | this folder | 1 day — NO LONGER GATED, §3.1 is closed |
| HeaderView lifecycle / aliasing tests | language level | gated on arena instrumentation |
| Trailer handling (post-chunked-end) | stdlib + tests | 4h |
