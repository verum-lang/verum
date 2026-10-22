# Diagnostic evidence and mathematical proof

Intrinsic decisions, reflection samples and `apply` dependency graphs do not
carry native proof terms for the propositions whose names they mention. Their
positive results have status `structural_only`. Computation and bibliography
remain distinguishable provenance, but neither grants theorem authority.

This boundary is represented by `VerificationVerdict::StructuralOnly`:

- An intrinsic records the observed flag, reason and citation without discharging
  an annotated theorem. Numeric flags may still reject malformed diagnostic inputs.
- A reflection report records finite samples and the declared universe footprint.
  Stable samples do not prove stability at every level; citations do not prove
  that the referenced statements were formalized or checked.
- An apply graph separates dependency resolution from proof status. Kernel-like
  names and framework annotations do not check applications or their premises.
- Aggregation preserves structural obligations. An acceptance of some other
  statement cannot turn those observations into a discharged conjunction.

The `--kernel-discharged-axioms`, `--reflection-tower` and `--apply-graph` proof
gates return nonzero when they encounter these unverified obligations. This is
an expected diagnostic outcome, not a failure of the native proof checker.
JSON preserves structural counts separately from proof-discharge counts.

Native certification remains a different operation: an exact judgment and proof
term are replayed by the substitution and normalization checkers in
`verum_proof`. Source-level certification additionally requires faithful
elaboration and the exact source declaration. Passing a dependency audit, a
metadata checksum, or a finite test cannot replace those checks.

This change does not establish soundness of the legacy standard-library model or
all historical audit mechanisms. In particular, bibliography-only rule manifests
and bootstrap Verum models still require separate source correspondence audits.
