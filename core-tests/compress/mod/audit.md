# `compress/mod` audit

Module: `core/compress/mod.vr` (297 LOC) — abstract compression
codec dispatch. Defines Algorithm 7-variant + CompressError 7-variant
+ Codec protocol + top-level encode/decode dispatch fns.

Tests cover: Algorithm 7-variant + content_encoding HTTP tokens +
from_content_encoding case-insensitive parsing + Eq + Display +
CompressError 7-variant construction.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2` | HTTP response Content-Encoding negotiation. |
| `core.net.http3.qpack` | header-table compression for QUIC. |
| `core.storage.s3` | server-side compression for uploads. |
| `core.compress.{brotli,gzip,lz4,zstd}` | concrete adapters dispatch through Algorithm. |

## 2. Crate-side hardcodes

None today — pure Verum dispatch. Future Rust-side intercepts for
zero-copy zstd hot-path (when implemented) must preserve Algorithm
variant tags.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified match arms

Pre-fix `content_encoding`, `algorithm_name`, `algorithm_tag` used
bare `Gzip` / `Deflate` etc. Task #17/#39 collision hazard with
`HttpStatus.Gzip` (if any), or any sibling module's Gzip variant.
Qualified to `Algorithm.<Variant>`. Display + Debug + Eq match arms
in CompressError remain bare — separate qualification fix needed.

### §3.2 Display arms for CompressError still use bare-name patterns

`UnsupportedAlgorithm(a)`, `CorruptInput(reason)`, etc. in
CompressError Display/Debug match arms still bare. Same fix
template as configuration/error / context/error — apply once.

**Effort:** trivial (~10 min, find/replace).

### §3.3 `from_content_encoding` could return Result for typed error

Today returns Maybe<Algorithm>. Returning `Result<Algorithm,
ParseError>` with reason ("unknown token", "deprecated alias",
etc.) would aid HTTP error reporting.

### §3.4 No backwards-compat aliases for HTTP tokens

Some clients send `x-gzip` (legacy) instead of `gzip`. `from_content_encoding`
silently returns None. Document or add the alias table.

## Action items landed in this branch

* `core/compress/mod.vr` — qualified Algorithm variants in
  content_encoding + algorithm_name + algorithm_tag.
* `core-tests/compress/mod/unit_test.vr` — 28 unit tests.
* `core-tests/compress/mod/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Qualify CompressError Display/Debug/Eq match arms | `core/compress/mod.vr` | 15 min |
| Switch `from_content_encoding` to Result-returning variant | `core/compress/mod.vr` + caller migration | 1h |
| Add `x-gzip` etc. backwards-compat alias table | `core/compress/mod.vr` + tests | 30 min |
| Sister tests for `core.compress.{brotli,gzip,lz4,zstd}` adapters | sister folders | 1 day each |
EOF
