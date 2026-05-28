# `core/encoding/json_patch` — conformance audit

| | |
|---|---|
| Module | `core.encoding.json_patch` |
| Spec | RFC 6902 (JSON Patch) |
| Tier | regression-only (pure-data ADT surface) |
| Status | **partial** — JsonPatchError 6-variant ADT covered; JsonPatchOp record-payload + apply round-trip deferred |

## What's covered (`unit_test.vr` — 21 @test GREEN under `--interp`)

### §1 — JsonPatchError 6-variant ADT construction (6)
- Malformed(Text)
- BadPointer(Text)
- MissingField(Text)
- TargetNotFound(Text)
- TestFailed(Text)
- InvalidOp(Text)

### §2 — Pairwise disjointness (6)
Each variant tested against its 5 siblings via `is`-pattern.

### §3 — Match exhaustiveness (6)
`describe(e: &JsonPatchError) -> Text` covers all 6 arms.

### §4 — Payload preservation (3)
Malformed / BadPointer / InvalidOp Text payloads round-trip through
construction + match-binding.

## Deferred

### §A — JsonPatchOp 6-variant ADT
- Add     { path: JsonPointer, value: JsonValue }
- Remove  { path: JsonPointer }
- Replace { path: JsonPointer, value: JsonValue }
- Move    { from: JsonPointer, path: JsonPointer }
- Copy    { from: JsonPointer, path: JsonPointer }
- Test    { path: JsonPointer, value: JsonValue }
All record-payload variants gated on the field-destructure resolver
defect — sister of [[btree_pattern_match_ref_generic_class]].

### §B — JsonPatch wrapper record
`JsonPatch { ops: List<JsonPatchOp> }` — covered indirectly by
parse/apply tests once §A unblocks.

### §C — parse() + apply() round-trip
RFC 6902 parser/applier deferred behind json.vr's parser surface
which itself enters the Text-builder stub cascade. Multi-day VBC
codegen fix.

### §D — RFC 6902 canonical vectors
Pinned vector tests deferred until §C unblocks:
- Section 3 — Document Structure (add/remove/replace test ops)
- Section 4 — Operations (move/copy semantics)
- Section A — Test cases (atomic apply on failure)
- Patch on array vs object discriminations
- Path escaping (~0 / ~1) round-trip

### §E — Display / Debug / Eq impls
Implementations exist; gated on stub-cascade family.

## Tier-1 (AOT) gate

Same as other encoding/ modules — pre-existing AOT stdlib build
blocker (task #7).
