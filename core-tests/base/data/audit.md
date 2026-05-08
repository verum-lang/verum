# Audit — `core/base/data.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/data.vr` (1177 lines) |
| Tests | `core-tests/base/data/` — `unit_test.vr` (1716 LOC, migrated), `property_test.vr` (NEW, ~210 LOC, variant disjointness + round-trip + Eq + DataBuilder), `integration_test.vr` (NEW, ~140 LOC, JSON parse, builder API, accessors, deep_merge) |
| Hardcodes in `crates/` | minimal — Data is structural; closely tied to JSON and serde |

## §1  Variant layout — drift surface

`Data` has 7 variants — `Null | Bool | Int | Float | Text | Array | Object` —
and `DataError` has 5 variants. Both are sum types whose tags are
hardcoded by codegen at registration time. Same drift pattern as
Maybe/Result/Ordering, but not yet pinned via canonical layout
constants.

**Action item (deferred):** add `DATA_VARIANT_LAYOUT` and
`DATA_ERROR_VARIANT_LAYOUT` constants in
`verum_common::well_known_types` once the public API has stabilised.
Pattern is identical to MAYBE_VARIANT_LAYOUT.

## §2  JSON parse-roundtrip — the central contract

`parse_json(d.to_json()) == Ok(d)` is the load-bearing law for any
serialisation roundtrip. Property tests exercise it for primitives;
nested cases pinned in `unit_test.vr`.

Edge cases to watch:
- Float NaN and ±Inf are not representable in JSON — expected to fail
  to_json or to round-trip via stringification.
- Unicode in Text values — Verum uses UTF-8 throughout; JSON encoding
  must be lossless.
- Integer overflow at JSON parse boundary (53-bit float vs 64-bit Int).

## §3  Cross-stdlib usage

- `core.encoding.json` likely consumes/produces `Data` directly
- `core.io.config` parses YAML/TOML into Data shape
- API clients in `core.net.*` deserialize responses into `Data`

## §4  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/data_test.vr` →
       `core-tests/base/data/unit_test.vr` (frontmatter stripped)
- [x]  Add `property_test.vr` covering variant predicate disjointness,
       round-trip via parse_json/to_json, Eq laws, DataError variant
       distinction, merge identity, plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering JSON parse with nested
       structures, DataBuilder fluent API, to_json_pretty round-trip,
       keys/values accessors, deep_merge
- [x]  Add this audit document

## §5  Action items deferred (not landed)

1. **DATA_VARIANT_LAYOUT canonical constant** — mirror Maybe/Result.
2. **NaN/Inf round-trip behaviour** — pin the contract for
   non-representable Float values (probably ParseError or special
   sentinel).
3. **Integer-overflow boundary** — what happens when JSON has
   `9007199254740993` (just above 2^53)? Pin behaviour.
4. **Streaming parse for large JSON** — currently `parse_json` is
   all-in-memory; no streaming API. Out of scope for this audit but
   logged.
