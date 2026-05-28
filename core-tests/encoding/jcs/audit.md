# `core/encoding/jcs` — conformance audit

| | |
|---|---|
| Module | `core.encoding.jcs` |
| Spec | RFC 8785 (JSON Canonicalization Scheme) |
| Tier | regression-only (pure-data surface) |
| Status | **partial** — pure-data ADT surface covered (JcsError); canonicalize round-trip gated on stdlib refresh |

## What's covered (`unit_test.vr` — 7 @test GREEN + 1 @ignored under `--interp`)

### §1 — JcsError 2-variant construction (2)
- JsonParseFailed(JsonError) wraps a nested JsonError record
- UnsupportedValue(Text) for NaN/Inf/custom values rejected by JSON

### §2 — Variant pairwise disjointness (2)
JsonParseFailed and UnsupportedValue mutually exclude.

### §3 — Match exhaustiveness (2)
Static dispatch via `describe(e) -> Text` covers both arms.

### §4 — Payload preservation (1 GREEN + 1 @ignored)
- UnsupportedValue(Text) round-trips its payload — GREEN
- JsonParseFailed inner-record kind extraction — @ignored (nested
  field access through match-binding trips field-index resolver
  defect, sister of [[btree_pattern_match_ref_generic_class]])

## Deferred

### §A — canonicalize_value / canonicalize_str round-trip
Both entry points produce Text via the json.vr stringification path
which exercises Text builders gated on the lenient-stub cascade. Same
defect family as semver `format()` and pem `encode_block()`. Multi-
day VBC codegen fix.

### §B — RFC 8785 canonical vectors
Pinned vector tests deferred until §A unblocks:
- numeric canonicalisation (integer / Float / NaN-reject)
- string escaping (RFC 8785 §3.2.2.2)
- object key sorting (UTF-16 code-unit lexicographic)
- nested object / array round-trip

### §C — Display / Debug / Eq impls
Implementations exist; gated on same stub-cascade as §A.

## Tier-1 (AOT) gate

Same as PEM — pre-existing AOT stdlib build blocker (task #7).
