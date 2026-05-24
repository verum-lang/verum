# `encoding/hex` audit

Module: `core/encoding/hex.vr` (165 LOC) — hex (base16) encoding
matching git/TLS-cert-fingerprint/hash-output conventions.

Surface:
* `encode(&[Byte]) -> Text` — lowercase
* `encode_upper(&[Byte]) -> Text` — uppercase
* `decode(&Text) -> Result<List<Byte>, HexError>` — case-insensitive
* `HexError` (kind/position/message) + `HexErrorKind` (InvalidChar / InvalidLength)

Tests: `unit_test.vr` (~24 unit tests covering both encode variants
+ decode happy path + 5 decode error paths + HexErrorKind variants),
`property_test.vr` (~13 properties — round-trip, output length law,
lowercase/uppercase canonical-form, case-insensitive decode,
odd-length rejection at 1/3/7 chars).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.tracing.id` | TraceId/SpanId encode bytes → hex via `to_hex()` (mirrors this module's contract). |
| `core.security.*` | TLS cert fingerprints, hash digests, signature output. |
| `core.encoding.{base32,base58,base64,pem,der}` | sister encoding modules. |
| Application code | git-style hex display, log-line hash output. |

## 2. Crate-side hardcodes

None today — pure Verum byte-level arithmetic. Rust-side
performance intercepts (SIMD nibble-pack, AVX2 batch decode)
when implemented must produce bit-identical output verified by
the round-trip property tests above.

## 3. Language-implementation gaps

### §3.1 No `decode_into(&mut List<Byte>, &Text)` zero-alloc variant

`decode` allocates a fresh List<Byte> per call. For hot-path
consumers (TLS cert verification, hash chain validation) a
`decode_into(dst: &mut List<Byte>, input: &Text) -> Result<Int, HexError>`
that writes into a pre-allocated buffer would skip the allocation
+ enable byte-counting.

**Effort:** small (~30 min) + 3 tests.

### §3.2 No streaming encode/decode for very large inputs

Single-buffer encode/decode assume the input fits in memory.
Streaming (`encode_chunked(&mut Read, &mut Write)`) would handle
multi-GB inputs without OOM. Add once need arises.

### §3.3 `HexError` Eq impl includes `message` field

The Eq impl at `hex.vr:85-89` compares all 3 fields including the
message string. This means two HexErrors with the same kind +
position but different messages compare unequal. For most callers
the (kind, position) tuple is the discriminator; message is
diagnostic. Document this OR change to (kind, position)-only Eq.

**Effort:** decide first; ~10 min once decided.

### §3.4 `encode_with` Bool parameter could be enum

The internal `encode_with(input, upper: Bool)` helper uses a Bool
flag for case selection. An enum `HexCase { Lower, Upper }` would
be more self-documenting at call sites. Not exposed in public API
so trivial refactor.

**Effort:** trivial (~10 min).

## Action items landed in this branch

* `core-tests/encoding/hex/unit_test.vr` — 24 unit tests over
  encode/encode_upper/decode + error paths.
* `core-tests/encoding/hex/property_test.vr` — 13 properties
  (round-trip, output length, canonical case, case-insensitivity,
  odd-length rejection).
* `core-tests/encoding/hex/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `decode_into(dst, input)` zero-alloc variant | `core/encoding/hex.vr` + tests | 30 min |
| Add streaming encode/decode for multi-GB inputs | `core/encoding/hex.vr` | 2-3h |
| Decide HexError.Eq semantics (3-field vs (kind,position)) + document | `core/encoding/hex.vr` + 2 tests | 30 min |
| Refactor `encode_with` Bool → HexCase enum | `core/encoding/hex.vr` | 10 min |
| Sister tests for `core.encoding.{base32,base58,base64,pem,der,jcs,json_pointer,json_patch,msgpack,varint,cbor,bson}` | sister folders | 1 day each |
