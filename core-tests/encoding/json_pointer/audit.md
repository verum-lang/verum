# `encoding/json_pointer` audit

Module: `core/encoding/json_pointer.vr` (348 LOC) — RFC 6901
JSON Pointer parser, formatter, and resolver. Defines JsonPointer
record + JsonPointerError 3-variant + parse/format_json_pointer +
JsonPointer.{root, from_tokens, is_root, depth, push, parent} +
resolve against a JsonValue document.

Tests: `unit_test.vr` (~22 unit tests over RFC 6901 §3+§4+§5
fixtures + escape handling + builder API).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.encoding.json_patch` | RFC 6902 JSON Patch references targets by JSON Pointer. |
| `core.configuration.{toml,format}` | schema-driven loaders address fields by JSON Pointer. |
| `core.search.types.SearchHit.formatted` | field-level highlight uses JSON Pointer. |
| Application config schema validators | JSON Pointer for diagnostic locations. |

## 2. Crate-side hardcodes

None today — pure Verum codec.

## 3. Language-implementation gaps

### §3.1 Closed in this branch — qualified Display/Debug/Eq arms

Source-side fix in this round (qualified `JsonPointerError.<Variant>`).

### §3.2 `resolve(p, doc)` not unit-tested

`resolve(&JsonPointer, &JsonValue) -> Maybe<JsonValue>` is the
load-bearing consumer API — needs unit tests with sample JsonValue
fixtures. Deferred because JsonValue is a recursive variant ADT
requiring more elaborate test setup.

**Effort:** medium (~2h).

### §3.3 No property_test.vr round-trip law

∀p. parse(format_json_pointer(p)) == Ok(p) — round-trip
identity. Adds confidence the escape encoder/decoder are inverses
across the full Unicode token surface.

**Effort:** small (~30 min).

### §3.4 RFC 6901 §6 URI-fragment form not exposed

The spec defines an alternative URI-fragment form (#/foo) but
only the JSON-string form (/foo) is parsed today. Document the
omission OR add `parse_fragment` / `format_fragment` variants.

## Action items landed in this branch

* `core/encoding/json_pointer.vr` — qualified Display/Debug/Eq arms.
* `core-tests/encoding/json_pointer/unit_test.vr` — 22 unit tests.
* `core-tests/encoding/json_pointer/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add unit tests for `resolve(p, doc)` against JsonValue fixtures | this folder | 2h |
| Add property_test.vr round-trip law | this folder | 30 min |
| Add URI-fragment parse/format variants | `core/encoding/json_pointer.vr` + tests | 1h |
| Sister tests for `core.encoding.json_patch` (RFC 6902) | core-tests/encoding/json_patch/ | 1 day |
EOF
