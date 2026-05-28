# `core/encoding/jsonb` — conformance audit

| | |
|---|---|
| Module | `core.encoding.jsonb` |
| Spec | PostgreSQL / SQLite JSONB wire format (version 1) |
| Tier | regression-only (pure-data ADT + record surface) |
| Status | **partial** — JsonbValue + JsonbError scalar variants covered; wire encode/decode + JsonError/ConvertError-wrapping variants deferred |

## What's covered (`unit_test.vr` — 14 @test GREEN under `--interp`)

### §1 — JSONB_VERSION_1 constant (1)
Pins the wire-version byte at 1 (current Postgres / SQLite default).

### §2 — JsonbValue record + accessors (6)
- `JsonbValue.new(json_text)` constructor (defaults version to 1)
- `.version()` accessor returns the stored version
- `.as_text()` accessor returns &Text view
- Direct record construction `JsonbValue { version, json_text }`
- `.equals(&other)` reflexive on same-payload pair
- `.equals(&other)` rejects different payload + different version

### §3 — JsonbError scalar variants (3)
- JbeEmpty
- JbeUnsupportedVersion(Int)
- JbeBadUtf8

### §4 — Pairwise disjointness — scalar variants (3)

### §5 — Payload preservation — JbeUnsupportedVersion (1)

## Deferred

### §A — JsonbError compound variants
- JbeBadJson(JsonError)         — wraps json.vr error
- JbeBadValue(ConvertError)     — wraps value.vr error
Variant construction works but compound-payload extraction via
nested-record-field-binding trips the same field-index resolver
defect as JCS JsonParseFailed (sister of
[[btree_pattern_match_ref_generic_class]]).

### §B — Wire encode/decode round-trip
`encode(&JsonbValue) -> List<Byte>` + `decode(&[Byte]) -> Result<JsonbValue, JsonbError>` round-trip pinned tests deferred behind:
- Byte-array compile-time class (sister of CBOR / Base58 / DER round-trip)
- Version-byte sentinel: encode prepends 0x01; decode rejects unknown
- UTF-8 validation: bytes after version byte must be valid UTF-8
- Empty payload: decode([]) → JbeEmpty
- Wrong version: decode([0x02, ...]) → JbeUnsupportedVersion(2)

### §C — `.parse()` method — JsonbValue → JsonValue
Re-parses the json_text payload through `core.encoding.json.parse`.
Gated behind json.vr's parser surface entering the Text-builder
lenient-stub cascade.

### §D — Display / Debug / Eq impls
Implementations exist; gated on stub-cascade family.

### §E — JSON-against-JsonbValue canonical round-trips
Pinned vector tests deferred until §B + §C unblock:
- null / true / false / 42 / 3.14 / "hello" scalar round-trips
- {} / [] empty-container round-trips
- nested {"a":[1,2]} round-trip
- UTF-8 multibyte payload (€ / 𝕊)

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
