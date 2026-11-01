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
hashes and producer sources. CI runs the dispatch, candidate-locality, diagnostic-fibre, exact-transport and finite-refinement
batteries. Rejection controls change the policy or evidence while retaining
the property being checked; admission-limit rejection is not a passing control.

## Constructive natural arithmetic

`check_peano_arithmetic.py` checks addition, multiplication, structural order,
positive cancellation and polynomial identities. Their proofs use natural
induction and identity elimination; arithmetic and order laws are not assumed.
The untrusted polynomial normalizer emits ordinary proof terms. Supplied lemma
proofs are checked at explicit function applications. Independent lemmas share
balanced products within dependency layers, avoiding repeated proof expansion
and a linear-depth tower of cuts. Computational definitions remain transparent.

`test_arithmetic_terms.py` also exports the exact wire terms to Lean definitions
and asks Lean's kernel to check them, with empty axiom rosters. This is separate
from running the Lean reference checker. The exporter handles natural/list/unit/
empty arithmetic and nonindexed algebraic data with value and recursive fields.
Each complete schema gets a distinct Lean inductive declaration; value-field
types are explicit parameters and recursive hypotheses retain native order.
Dependent fields and unsupported declarations are refused. Both replay routes reject false order, zero-factor cancellation and
altered polynomial claims. These arithmetic laws alone do not establish a KT
partition bound or correspondence with a native scorer.

`check_fixed_kt.py` supplies that partition proof using the same arithmetic.
For declared alphabet `K = k + 1`, define
`R(a,n) = product(i < n, a + 2*i)`. A cell with counts `n_y` has likelihood
`product_y R(1,n_y) / R(K,sum_y n_y)`. For every finite partition with the
same coordinate-wise histogram, the maximum is
`product_y (R(1,n_y) / R(K,n_y))`.
The certificates prove positivity, mixed-cell splitting, same-outcome merging,
the bound, the diagonal witness's histogram and width, attainment, zero-cell
deletion and division by the fixed empirical parent likelihood (`0^0 = 1`).
The factor-range and counter-fold laws identify zero-origin grouped denominator
traversal with `R(K,sum_y n_y)`. They assume neither the inequalities nor
field/order axioms.

`test_fixed_kt_terms.py` checks the exact exported terms in Lean's kernel with
empty axiom rosters. False controls change K, replace per-outcome counts with
equal totals, omit histogram evidence, reverse merging, claim mixed-cell
attainment or admit K=0. The algebra is diagnostic only: a frozen Rust bridge
must separately establish coordinate identity, count conservation, actual
factors, exact arithmetic and refusal boundaries. No predictor, causal test,
admission, pruning or repricing authority follows from the maximum.

`verification/external/lean/check_fixed_kt_export.py` checks a frozen diagnostic
export against an independently supplied candidate roster. It reconstructs all
counts from ordered context/cell/outcome IDs and replays exact KT/ML fractions,
denominator traversal and envelope comparisons in Lean. Comparison requires
positive numerators and denominators; undefined fractions cannot pass by cross
multiplication. The report binds the export, captured model, producer, toolchain
and supplied native provenance to this invocation. It establishes finite
export correspondence, not a verified translation of Rust or laws for generic
`Clone`/`Ord`, BigUint arithmetic or physical ownership. Those remain separate
implementation obligations even when the native exporter reproduces exactly.

The frozen fixture's source bridge retains hashes for `Intake`, `Counter`,
`Positive`, the common-window caller and the test exporter. In those sources,
declared alphabets are retained by the same owner, parent and child increments
use the same outcome, the counter starts at zero, and positive ratios use
BigUint cross-cancellation. Intake invalidity and panic poison survive sealing;
count overflow is refused, work refusal precedes mutation, and unfinished or
invalid cursors cannot return a score. This is a source audit, not a formal Rust
semantics. In particular, equal completion totals alone do not prove outcome
conservation, and logical work units do not bound BigUint or generic `Ord` costs.

## Prefix codes and exact allocation

`check_prefix_codes.py` derives suffix-preserving decoding, prefix-freeness,
dependent sequencing and Kraft subprobability for binary parsing trees with
explicit rejection leaves. Concrete MSB-first fixed-width fields and gamma
cutoffs use the same definitions. Full fields have mass one; a gamma cutoff
allowing fewer than `d` leading zeros retains exactly `2^-d` outside its language.
Unused ordinal patterns are rejected without redistributing their mass.

Word concatenation multiplies its exact dyadic prices. The paid-look schedule
`1/((n+1)*(n+2))` has finite sum `n/(n+1)` and unspent tail `1/(n+1)`.
The certificates bound the joint name/look rectangle and distinguish the
optimistic opening allocation from the actual word price. They assume neither
the concrete decoder law nor the Kraft inequality. Direct Lean term replay has
empty axiom rosters; false controls cover truncation, discarded suffixes,
undefined ordinals, altered schemas, renormalized holes and reused look prices.

These are model laws for the stated constructors. A native grammar adapter must
bind the complete frozen catalogs, field widths, lexical child order, recursive
edges, actual code words, materialized bodies and retained pricing authority.
A V2/V3 catalog whose selected address uses no control does not demonstrate a
lexical or recursive encoding. Native VM semantics and physical ownership remain
separate from a code-word replay.

## Adaptive observations

`check_adaptive_observation.py` replays the universal laws in
`VerumExternalReplay/AdaptiveObservation.lean`: reversible XOR lifts, total and
Option readout transport, reflection of absence, and the separated-fibre
obstruction to reconstruction. Carriers have independent symbolic universe
levels. Products, existence and iff use constructive Sigma encodings; negation
targets an empty inductive type. The proof producer contains no project axioms.

Only the function-valued lift laws select `verum-native-funext/1`; the remaining
laws use the intensional profile. Foundation selection is verifier configuration,
and each report checks the exact requested profile. `--export-dir NEW_DIRECTORY`
retains the exact judgments and independent replay battery with their hashes.
The corresponding source audit is
`verification/external/lean/check_adaptive_observation.py`. False erased-input
and blind-readout claims must fail semantically. These are model proofs, without
a verified translation of runtime code or a statistical-risk/cost guarantee.

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

## Composition of exact partial-execution transports

`check_exact_transports.py` checks composition of `Option.map`, exact one-step
transport composition, definedness reflection and composition of injective
embeddings. The seven rejection controls retain their original claimed types
while omitting an embedding, transport/injectivity premise or preserving the
wrong definedness tag. `VerumExternalReplay.ExactTransports` independently proves
the mathematical statements in Lean with guarded axiom audits.

For arbitrary executions `f`, `g`, `h` and embeddings `i`, `j`, the premises are
`g(i(x)) = map(i, f(x))` and `h(j(y)) = map(j, g(y))`. Their composition gives
`h(j(i(x))) = map(j ∘ i, f(x))`. Definedness is preserved and reflected even
without injectivity; injectivity of the composite requires both injectivity
premises. These results compose supplied one-step contracts. They do not prove
that an acquisition grammar, extension, learner or Rust implementation satisfies
those contracts, or establish empirical generalization or physical execution.
