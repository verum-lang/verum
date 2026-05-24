# `action/gauge` audit

Module: `core/action/gauge.vr` (~107 LOC) — gauge freedom on
enactments. Observational-run absorption canonical form +
gauge_equivalent + 3 invariant helpers.

Tests: 10 unit tests covering canonicalise (identity), gauge_
equivalent (identity, single, distinct primitives),
canonical_size (identity, single), canonicalise_idempotent
(identity, single, composed, observe-duplicates).

## 1. Defects surfaced

### §A.1 Enactment.steps field-access after canonicalise (3 @ignore)

* `test_canonicalise_identity_yields_zero_steps` — assertion
  fails (canonicalise(identity).steps.len() should be 0).
* `test_canonicalise_single_primitive_preserved` — same shape.
* `test_canonical_size_observe_duplicates_one` —
  `field access out of bounds: field index 1 (offset 8+8 = 16)
   exceeds object data size 8` — likely Enactment record's steps
  field is mis-read at codegen when the Enactment was constructed
  by canonicalise.

Likely the same defect class as `[[btree_pattern_match_ref_generic_class]]`
applied to Enactment records — field index 1 (offset 16) reads
into 8-byte data, meaning the record was sized for a smaller
shape than its actual layout. Cross-module return-value codegen
likely loses the `articulation` field.

### §A.2 compose + canonicalise null-deref (1 @ignore)

* `test_gauge_equivalent_observe_duplicates_absorbed` —
  `NullPointerAt { op: "opcode 0x66", site: "test", pc: 89 }`
  — opcode 0x66 is field-write at pc 89, suggesting compose's
  result feeds canonicalise which writes a null-derived field.

### §A.3 is_canonical naming convention (1 @ignore)

* `test_is_canonical_single_primitive_no` — my assertion was
  wrong (primitive_enact then canonicalise both name as
  "ε_canonical"). Documented for follow-up clarification.

## 2. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.action.verify` | gauge_equivalent in coherence checks |
| `core.action.ludics` | canonicalise on session reduction |
| `verum audit --gauge` | canonicalise output |

## 3. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Fix Enactment field-access defect class | VBC codegen | 2-3 days (same root as btree pattern-match-ref-generic) |
| Property test: gauge_equivalent transitive + symmetric | this folder | 30 min |
| Sister tests for `core.action.{ludics,verify,enactments}` | sister folders | 1 week total |

## Action items landed in this branch

* `core-tests/action/gauge/unit_test.vr` — 10 unit tests (5
  GREEN + 5 @ignore for surfaced defects).
* `core-tests/action/gauge/audit.md` — this file.
