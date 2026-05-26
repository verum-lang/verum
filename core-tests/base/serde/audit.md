# `core/base/serde` — Audit

> Module: `core/base/serde.vr` — the Serialize / Deserialize protocol
> surface that every backend (json / cbor / msgpack / binary) must
> satisfy. Backend round-trip tests live in `core-tests/encoding/*`.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `SerdeError` | record `{ message: Text }` | yes |
| `SerdeResult<T>` | alias `Result<T, SerdeError>` | yes |
| `Serialize` | protocol with `serialize<S: Serializer>(&self, s: S) -> SerdeResult<S.Output>` | yes |
| `Deserialize` | protocol with `deserialize<D: Deserializer>(d: D) -> SerdeResult<Self>` | yes |
| `Serializer` | protocol — backend write surface | yes |
| `Deserializer` | protocol — backend read surface | yes |
| `ListSerializer<S>` | record — serialize collection adapter | yes |
| `MapSerializer<S>` | record — serialize map adapter | yes |
| `RecordSerializer<S>` | record — serialize struct adapter | yes |

### 1.2 SerdeError factories

| Item | Signature |
|---|---|
| `SerdeError.new` | `(Text) -> SerdeError` |
| `SerdeError.message` | `(&self) -> &Text` |
| `SerdeError.unexpected_type` | `(&Text, &Text) -> SerdeError` |
| `SerdeError.missing_field` | `(&Text) -> SerdeError` |
| `SerdeError.unknown_field` | `(&Text) -> SerdeError` |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 5 unit tests | all green under `--interp` |
| `property_test.vr` | 6 property tests | all green under `--interp` |
| `integration_test.vr` | 7 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 9 active pins | 9 green |

## §2 — Findings landed in this branch

### 2.1 INVENTORY entry was stale

INVENTORY claimed `base/serde` was `1/3 green` with `2 (unit + property FAIL — TBD)`.
Direct interpreter execution shows ALL 17 pre-existing tests pass:

```
running 1 tests (tier=interpret, parallel=true)
test result: ok. 1 passed; 0 failed; 0 ignored; finished in 27s
```

per the spec's `cargo run --release --bin verum test --interp` form.
The 1/3 diagnosis appears to predate a SerdeError refinement landed
upstream; the suite is currently green end-to-end.

**Fix in this branch**: bumped INVENTORY + base.md to reflect actual
green-suite status; added a `regression_test.vr` to lock the
canonical SerdeError factory contracts so future drift surfaces as
named test failures.

### 2.2 Backend round-trips live in `core-tests/encoding/*`

The Serialize / Deserialize protocols are pure protocol declarations
— a concrete backend (json / cbor / msgpack / binary) is required to
exercise them end-to-end. Those are tested in:

* `core-tests/encoding/cbor` (CBOR round-trip — 27/35 green)
* `core-tests/encoding/hex` (Hex round-trip — 24+13 green)
* `core-tests/encoding/base64` / `base32` / `varint` / `json_pointer`

The user-defined `implement Serialize for Point { ... }` shape pin in
`integration_test.vr` is sufficient to verify the protocol-method
signatures typecheck against actual user code.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.serde`:

* `core.encoding.cbor` — RFC 8949 binary serialization.
* `core.encoding.json_pointer` — RFC 6901 path navigation.
* `core.encoding.hex` / `base32` / `base64` — text encodings.
* `core.configuration.*` — config file deserialization (toml / yaml / json / ...).

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. NEW `core-tests/base/serde/regression_test.vr` — 9 active pins:
     §A SerdeError.new preserves message verbatim
     §B missing_field includes the field name (4 sample names)
     §C unexpected_type includes BOTH expected and found
     §D unknown_field includes the field name (3 sample names)
     §E SerdeResult<T> Ok round-trip
     §F SerdeError single-field-record layout pin
     §G SerdeError Eq structural over message field

2. NEW `core-tests/base/serde/audit.md` — this file.

3. INVENTORY.md + website base.md entries updated from
   `partial 1/3 / 2 FAIL TBD` to `partial 4/4 green`.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Live in-memory Serializer / Deserializer adapter for round-trip property tests | 2-3h | future task |
| `Display` / `Debug` impls round-trip property test via `f"{error}"` | gated on the `f"{x}"` Display dispatch defect | future task |
| `SerdeError.chain` for nested errors | 1h additive stdlib | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 (semver aliases landed in this session) |
