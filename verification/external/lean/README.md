# Verum external replay

Use the toolchain pinned in `lean-toolchain`. From this directory:

```sh
LEAN_NUM_THREADS=1 lake build
LEAN_NUM_THREADS=1 lake build verum_replay_checker
LEAN_NUM_THREADS=1 lake env lean VerumExternalReplay/ConservativeRefinement.lean
```

`ReferenceChecker` independently checks native certificate terms. The executable
decodes the certificate battery used by the Verum audit. From the repository root,
run its equality and rejection controls against the native direct and NbE checkers:

```sh
VERUM_LEAN_REPLAY="$PWD/verification/external/lean/.lake/build/bin/verum_replay_checker" \
  cargo test --locked -p verum_proof --test lean_replay -- --ignored
```

## Conservative representation refinement

`ConservativeRefinement.finite_execution_preserved` proves by list induction that
a selected finite action word retains its complete reached-state trace and exact
continuation, termination or refusal result under projection. Action lifting is
state dependent. Step commutation is required at every reached admissible action;
actions after termination or refusal are not executed. The observation theorem
additionally requires pointwise observation commutation.

The initial refined state is given. Covering every previous state additionally
requires an executable initial lift `embed` with `project (embed s) = s` and
admissibility of the transferred programs, or an explicitly restricted domain.

The module contains seven negative mathematical counterexamples and one positive
first-step equality. These are distinct from the serialized certificate rejection
tests above. Both theorems report only Lean's standard `propext` axiom. There are
no admitted proofs or new axioms asserting the result. Rechecking relies on the
Lean kernel, its standard library and the execution environment. The existing
native certificate checkers remain separate trusted implementations.

This establishes an abstract finite-word theorem in Lean. It does not establish
the corresponding native Verum induction certificate, adaptive-controller or
stochastic preservation, correctness of an external Rust implementation, learning
quality, search completeness or benchmark performance. The general named-term
kernel's induction boundary requires separate validation; step certificates alone
are not a proof of the finite-word induction theorem.
