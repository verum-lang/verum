# DropRef object authority and register cleanup

This packet records a bounded T0379 repair linked to T1196 and T1752.
DropRef now checks exact interpreter-heap ownership before reading an object
header or enumerating tuple fields. The common Shared extent helper checks
that ownership, header validity and one complete pointer field before following
the separately bounded live inner allocation. A genuine Shared drop decrements
its strong count once and then clears the operand register and advances its
generation. Interior references retain their pointee and use register cleanup.

The implementation was tested at `f23a8db072e3fac8779917e2101779e2fff8144f`.
Its production file is unchanged from `0e074994506ad1d7f96310c49c4d48eafb42fa76`.
The subsequent `1ae1e7e337a154d90577c7af3d642b614400abde` changes only a SAFETY
comment from “the caller” to “the checks above”; the release receipt verifies
that exact substitution. The checker target was explicitly released afterward.

| Run | Result | Interpretation |
| --- | --- | --- |
| `baseline` | 3 pass, 6 fail | Five runtime authority/cleanup failures and one incorrect source-lowering assumption |
| `fixed-before-oracle-correction` | 8 pass, 1 fail | All runtime controls pass; source test incorrectly combines registers from mutually exclusive match arms |
| `final` | 9 pass | Seven runtime controls and two source/wire instruction inspections |
| `retained-adjacent` | 12 pass | Ten T1752 controls and two existing descriptor-layout guards, using the final retained library executable |
| `receiver-adjacent` | 13 pass | Six Shared receiver codegen controls and seven reference-receiver interpreter controls |
| `owned-drop-adjacent` | 1 pass | Existing source/wire control preserves declared Drop and defer order; 15 other tests filtered |

The final 35 selected controls have no failures or ignores. The focused library
gate leaves 2,101 other library tests filtered. These are bounded checks, not a
full VBC run. All gate source, executable and artifact identities are retained.
The packet verifies 31,938 historical source bindings against 5,326 unique Git
blobs. `manifest.json` gives both original and compressed SHA-256 hashes;
`.gz` files preserve the original receipt/log bytes exactly. Test executables
remain at the paths recorded in the receipts outside the Cargo target.

## Causal controls

The integer-520 decoy uses a fully initialized padded allocation: the old
handler mistakes an interior Value for a Shared header and decrements an
unrelated live strong count. The true last-slot control runs only after that
safe prerequisite passes. Short and FREED carrier controls likewise start from
padded owned allocations, restore the temporary header changes, and fail on
wrong state changes before any genuinely short-object case runs.

Positive controls retain genuine one-slot and three-slot Shared carriers,
ordinary interior values, and register references. Repeating DropRef after a
genuine Shared register was cleared must not decrement its count again.

The parsed explicit returned-reference local emits DropRef. The parsed
Maybe-pattern payload instead emits `GetVariantData`, `ChkRef`, `Deref` and a
scalar return without a payload DropRef. Its final test inspects the checked
payload read; it does not claim control-flow execution or a cleanup instruction
that is absent. Both earlier incorrect test-oracle failures remain in this
packet; no lowering was changed to satisfy them.

## Boundaries

No ordinary CLI, automatic stdlib bake, AOT, source borrow-checking, or general
stale-reference lifetime claim follows from these gates. The five immutable
Stage23 artifacts are pinned and unchanged but are not consumed by these
focused targets. The six Shared receiver adjacency tests use a local wrapper
rather than executing the actual stdlib Shared body.

T0379 remains open for its global header-probe and folded audit requirements.
The earlier TraceDeref probes are outside this repair. Read-only review also
found that a tuple can queue multiple Shared values while a completed queued
Shared does not rewind for the remaining queue. The old early return had the
same scheduling limitation; this packet does not measure or repair that
independent path and makes no complete tuple-draining claim.
