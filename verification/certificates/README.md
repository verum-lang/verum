# Portable execution contracts

These generators are untrusted proof producers. Acceptance requires the exact
statement and proof to pass bounded native replay (substitution and NbE) and
the independent Lean reference checker. The reference checker is an executable
algorithm; its acceptance is not a proof of that algorithm's soundness.

```sh
cargo build --locked --release -p verum_proof --features process --bin verum-proof
(cd verification/external/lean && lake build verum_replay_checker)
python3 verification/certificates/check_dispatch_contract.py \
  --native target/release/verum-proof \
  --lean verification/external/lean/.lake/build/bin/verum_replay_checker \
  --report owned-dispatch-acceptance.json
```

The report binds verdicts to exact judgments, checker identities, executable
hashes and producer sources. CI runs the dispatch, candidate-locality, diagnostic-fibre and finite-refinement
batteries. Rejection controls change the policy or evidence while retaining
the property being checked; admission-limit rejection is not a passing control.

## Dispatch correspondence

The contract is a projection of SYNARC `c80ce5455`, specifically
`rust/crates/synarc-core/src/plans.rs`, `plans/dispatch.rs` and `sym.rs`.
It does not claim a verified translation of those Rust files.

| Formal input or operation | Application obligation |
| --- | --- |
| `owned`, `ownerLaw` | For a queue-backed continuation, successful preflight establishes exact selected/current token identity. Allocation-backed tokens carry owner/revision identity. The exclusive frame borrow keeps preflight valid during synchronous dispatch. |
| `unchanged`, `actionLaw` | Lawful equality on complete proposed/dispatched actions, with no projection or hash substituted for identity. |
| `stop`, `advance` | The applicable continuation operation: walk/goal stop and next, or completed-walk disown and take-head. The theorem quantifies over these operations; their Rust implementation is not proved by doing so. |
| `Ok`, `Err`, `Unwind` | The actual callback result or the guard's unwind path. Adapter fidelity, panic behaviour and absence of process abort are external premises. |
| Envelope value, receipt and admissibility | Private fields of `Dispatch`; `map` retains admissibility, `try_map` failure sets it to false, and `assess` uses the retained value. Interpreter/context correctness is separate from ownership. |

The universal laws require full action/token identity before credit, preserve
foreign continuations, deny credit after error/unwind, and preserve receipt
identity and irreversible inadmissibility through interpretation. Transport
confirmation records the original result independently of interruption reason
and later interpretation failure. An interrupted `Ok` confirms transport while
withholding execution credit.

Controls reject owner/action/interruption bypasses, error/unwind credit, foreign
queue mutation, substituted receipts/observations, and restored admissibility.
The repeated-credit countermodel shows why retaining an old successful admission
fact cannot establish at-most-once execution. Fresh revision invalidation and
Rust affine consumption remain bridge obligations: portable Pi/Sigma terms are
freely duplicable. These certificates establish neither physical execution,
durable exactly-once delivery, the complete outcome accountant nor end-to-end
Rust refinement. The separate `ReceiptCredit.lean` model is an abstract policy,
not an implementation of this application boundary.

## Candidate-support locality

Run `check_candidate_locality.py` with the same `--native`, `--lean` and
`--report` arguments. It checks five universally quantified Pi/Sigma laws and
seven corrupted-evidence controls, without enumerating a finite situation set.

The projection concerns SYNARC `synarc-kan/src/exhaust/candidate_support.rs`
and `exhaust.rs::{hall_prefix_court_in_universe,judge_labelled_menu}`:

- At a fixed **valid rung**, an admitted domain is nonempty, lies inside the
  full situation universe, has sufficient count at every member, and contains
  both a time-extent witness and an event-extent witness. These two witnesses
  may be different members.
- Preserving membership and each support predicate on the old domain preserves
  admission of that **same domain**, hence the existence of an admitted domain.
  Disjoint evidence extension implies this premise only when old support and
  membership actually remain available. The certificate takes the preservation
  maps as premises; it does not prove that the Rust update implements them.
- The retained domain lies in the extended eligible set: its count-supported
  members supply the global time and event witnesses. Any selection contained
  in that eligible set lies inside the **full extended universe**, which need
  not equal the old universe. Cardinality alone is not the membership premise.
- Identity-score comparison is invariant under equality of the actual domain,
  with observations, alphabets, scoring function and base code held fixed.
  It gives no equality between scores for different selected domains.

The abstract predicates stand for the Rust count and extent comparisons. The
certificates do not establish their arithmetic implementation, balanced
coverage, statistical confidence, finite-map correspondence or Rust refinement.
Legal-rung validation must precede admission. Menu availability additionally
requires that the relevant cap does not shrink and that enumeration retains
that valid rung; no optimizer-completeness theorem is claimed. The selector
examines gain-ordered prefixes, not every subset. Its full-universe selection
price can change when unrelated evidence expands the universe, so purchase,
gain and elected-domain invariance are deliberately absent from the claims.

Controls keep the proposition fixed while substituting unpreserved support,
exchanging time/event evidence, confusing support with membership, or comparing
identity scores without using domain equality. Both native algorithms and
independent replay must reject the altered proof itself; resource-limit
rejection does not qualify.

## Diagnostic fibres and impossible reconstruction

`check_diagnostic_fibres.py` checks three general laws and six altered-evidence
controls. If two histories have the same projected diagnostic, every total
decoder agreeing with the desired query at both histories forces their answers
to be equal. A supplied inequality therefore rules out any such decoder and,
a fortiori, a decoder correct for every history. All controls retain the exact
claimed type while corrupting collision, endpoint or agreement evidence.

`VerumExternalReplay/DiagnosticFactorization.lean` proves the same mathematical
principle directly in Lean, for arbitrary types and functions. Its axiom audits
are build assertions. This is distinct from executing the reference checker on
portable certificates; neither route proves the Rust checker sound.

To apply this to a retention or merge implementation, independently establish
that the complete retained representation is equal and the intended historical
query differs. Omitting a field from the projection does not prove information
loss for the full representation. The theorem makes no claim about the correctness
of SYNARC's concrete histories, eviction/merge operations or causal-rank oracle.
