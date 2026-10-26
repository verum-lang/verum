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
hashes and producer sources. CI runs the dispatch and finite-refinement
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
