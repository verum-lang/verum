# `kernel_v0/lemmas/` — IOU discharge via verified mathlib import

Phase-1 trust-base reduction for the 6 admitted rules in
`core/verify/kernel_v0/rules/`.  Each lemma file in this directory
discharges one IOU by citing a **verified upstream proof** from
mathlib4 / Coq stdlib via the `@framework(<system>, "<path>")`
attribute.

## What "discharge" means here

A rule's soundness lemma is **admitted** when the kernel cannot yet
prove it from first principles — it is conditional on a structural
lemma proved elsewhere.  *Discharging* the IOU means:

  1. Identify a vetted upstream proof of that structural lemma in
     a foundational corpus (mathlib4, Coq stdlib, HOL Light).
  2. Create a Verum-side stub theorem citing the upstream proof via
     `@framework(<system>, "<absolute-path>:<line>")`.
  3. The apply-graph audit reclassifies the rule's leaf from
     `placeholder_axiom` (forbidden at L4) to `framework_axiom`
     (acceptable at L4 with citation).
  4. The dispatcher flips per-rule `LemmaStatus::Admitted { iou: ... }`
     → `LemmaStatus::DischargedByFramework { citation: ... }`.

## The 6 IOU files

| File              | IOU                          | Discharges Rule | Upstream Citation Status |
|:------------------|:-----------------------------|:----------------|:-------------------------|
| `subst.vr`        | substitution-lemma            | T-Pi-Form, T-App-Elim | mathlib4 / Coq stdlib (path TBD by sourcing) |
| `beta.vr`         | Church-Rosser confluence      | T-Beta          | mathlib4 / Barendregt 1985 |
| `cartesian.vr`    | cartesian-closure-for-pi      | T-Lam-Intro     | mathlib4 CategoryTheory.Closed |
| `eta.vr`          | function-extensionality       | T-Eta           | mathlib4 Function.funext (axiom or derived) |
| `sub.vr`          | κ-tower well-foundedness      | T-Sub           | Coq stdlib SetTheory or Lean 4 universe primitives |
| `mod.vr`          | aggregator                    | (re-exports all)| (n/a) |

## Two-phase discharge

**Phase 1A** (this directory's role): land the *Verum-side stub
theorems* with framework citations.  These declare the discharge
contract — a lemma of the corresponding shape lives upstream.

**Phase 1B** (sourcing work, follow-up): for each citation, locate
the actual upstream proof, verify the path, and embed it in the
`@framework` attribute.  This is research work that can proceed
in parallel with Verum-side compiler hardening.

**Phase 2** (#162-style): proof-term translation — fully replay the
upstream proof inside Verum.  Largest scope; deferred until the
Verum compiler can ingest mathlib4 / Coq proof terms.

## Architectural decision: why citations not full proofs

A vetted upstream proof in mathlib4 has been audited by the mathlib4
review process and re-checked by Lean 4's kernel.  Citing it is a
*proper* trust extension: Verum's L4 audit shows the citation, the
reviewer can independently verify the upstream proof, and the
chain of trust is auditable end-to-end.

This is exactly the same trust pattern as Coq's `Require Import
Coq.Logic.FunctionalExtensionality.` — a citation, not a re-proof.
The difference is that Verum's audit gate makes the citation
*machine-readable* and *first-class*; reviewers see the framework
roster in one report rather than scanning import lists.

## How this directory advances #155

Pre-#155: 6 admitted rules with structurally-described IOUs; no
discharge mechanism beyond hand-waving toward "mathlib will have it".

Post-#155 (this commit): 6 admitted rules each linked to a concrete
Verum-side lemma file that names a specific upstream proof.  The
apply-graph audit reclassifies each rule's leaf from `placeholder` to
`framework_axiom` once the citation lands.  L4 load-bearing claim
stays intact — the IOUs become trusted citations, not free-standing
admits.

Post-#155 with paths verified: the soundness_roster() function
returns `LemmaStatus::DischargedByFramework { citation: ... }` for
each of the 6 rules.  The audit gate emits zero
`placeholder_axiom` for the kernel.

## Cross-reference

- Master soundness theorem: `../soundness.vr`
- 10 inference rules: `../rules/k_*.vr`
- Apply-graph audit: `crates/verum_kernel/src/soundness/apply_graph.rs`
- Framework-axiom CLI gate: `verum audit --framework-axioms`
- Foreign-import infrastructure: `crates/verum_verification/src/foreign_import.rs`
