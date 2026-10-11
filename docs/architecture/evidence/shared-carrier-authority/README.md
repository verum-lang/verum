# Shared carrier authority

The whole-value Shared accessor now requires an exact live interpreter heap-object base, a valid Shared header and at least one complete pointer field before following the inner extent. The existing inner-extent bound still requires all three SharedInner slots. It does not require unused carrier generation/epoch fields.

The initial seven-control baseline passed four positives and failed three interior-pointer controls. A padded, tracked Value containing Int(520) was accepted as a Shared header; its write left the intended value unchanged. The padding and a live unrelated inner extent make the oracle deterministic without an invalid read. The last-slot control first checks this safe case, so the failing baseline never probes beyond an actual last slot.

The ownership-only repair passed all seven controls. Review then exposed two more failures: a header advertising no pointer payload and an invalid header still authorized the inner extent. Those controls use padded allocations with restored header geometry before any assertion or destruction. The final repair passes ten controls, including genuinely short objects, last-slot writes, ordinary slots, genuine Shared scalar and whole-pointer writes, and one-pointer-slot compatibility.

The final thirteen adjacent controls comprise six Shared receiver codegen checks and seven parsed reference-receiver interpreter checks. The Shared codegen fixture declares a local wrapper; it is not execution of the compiled standard-library Shared implementation.

[The manifest](manifest.json) binds every compressed receipt/log to its original bytes and each historical source input to Git. The receipts retain commands, bounds, executable hashes and paths. No automatic precompilation ran; the unchanged Stage23 archive identities are provenance checks, and these controls do not consume those archives. This packet does not establish ordinary CLI, fresh-bake, full-library, AOT, source borrow-checking or stale-reference acceptance.

The existing DropRef direct call to the lower-level extent accessor and the TraceDeref alignment-only diagnostic header probes remain under T0379. General Map representation and invalidation behavior remains outside this whole-value helper repair.
