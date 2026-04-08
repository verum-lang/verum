# L2-Standard Verification Tests

This directory contains comprehensive verification tests for the Verum Compliance Suite (VCS),
covering verification levels, contracts, SMT solving, and integration scenarios.

## Directory Structure

```
verification/
├── levels/                          # Verification level tests
│   ├── verify_runtime.vr            # @verify(runtime) - runtime checks
│   ├── verify_runtime_fail.vr       # Runtime verification failures
│   ├── verify_static.vr             # @verify(static) - static analysis
│   ├── verify_static_fail.vr        # Static verification failures
│   ├── verify_proof.vr              # @verify(proof) - SMT proving
│   ├── verify_proof_fail.vr         # Proof verification failures
│   └── level_fallback.vr            # Verification level fallback behavior
│
├── contracts/                       # Contract specification tests
│   ├── requires_clause.vr           # Preconditions
│   ├── requires_clause_fail.vr      # Precondition violations
│   ├── ensures_clause.vr            # Postconditions
│   ├── ensures_clause_fail.vr       # Postcondition violations
│   ├── invariant_clause.vr          # Loop and type invariants
│   ├── invariant_clause_fail.vr     # Invariant violations
│   ├── old_expression.vr            # old(expr) in ensures
│   ├── old_expression_fail.vr       # old() verification failures
│   ├── result_expression.vr         # result in ensures
│   ├── result_expression_fail.vr    # result verification failures
│   ├── forall_quantifier.vr         # Universal quantification
│   ├── forall_quantifier_fail.vr    # forall violations
│   ├── exists_quantifier.vr         # Existential quantification
│   ├── exists_quantifier_fail.vr    # exists violations
│   ├── contract_inheritance.vr      # Behavioral subtyping (LSP)
│   └── contract_inheritance_fail.vr # LSP violations
│
├── smt/                             # SMT solver integration tests
│   ├── arithmetic_verification.vr   # Linear/non-linear arithmetic
│   ├── arithmetic_verification_fail.vr
│   ├── array_verification.vr        # Array theory
│   ├── array_verification_fail.vr
│   ├── datatype_verification.vr     # Algebraic datatypes
│   ├── datatype_verification_fail.vr
│   ├── quantifier_instantiation.vr  # Quantifier handling
│   ├── quantifier_instantiation_fail.vr
│   ├── timeout_handling.vr          # @timeout: 5000
│   └── counterexample_generation.vr # SMT counterexamples
│
└── integration/                     # Cross-feature integration tests
    ├── contract_with_refinement.vr  # Contracts + refinement types
    ├── contract_with_refinement_fail.vr
    ├── contract_with_cbgr.vr        # Contracts + CBGR memory safety
    ├── contract_with_cbgr_fail.vr
    ├── contract_in_async.vr         # Contracts in async code
    └── contract_in_async_fail.vr
```

## Error Codes

The verification tests use the following error code ranges:

| Range | Category | Examples |
|-------|----------|----------|
| E5XX  | SMT/Verification Errors | E501-E510 |
| E6XX  | Contract Errors | E601-E610 |

### E5XX - SMT/Verification Errors

- **E501**: SMT verification failed - postcondition cannot be proven
- **E502**: SMT array theory verification failed
- **E503**: SMT datatype verification failed
- **E504**: SMT quantifier instantiation failed
- **E508**: Contract-refinement integration verification failed
- **E509**: Contract-CBGR integration verification failed
- **E510**: Contract-async integration verification failed

### E6XX - Contract Errors

- **E601**: Precondition not satisfied at call site
- **E602**: Postcondition cannot be verified / Static verification failed
- **E603**: Loop invariant violated
- **E604**: Old expression verification failed
- **E605**: Universal quantifier not satisfied
- **E606**: Existential quantifier not satisfied
- **E607**: Contract inheritance violation (LSP violated)

## Test Annotations

### Test Types

```verum
// @test: verify-pass    // Verification should succeed
// @test: verify-fail    // Verification should fail with @expected-error
// @test: run            // Execute and verify output
// @test: run-panic      // Execute and expect panic
// @test: typecheck-pass // Type checking should succeed
// @test: typecheck-fail // Type checking should fail
```

### Expected Errors

```verum
// @expected-error: E501 "SMT verification failed"
// @expected-error: E601 "Precondition not satisfied"
// @expected-panic: "Contract violation: requires y != 0.0"
```

### Verification Levels

```verum
@verify(runtime)              // Runtime checks (~15ns overhead)
@verify(static)               // Compile-time via type system (0ns)
@verify(proof)                // SMT solver verification
@verify(auto)                 // Automatic level selection
@verify(proof, fallback: static)  // Fallback chain
```

### Timeouts

```verum
// @timeout: 5000             // 5 second timeout for SMT solving
// @timeout: 100              // Short timeout for timeout tests
```

## Contract Syntax

### Basic Contracts

```verum
@verify(proof)
fn example(x: Int) -> Int {
    contract#"
        requires x > 0;           // Precondition
        ensures result >= x;       // Postcondition
    "
    x * 2
}
```

### Loop Invariants

```verum
while condition {
    invariant#"
        sum >= 0;
        i <= arr.len();
    "
    // loop body
}
```

### Type Invariants

```verum
struct BoundedList<T> {
    data: List<T>,
    max: Int,

    invariant#"
        data.len() <= max
    "
}
```

### Quantifiers

```verum
contract#"
    ensures forall i. 0 <= i < arr.len() implies arr[i] > 0;
    ensures exists j. 0 <= j < arr.len() && arr[j] == target;
"
```

### Old Expression

```verum
contract#"
    ensures *x == old(*x) + 1;
    ensures arr.len() == old(arr.len());
"
```

## Running Tests

```bash
# Run all L2 verification tests
vtest run vcs/specs/L2-standard/verification/

# Run only passing tests
vtest run vcs/specs/L2-standard/verification/ --filter verify-pass

# Run only failure tests
vtest run vcs/specs/L2-standard/verification/ --filter verify-fail

# Run with verbose SMT output
vtest run vcs/specs/L2-standard/verification/smt/ --smt-verbose

# Run with specific timeout
vtest run vcs/specs/L2-standard/verification/ --timeout 10000
```

## Coverage

These tests cover section 11 of the VCS specification (`docs/vcs-spec.md`):

- 11.1 Verification Levels (runtime, static, proof)
- 11.2 Contract Literals (requires, ensures, invariant)
- 11.3 SMT Integration (arithmetic, arrays, datatypes, quantifiers)
- 11.4 Integration with other language features (refinements, CBGR, async)
