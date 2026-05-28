# `core/encoding/der` — conformance audit

| | |
|---|---|
| Module | `core.encoding.der` |
| Spec | X.690 ASN.1 BER/DER (Distinguished Encoding Rules) |
| Tier | regression-only (pure-data ADT + constant surface) |
| Status | **partial** — TagClass + Tag + DerErrorKind + 9 canonical universal tag constants covered; read/write entry points deferred |

## What's covered (`unit_test.vr` — 26 @test GREEN under `--interp`)

### §1 — TagClass 4-variant construction (4)
- Universal / Application / ContextSpecific / Private

### §2 — TagClass pairwise disjointness (4)
Each class tested against its 3 siblings.

### §3 — Tag 3-field record construction (3)
- Universal SEQUENCE (constructed=true)
- Universal INTEGER (constructed=false)
- ContextSpecific[0] (explicit tag)

### §4 — Canonical UNIVERSAL-class tag numbers (9)
ASN.1 universal tag-number table pinned for X.509 / PKIX:
| Constant | Value | Use |
|---|---:|---|
| TAG_BOOLEAN | 0x01 | BOOLEAN |
| TAG_INTEGER | 0x02 | INTEGER |
| TAG_BIT_STRING | 0x03 | BIT STRING |
| TAG_OCTET_STRING | 0x04 | OCTET STRING |
| TAG_NULL | 0x05 | NULL |
| TAG_OID | 0x06 | OBJECT IDENTIFIER |
| TAG_UTF8_STRING | 0x0C | UTF8String |
| TAG_SEQUENCE | 0x10 | SEQUENCE (always constructed) |
| TAG_SET | 0x11 | SET (always constructed) |

### §5 — DerErrorKind 10-variant construction (10)
- UnexpectedEndOfInput / MalformedDer / UnexpectedTag
- LengthOverflow / TrailingBytes / InvalidOid
- InvalidBoolean / InvalidInteger / InvalidBitString
- InvalidTime

## Deferred

### §A — DerError 3-field record
`DerError { kind, position, message }` record-payload tests deferred
behind the same field-destructure defect class as
[[btree_pattern_match_ref_generic_class]].

### §B — Tag.encode() / Tag.decode() round-trip
Tag encoder/decoder enters the byte-array compile-time class —
gated.

### §C — Read entry points
`read_boolean`, `read_integer`, `read_bit_string`, `read_octet_string`,
`read_oid`, `read_sequence`, `read_set`, `read_utc_time`,
`read_generalized_time` — all gated on byte-array codegen + the
Text-builder lenient-stub cascade (for the error-message path).

### §D — Write entry points
`write_boolean`, `write_integer`, `write_bit_string`,
`write_octet_string`, `write_oid`, `write_sequence`, `write_set`,
`write_utc_time` — same byte-array gating.

### §E — Real X.509 / PKIX canonical vectors
- RFC 5280 §4.1 Certificate SEQUENCE round-trip
- Common Name (CN) PrintableString
- ECDSA signature OID (1.2.840.10045.4.3.2)
- RSA encryption OID (1.2.840.113549.1.1.1)

All deferred until §B/§C/§D unblock.

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
