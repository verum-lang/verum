# External deterministic backends over the typed-IR seam (design answer)

**Status:** design answer accepted 2026-08-05 (T0675 campaign; capability
question folded from T0676). The general seam itself is specified in
`deterministic-profile-and-typed-export.md` §4; this note records the ONE
design decision that document left open: *how capability-scoped effects are
expressed by authors and recovered by an external backend.*

## Decision: Option C — capability keys as effect-fn signatures

Of the routes considered —

* **A. Parameterised capability types** (`Cap<Key>` woven through the type
  system): the deep-language route. Maximal static precision, but requires
  new type-level machinery (per-key generics, capability polymorphism)
  whose weight is not justified by today's consumers. Kept as the
  long-range expressiveness item.
* **B. Declared footprints** (authors annotate every fn with the keys it
  touches): redundant with what the compiler already knows from the call
  graph, and declared-but-wrong footprints are worse than none.
* **C. Effect-module signatures** (the backend supplies a module whose
  public fns name the storage keys; author code calls them naturally —
  `ledger.transfer(to, amount)`; the exporter/lowering READS the call
  graph): fits how authors already write Verum, and requires nothing
  beyond what the language guarantees today.

**Option C is the answer.** The footprint of an item is *derived* from the
call graph rooted at its body — never declared. Any future need for
key-generic authoring migrates to Option A without breaking the seam,
because the artefact records calls, not footprints.

## Language guarantees Option C rests on (each independently verified)

1. **Explicit dependencies.** Effectful entry points must be reachable
   only through `using [...]` contexts or through the effect module's
   public fns — Verum has no ambient global state to smuggle a key
   through. (Context system; `contexts` field of the export.)
2. **Computational-property inference.** `Pure` vs `IO`/`Mutates` is
   inferred and exported per function (`properties` field), so a backend
   can reject an effect call reached from a supposedly pure region.
3. **Capability-typed signatures survive the archive round-trip.** The
   typed-IR artefact carries capability sets, declared contexts,
   refinements and attributes verbatim (`typed_export_contract.rs`
   pins contexts / refinements / loop bounds; the capability-restricted
   type form travels as `TypeIr::CapabilityRestricted`).
4. **Canonical bytes.** Two exports of the same source are byte-identical
   (double-run test in `typed_export_contract.rs`), so a deployed
   artefact is identified by its hash — reproducibility is the audit
   chain.
5. **Termination metadata.** Every loop carries its `decreases` measure
   and the §3 bound classification (`ConstantBound(k)` /
   `FiniteNotConstant` / `Unproven`), letting bounded-total consumers
   reject unbounded constructs with the author-actionable message.

## What external backends do

An external code generator consumes the `.vtir` artefact and lowers it to
its own term algebra out of tree. Verum's contract is exactly: schema
stability (`verum-typed-ir`, independent semver), canonical bytes, and the
guarantees above recorded in the artefact. Verum knows nothing about any
particular downstream IR; no backend-specific code lives in this
repository.
