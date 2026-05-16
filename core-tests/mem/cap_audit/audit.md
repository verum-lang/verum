# `core.mem.cap_audit` — audit findings

> Module under test: `core/mem/cap_audit.vr` (299 LOC; 1 sum type
> `CapEventKind` with 6 variants + Display/Debug/Eq impls, 1 record
> `CapEvent` with 8 fields + Display/Debug/Eq + `bumped_generation`
> predicate, no free functions).
>
> Test surfaces (this branch):
> `unit_test.vr` (~205 LOC), `property_test.vr` (~115 LOC),
> `integration_test.vr` (~110 LOC), `regression_test.vr` (~100 LOC).
>
> Tests pin the value-level shape — no ring-buffer interaction.
> Ring-buffer tests live in `core-tests/mem/cap_audit_ring/`.

## 1. Cross-stdlib usage

CapEvent is the format-stable boundary between CBGR writer entry
points (in `header.vr` and `cap_audit_ring.vr`) and observers
(panic post-mortem dumps, runtime monitoring, future debugger
integrations).

| Consumer | Use |
|---|---|
| `core/mem/header.vr` | Writer entry points `try_revoke`/`attenuate_capabilities`/`increment_ref_count`/`decrement_ref_count` build CapEvent records and call into `cap_audit_ring::commit`. |
| `core/mem/cap_audit_ring.vr` | Producer-side ring buffer; consumes `CapEvent` from the writers and stores them for observers. |
| Future panic handler | Reads recent CapEvents via `recent(n)` to construct a use-after-free post-mortem trace. |

## 2. Crate-side hardcodes

Drift surfaces:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `CapEventKind` variant order: Revoke(0) / Attenuate(1) / RefIncr(2) / RefDecr(3) / GenBump(4) / EpochAdvance(5) | Tag-based discriminator for `cap_event_kind_tag` private helper used by Eq | A reorder would silently shift `tag` values for every variant, breaking serialised event logs from older `verum` binaries. |
| CapEvent 8-field record layout | 48-byte stack-allocated record for memcpy into ring slot | Adding/removing a field changes the bit-layout the ring buffer's seqlock-protected reader assumes. Pinned by `regression_test §D`. |

## 3. Language-implementation gaps

### 3.1 No reflection escape hatch by design

CapEvent fields are scalar values (UInt32/UInt16/UInt64). No raw
pointer storage, no type-erased payloads — this is by design (event
records must be stable bytes for cross-process / cross-version
forensic reconstruction). Tests pin this invariant by ONLY using
scalar field reads.

### 3.2 `seq` assigned by ring buffer, not constructor

`CapEvent.new` returns seq=0; the ring buffer's `commit(event)`
assigns a fresh seq before storing. Tests must not assume seq != 0
for an uncommitted event. Pinned by
`regression_test.regression_cap_event_new_seq_zero`.

### 3.3 `bumped_generation` is true ONLY for Revoke and GenBump

The predicate's truth table is fixed by design:
- Revoke → true (generation incremented as part of revoke)
- GenBump → true (explicit increment)
- All other variants → false (generation unchanged)

Pre-fix some early drafts returned true for Attenuate as well, but
Attenuate narrows caps WITHOUT bumping generation. Pinned by
`property_test §C` and `regression_test §B`.

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/cap_audit.vr` | `core-tests/mem/cap_audit/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~530 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/cap_audit/` | This file. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Cross-tier divergence sweep on `--aot` + `--interp`. | 1 hour wall-clock | open |
| §B | Pin the `cap_event_kind_tag` private mapping with a `verum_common::well_known_types` constant so the variant order cannot drift silently. | ~30 min | open |
| §C | Test CapEvent serialisation/deserialisation round-trip when (future) wire-format support lands. | Blocked on wire format | open |
| §D | Test the Display + Debug impl output strings — pre-fix some drafts emitted lowercase "revoke" via Display but "Revoke" via Debug; pin both outputs. | ~20 min | open |
