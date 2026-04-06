# verum_protocol_types

Foundational protocol/trait type definitions for the Verum compiler.

## Purpose

This crate breaks the circular dependency between `verum_types` and `verum_smt` by providing shared protocol type definitions without any verification logic.

## Dependency Resolution

**Before:**
```
verum_types ←→ verum_smt  (circular dependency)
```

**After:**
```
verum_types ──┐
               ├──> verum_protocol_types
verum_smt ────┘
```

## Architecture

This crate sits at LAYER 1.5 (between verum_ast and verum_types):

```
LAYER 2: Type System & Verification
├── verum_types   (type checking + inference)
└── verum_smt     (SMT verification)
        ↓
LAYER 1.5: Protocol Type Definitions
└── verum_protocol_types (pure type definitions)
        ↓
LAYER 1: Foundation
├── verum_ast     (AST definitions)
├── verum_core    (semantic types)
└── verum_std     (standard library)
```

## Contents

### Core Modules

1. **protocol_base** - Base protocol (trait) type definitions
   - `Protocol`: Protocol declarations
   - `ProtocolImpl`: Protocol implementations
   - `ProtocolBound`: Protocol constraints
   - `MethodResolution`: Method lookup metadata

2. **gat_types** - Generic Associated Types (GATs)
   - `AssociatedTypeGAT`: GATs with type parameters
   - `GATTypeParam`: Type parameters for GATs
   - `GATWhereClause`: Per-GAT constraints
   - `Variance`: Covariant/contravariant/invariant

3. **cbgr_predicates** - CBGR generation tracking predicates
   - `CBGRPredicate`: Generation/epoch/validity predicates
   - `GenerationPredicate`: Simplified predicate interface
   - `CBGRVerificationResult`: Verification results
   - `ReferenceValue`: Concrete reference values

4. **specialization** - Specialization lattice types
   - `SpecializationLattice`: Implementation ordering
   - `SpecializationInfo`: Specialization metadata
   - `SpecificityOrdering`: Relative specificity
   - `SpecializationError`: Coherence errors

## Design Principles

1. **No Verification Logic**: Only type definitions, no SMT or verification code
2. **Semantic Types**: Uses `List`, `Text`, `Map` from `verum_core`
3. **Spec References**: All types link to relevant specification sections
4. **Minimal Dependencies**: Only depends on `verum_core`, `verum_std`, `verum_ast`
5. **Zero-Cost**: Pure data structures with no runtime overhead

## Usage

This crate is not meant to be used directly by end users. It's an internal crate used by:

- **verum_types**: For protocol/trait type checking and inference
- **verum_smt**: For protocol verification and GAT well-formedness checking

## Specification References

- `docs/detailed/03-type-system.md` - Section 6: Protocol System
- `docs/detailed/18-advanced-protocols.md` - Advanced protocol features
  - Section 1: Generic Associated Types (GATs)
  - Section 2: CBGR Integration
  - Section 3: Specialization System
- `docs/detailed/26-cbgr-implementation.md` - CBGR memory model

## Performance

All types in this crate are zero-cost abstractions that exist only at compile-time. They impose no runtime overhead.

## Version

Version 1.0.0 - Part of the Verum v1.0 production release.
