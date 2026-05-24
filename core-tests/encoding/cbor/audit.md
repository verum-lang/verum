# `encoding/cbor` audit

Module: `core/encoding/cbor.vr` (~600 LOC) — RFC 8949 Concise
Binary Object Representation.

Tests: 35 unit tests covering CborValue 11-variant +
disjointness + CborError 8-variant + Eq (round-trip per-variant)
+ MAX_CBOR_NESTING / MAX_CBOR_ARRAY_ITEMS / MAX_CBOR_MAP_ENTRIES
DoS-protection caps + encode + decode round-trip on simple
values (uint/null/bool/text/array empty) + decode rejects
empty input + encode_canonical produces bytes.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.security.cose` | COSE signatures over CBOR maps. |
| `core.security.webauthn` | WebAuthn authenticator data envelopes. |
| Application telemetry | compact binary serialisation for wire formats. |

## 2. Crate-side hardcodes

`encode_canonical` MUST agree with RFC 8949 §4.2 deterministic
encoding rules (smallest length encoding, sorted map keys,
half-precision floats where exact). Tested via fixed-vector
canonical encoding comparison (gated on JSON-test-vector import
infrastructure — currently L2 specs only).

## 3. Language-implementation gaps

### §3.1 Property tests

* ∀v: simple-shaped CborValue. decode(encode(v)) == Ok(v)
* ∀v. encode(v).len() > 0 (non-empty output for any value)
* ∀v. encode_canonical(v).len() >= 1
* Indefinite-length encoding round-trip (CborArray with > 23 items)

**Effort:** ~1h.

### §3.2 Hostile-input DoS rejection tests

* MAX_CBOR_NESTING + 1 nested array → NestingTooDeep
* MAX_CBOR_ARRAY_ITEMS + 1 claimed length → ArrayTooLarge
* MAX_CBOR_MAP_ENTRIES + 1 claimed entries → MapTooLarge

These are pinned in the constants but the rejection path needs
direct decode tests on synthesised byte streams.

**Effort:** small (~45 min).

### §3.3 RFC 8949 appendix A test vector import

Standard CBOR test vectors at https://github.com/cbor/test-vectors —
import as decode-only fixtures.

## Action items landed in this branch

* `core-tests/encoding/cbor/unit_test.vr` — 35 unit tests over
  CborValue 11-variant + CborError 8-variant + Eq + capacity
  constants + encode/decode round-trip on simple values.
* `core-tests/encoding/cbor/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (round-trip + canonical) | this folder | 1h |
| Hostile-input DoS rejection tests | this folder | 45 min |
| RFC 8949 appendix A test vector import | this folder | 1h |
| Sister tests for `core.encoding.{json,toml,messagepack}` | sister folders | 1 week total |
