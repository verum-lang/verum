# Audit — `core/base/serde.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/serde.vr` (344 lines) |
| Tests | NEW — `unit_test.vr` (~70 LOC, error construction + protocol shape) |
| Hardcodes in `crates/` | none — pure protocol definitions |

## §1  Backend-agnostic test scope

`serde.vr` defines the **protocol surface** (Serialize, Deserialize,
Serializer, Deserializer, ListSerializer, MapSerializer) but no
concrete backends. Backend implementations live in `core.encoding.json`,
`core.encoding.binary`, etc. Round-trip tests for actual serialization
belong with those backends, not here.

This folder's tests verify:
- SerdeError construction and message formatting
- Protocol signatures typecheck-clean for user-defined types
- Default-derive scaffolding (where supported)

## §2  Property tests deferred

True round-trip property tests (`deserialize ∘ serialize = id` for any
T: Serialize+Deserialize) require a concrete backend instance. They
will live in `core-tests/encoding/<backend>/property_test.vr` once
those folders are populated.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/serde/`
- [x]  `unit_test.vr` — SerdeError constructors (new, unexpected_type,
       missing_field, unknown_field), protocol-shape compile check
- [x]  This audit document

## §4  Action items deferred

1. **Concrete backend round-trip suite** — once `core-tests/encoding/`
   exists, every backend gets a `property_test.vr` verifying
   `deserialize(serialize(x)) == Ok(x)` for every primitive and a
   sample of compound types.
2. **Format-version negotiation** — if Verum's serde supports
   versioned formats, pin the upgrade-and-downgrade contract.
3. **Untrusted-input safety** — deserializing arbitrary bytes must
   not crash or produce panics on malformed input. Fuzz-tested
   coverage is the right shape but requires fuzz integration.
