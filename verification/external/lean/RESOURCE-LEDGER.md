# Shared resource ownership (T2008)

`ResourceLedger.lean` proves 46 model laws; `ResourceLedgerControls.lean` has
24 positive controls and five explicitly rejected false claims. The 70 named
proofs have exact axiom guards in `ResourceLedgerAxioms.lean`: only `propext`,
`Quot.sound`, or no axioms. There is no `sorry`, custom axiom or `native_decide`.
The model uses Lean 4.35 and the standard library. This is the external theorem
layer of Verum verification, not a certificate that the whole Rust system refines it.

Allocation identities are ledger slots. Aliases change reference multiplicity,
not charged bytes; final release frees the charge. New storage must coexist with
old storage before replacement. Poison prevents new admission but does not revoke
existing alias/drop authority. Finite class partitions conserve bytes; positive
reservation count is bounded by held bytes and the explicit machine maximum.
Refusal retains the ownership/byte projection. Native diagnostic refusal counters
are outside that projection. Resumption returns the same computation identity
when the stated account/capacity premises hold; this is not unconditional liveness.

Unboxing describes the payload **after** its borrowed predicate returns. Interior
mutation, mutation of other accounts and unwinding are possible in Rust. A borrowed
predicate cannot replace the owned Box, but it is not a purity proof. Accounting
moves and slots do not prove application-payload contents, pointer identity or
native control-flow refinement. Allocator metadata, transient excess capacity,
nested allocations and total RSS require separate contracts.

## Frozen native witness

`fixtures/resource-ledger-v1.json` is the actual eight-stage MemoryAccount export
from Omega r121. Its provenance names the native source hash and successful store
stage; the overall historical run was interrupted. This fixture does not certify
newer dormant-workspace or physical-time edits. The replay checks quota, current
bytes, peak, five class charges and reservation count against composed operations.
Diagnostic refusal counts are retained and validated as data but not modeled.

Run the existing immutable replay driver (in the shared resource queue):

```sh
python3 verification/external/lean/check_resource_ledger_export.py \
  --export verification/external/lean/fixtures/resource-ledger-v1.json \
  --sha256 981e8848fd64c0b5542c66b125cd7d8000537d5069c7cf01303fecfca8b909ae \
  --provenance verification/external/lean/fixtures/resource-ledger-v1.provenance.json \
  --output /tmp/resource-ledger-replay.lean \
  --report /tmp/resource-ledger-replay.json
python3 verification/external/lean/test_resource_ledger_export.py
```

The retained result contains 16 kernel-checked claims over eight native snapshots;
all five independently changed observable fields are refused as false propositions.
The Python boundary suite has eight tests, including malformed/partial/reordered
traces, duplicate keys, wrong classes and non-natural/out-of-word values.

The exact native maximum remains `18446744073709551615`. Initial direct reduction
of inequality-containing operations exhausted the checker. The accepted proof
uses `Nat.le_of_ble_eq_true rfl` and the universal reserve laws; it keeps the same
maximum and the same operations. Checkpoint equalities are proved, not assumed.
No higher checker limit or smaller convenient machine bound repairs the theorem.

An independent fresh replay checked the original 45-law model, 24 controls,
69 axiom guards and all five standalone false claims before the final additional
quota-refusal lemma. The final 46-law/70-guard package and the native trace were
then checked separately; these scopes must not be conflated.
