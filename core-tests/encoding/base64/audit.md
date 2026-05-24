# `encoding/base64` audit

Module: `core/encoding/base64.vr` (363 LOC) — RFC 4648 base64
encoding with standard + URL-safe alphabets.

Surface:
* `encode(&[Byte]) -> Text` — standard alphabet, with padding
* `encode_url(&[Byte]) -> Text` — URL-safe (-, _), no padding
* `encode_url_no_pad(&[Byte])` — explicit alias for encode_url
* `decode(&Text) -> Result<List<Byte>, Base64Error>`
* `decode_url(&Text)` / `decode_url_no_pad(&Text)`
* `Base64Error` (kind/position/message)
* `Base64ErrorKind` (InvalidChar / InvalidLength / InvalidPadding)

Tests: `unit_test.vr` (~22 unit tests covering both alphabets +
RFC 4648 §10 fixtures + decode error paths + ErrorKind disjointness),
`property_test.vr` (~15 properties — round-trip both alphabets,
length law, URL alphabet character constraints, RFC 4648 §10 vectors).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.security.jwt` | JWT tokens use URL-safe base64. |
| `core.security.webauthn` | challenge/response binary payloads. |
| `core.security.aead.chacha20_poly1305` | key/nonce serialisation. |
| `core.encoding.pem` | PEM-encoded keys/certs base64-encode the body. |
| Application code | data URLs, HTTP basic auth, opaque tokens. |

## 2. Crate-side hardcodes

None today — pure Verum byte arithmetic. Future SIMD intercepts
(NEON/AVX2 batch-encode) must produce bit-identical output verified
by round-trip + RFC 4648 fixture tests.

## 3. Language-implementation gaps

### §3.1 No `decode_into(&mut List<Byte>, &Text)` zero-alloc variant

Same gap as `encoding/hex` — hot-path callers want pre-allocated
buffer destination. Add zero-alloc variants for both alphabets.

**Effort:** small (~1h).

### §3.2 No streaming base64 codec

`encode_chunked(&mut Read, &mut Write)` for multi-GB inputs.
Deferred until need.

### §3.3 `Base64ErrorKind` Display matches Hex pattern — also exhibits typo class

Display arms use bare `InvalidChar` / `InvalidLength` /
`InvalidPadding`. Same first-wins hazard as
`core/configuration/error.vr`. Worth qualifying to
`Base64ErrorKind.X` for defense.

**Effort:** trivial (~10 min).

### §3.4 No MIME-style line-wrapped variant

Original PEM/MIME convention wraps base64 output at column 64 or
76 with CRLF. `core.encoding.pem` likely needs this — verify
that the PEM module owns the wrapping vs. delegating to base64.

## Action items landed in this branch

* `core-tests/encoding/base64/unit_test.vr` — 22 unit tests over
  encode/encode_url + decode happy/error paths + ErrorKind variants.
* `core-tests/encoding/base64/property_test.vr` — 15 properties
  covering round-trip both alphabets, length law (multiple-of-4 +
  ceil(N/3)*4), URL alphabet character constraints, RFC 4648 §10
  canonical vectors ("f"/"fo"/"foo"/"foob"/"fooba"/"foobar").
* `core-tests/encoding/base64/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `decode_into(dst, input)` for both alphabets | `core/encoding/base64.vr` + tests | 1h |
| Add streaming codec | `core/encoding/base64.vr` | 3h |
| Qualify Base64ErrorKind match arms | `core/encoding/base64.vr` | 10 min |
| Verify MIME line-wrap ownership (pem vs base64) | both modules + audit cross-ref | 30 min |
| Sister tests for `core.encoding.{base32,base58,pem,der,jcs}` | sister folders | 1 day each |
