#![allow(unexpected_cfgs)]
// Suppress informational clippy lints
#![allow(clippy::new_without_default)]
#![allow(clippy::large_enum_variant)]

//! # Verum Protocol Type Definitions
//!
//! This crate contains foundational protocol/trait type definitions needed by both
//! `verum_types` and `verum_smt` to break the circular dependency.
//!
//! ## Architecture
//!
//! This crate sits at LAYER 1.5 (between verum_ast and verum_types):
//!
//! ```text
//! LAYER 2: verum_types ──┐
//!                         ├──> LAYER 1.5: verum_protocol_types
//! LAYER 2: verum_smt ────┘           │
//!                                     ↓
//!                        LAYER 1: verum_ast, verum_common
//! ```
//!
//! ## Contents
//!
//! - **Protocol Type Definitions**: Core protocol/trait structures (without verification)
//! - **GAT Type Definitions**: Generic Associated Type structures
//! - **CBGR Predicate Types**: Generation tracking predicate types
//! - **Specialization Types**: Specialization lattice structures
//!
//! ## Design Principles
//!
//! 1. **No Verification Logic**: Only type definitions, no SMT or verification code
//! 2. **Semantic Types**: Use List, Text, Map from verum_common
//! 3. **Spec References**: Link to relevant specification sections
//! 4. **Zero Dependencies**: Minimal dependencies (only verum_common, verum_std, verum_ast)

#![allow(missing_docs)]
#![allow(unused_variables)]
#![allow(dead_code)]

pub mod cbgr_predicates;
pub mod gat_types;
pub mod protocol_base;
pub mod specialization;

// Re-export commonly used types
pub use cbgr_predicates::{
    CBGRCounterexample, CBGRPredicate, CBGRStats, CBGRVerificationResult, GenerationPredicate,
    ReferenceValue,
};
pub use gat_types::{
    AssociatedTypeGAT, AssociatedTypeKind, GATError, GATTypeParam, GATWhereClause, Kind, Variance,
};
pub use protocol_base::{
    AssociatedConst, AssociatedType, MethodResolution, MethodSource, ObjectSafetyError,
    Protocol, ProtocolBound, ProtocolImpl, ProtocolMethod, TypeParam, WhereClause,
};
// Re-export ConstValue from verum_common for protocol constants
pub use verum_common::ConstValue;
pub use specialization::{
    Ambiguity, SpecializationError, SpecializationInfo, SpecializationLattice, SpecializationStats,
    SpecializationVerificationResult, SpecificityOrdering,
};
