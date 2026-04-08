# Dependent Types Test Suite

This directory contains the comprehensive test suite for Verum's dependent type system as specified in section 14 of `docs/vcs-spec.md`.

## Overview

Dependent types are types that depend on values, enabling precise compile-time specifications that capture runtime behavior. This test suite validates the implementation of dependent types across five main categories.

## Directory Structure

```
dependent/
├── pi_types/           # Pi types (dependent functions)
│   ├── dependent_return.vr         # Return type depends on value
│   ├── dependent_return_fail.vr    # Failure cases
│   ├── length_tracking.vr          # List<T, n: meta Nat>
│   ├── length_tracking_fail.vr     # Length mismatch errors
│   ├── type_level_arithmetic.vr    # plus(M, N): meta Nat
│   ├── type_level_arithmetic_fail.vr
│   ├── pi_type_inference.vr        # Inference of dependent types
│   └── pi_type_inference_fail.vr
│
├── sigma_types/        # Sigma types (dependent pairs)
│   ├── refinement_desugaring.vr    # Int{> 0} as sigma
│   ├── refinement_desugaring_fail.vr
│   ├── proof_terms.vr              # Proof term construction
│   ├── proof_terms_fail.vr
│   ├── sigma_projections.vr        # Projection operations
│   ├── sigma_projections_fail.vr
│   ├── sigma_construction.vr       # Building sigma types
│   └── sigma_construction_fail.vr
│
├── indexed_types/      # Indexed inductive types
│   ├── fin_type.vr                 # Fin<N> bounded naturals
│   ├── fin_type_fail.vr
│   ├── vector_type.vr              # Vector<T, N>
│   ├── vector_type_fail.vr
│   ├── state_machine.vr            # Type-level state
│   ├── state_machine_fail.vr
│   ├── indexed_induction.vr        # Induction principles
│   └── indexed_induction_fail.vr
│
├── equality/           # Propositional equality
│   ├── propositional_equality.vr   # Eq<A, B>
│   ├── propositional_equality_fail.vr
│   ├── refl_symmetry_trans.vr      # Equality axioms
│   ├── refl_symmetry_trans_fail.vr
│   ├── substitution.vr             # Transport and substitution
│   ├── substitution_fail.vr
│   ├── equality_proofs.vr          # SMT-verified equality
│   └── equality_proofs_fail.vr
│
└── advanced/           # Advanced type features
    ├── inductive_types.vr          # General inductive types
    ├── inductive_types_fail.vr
    ├── coinductive_types.vr        # Streams, codata
    ├── coinductive_types_fail.vr
    ├── quantitative_types.vr       # Linear/affine types
    ├── quantitative_types_fail.vr
    ├── proof_irrelevance.vr        # Prop universe
    └── proof_irrelevance_fail.vr
```

## Test Categories

### 1. Pi Types (`pi_types/`)

Pi types (dependent function types) allow the return type of a function to depend on the value of its arguments.

**Key concepts tested:**
- Dependent return types: `fn replicate<T>(n: Nat, value: T) -> List<T, n: meta Nat>`
- Length-indexed lists with compile-time length tracking
- Type-level arithmetic: `plus`, `times`, `minus`, `div`, `mod`
- Type inference for dependent types

**Example:**
```verum
fn concat<T, M: meta Nat, N: meta Nat>(
    xs: List<T, M>,
    ys: List<T, N>
) -> List<T, plus(M, N): meta Nat> {
    xs ++ ys  // Result length is M + N
}
```

### 2. Sigma Types (`sigma_types/`)

Sigma types (dependent pair types) package a value together with a proof about that value. Refinement types desugar to sigma types.

**Key concepts tested:**
- NonEmpty as sigma: `(xs: List<T>, proof: Proof<len(xs) > 0>)`
- Proof term construction and manipulation
- Sigma projections (fst, snd)
- Smart constructors with validation

**Example:**
```verum
type Positive is (
    value: Int,
    proof: Proof<value > 0>
);
```

### 3. Indexed Types (`indexed_types/`)

Indexed inductive types are parameterized by values that constrain their structure.

**Key concepts tested:**
- `Fin<N>`: Natural numbers bounded by N
- `Vec<T, N>`: Length-indexed vectors
- Type-level state machines
- Indexed induction principles

**Example:**
```verum
fn safe_index<T, N: meta Nat>(arr: &[T; N], i: Fin<N>) -> T {
    arr[i.to_usize()]  // Cannot fail - i < N by construction
}
```

### 4. Equality Types (`equality/`)

Propositional equality types witness when two terms are definitionally equal.

**Key concepts tested:**
- Reflexivity, symmetry, transitivity
- Transport and substitution
- Congruence (applying functions to equalities)
- SMT-verified equality proofs

**Example:**
```verum
fn append_assoc<T>(xs: List<T>, ys: List<T>, zs: List<T>)
    -> Eq<append(append(xs, ys), zs), append(xs, append(ys, zs))>
{
    // Proof by induction on xs
}
```

### 5. Advanced Types (`advanced/`)

Advanced dependent type features including inductive/coinductive types, quantitative types, and proof irrelevance.

**Key concepts tested:**
- Inductive types and their elimination principles
- Coinductive types (streams, processes)
- Linear and affine types (quantitative type theory)
- Proof irrelevance and the Prop universe

**Example:**
```verum
// Linear file handling - must be used exactly once
fn safe_file_use(1 file: File) {
    let (content, file) = read_file(file);
    close_file(file);  // Must close - linear type ensures this
}
```

## Test Annotations

All tests use the following VCS annotations:

```verum
// @test: typecheck-pass | typecheck-fail | verify-pass | verify-fail
// @tier: all
// @level: L3
// @tags: dependent-types, <category>, [error]
// @expected-error: <error-code> "<message>"  // For fail tests
```

### Test Types

- `typecheck-pass`: Code should type-check successfully
- `typecheck-fail`: Code should be rejected by the type checker
- `verify-pass`: SMT verification should succeed
- `verify-fail`: SMT verification should fail

### Error Codes

- `E401`: Type mismatch
- `E302`: Use after move (linear types)
- `E502`: Refinement unsatisfiable / SMT verification failed

## Running Tests

```bash
# Run all dependent type tests
vtest vcs/specs/L3-extended/dependent/

# Run specific category
vtest vcs/specs/L3-extended/dependent/pi_types/

# Run only failure tests
vtest --filter "*_fail.vr" vcs/specs/L3-extended/dependent/

# Run with verbose output
vtest -v vcs/specs/L3-extended/dependent/
```

## Implementation Notes

### Type-Level vs Value-Level

Verum distinguishes between:
- `meta Nat`: Type-level natural numbers (compile-time)
- `Nat`: Value-level natural numbers (runtime)

Type-level values can be used in types and are erased at runtime.

### Proof Erasure

Proofs marked with `#[erased]` have no runtime representation:
```verum
type Refined<T, P: fn(T) -> Prop> is (
    value: T,
    #[erased] proof: P(value),  // Erased at runtime
);
```

### Linear Types

Usage annotations control how many times a value can be used:
- `1 T`: Used exactly once (linear)
- `0..1 T`: Used at most once (affine)
- `* T`: Used arbitrarily (unrestricted)

## Contributing

When adding new tests:
1. Place in the appropriate subdirectory
2. Include both pass and fail cases
3. Use descriptive comments explaining what's being tested
4. Follow the VCS annotation format
5. Reference the relevant specification section

## Related Documentation

- `docs/vcs-spec.md` Section 14: Dependent Types and Meta-system
- `docs/detailed/03-type-system.md`: Full type system specification
- `docs/detailed/12-dependent-types.md`: Dependent types detailed spec
