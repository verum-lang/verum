//! CBGR Generation Tracking Predicate Types
//!
//! CBGR Generation Tracking Predicates for SMT Verification:
//! Provides type definitions for generation counter predicates used by the SMT
//! solver to verify reference safety at compile time. The actual SMT encoding
//! and verification logic is in verum_smt.
//!
//! # CBGR Memory Model
//!
//! ```text
//! ThinRef<T>:
//!   ptr: *const T      // 8 bytes
//!   generation: u64    // 8 bytes (48-bit generation + 16-bit epoch)
//!   Total: 16 bytes
//!
//! Generation counter layout (64-bit):
//!   Bits 0-47:  Generation (48 bits, ~281 trillion)
//!   Bits 48-63: Epoch (16 bits, 65536 epochs)
//! ```
//!
//! # Predicates
//!
//! - `generation(ref)` - Extract generation counter from reference
//! - `epoch(ref)` - Extract epoch counter from reference
//! - `valid(ref)` - Check if reference is still valid
//! - `same_allocation(a, b)` - Check if references point to same allocation

use std::time::Duration;
use verum_common::{List, Map, Maybe, Text};

// ==================== Core Types ====================

/// Result of CBGR predicate verification
#[derive(Debug, Clone)]
pub struct CBGRVerificationResult {
    /// Whether the property holds
    pub is_valid: bool,
    /// Verification time
    pub duration: Duration,
    /// Counterexample (if property violated)
    pub counterexample: Maybe<CBGRCounterexample>,
    /// Statistics
    pub stats: CBGRStats,
}

/// Counterexample showing CBGR property violation
#[derive(Debug, Clone)]
pub struct CBGRCounterexample {
    /// Reference values causing violation
    pub ref_values: Map<Text, ReferenceValue>,
    /// The violated property
    pub violated_property: Text,
    /// Explanation
    pub explanation: Text,
}

/// Concrete reference value
#[derive(Debug, Clone)]
pub struct ReferenceValue {
    /// Pointer value (symbolic or concrete)
    pub ptr: u64,
    /// Generation counter
    pub generation: u64,
    /// Epoch counter
    pub epoch: u16,
    /// Whether reference is valid
    pub is_valid: bool,
}

/// CBGR verification statistics
#[derive(Debug, Clone, Default)]
pub struct CBGRStats {
    /// Number of generation checks
    pub generation_checks: usize,
    /// Number of epoch checks
    pub epoch_checks: usize,
    /// Number of validity checks
    pub validity_checks: usize,
    /// Number of allocation checks
    pub allocation_checks: usize,
    /// SMT solving time
    pub smt_time: Duration,
}

// ==================== Predicate Definitions ====================

/// CBGR predicate for generation tracking
///
/// These predicates are used in refinement types to reason about
/// generation counters and reference validity.
///
/// Example:
/// ```verum
/// // Refinement type ensuring reference is valid
/// type ValidRef<T> = &T where valid(it)
///
/// // Refinement ensuring same allocation
/// fn compare_refs<T>(a: &T, b: &T) -> Bool
///     where same_allocation(a, b)
/// {
///     // ...
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CBGRPredicate {
    /// Extract generation counter: `generation(ref) -> u64`
    Generation {
        /// Reference being examined
        reference: Text,
    },

    /// Extract epoch counter: `epoch(ref) -> u16`
    Epoch {
        /// Reference being examined
        reference: Text,
    },

    /// Check reference validity: `valid(ref) -> bool`
    Valid {
        /// Reference to check
        reference: Text,
    },

    /// Check same allocation: `same_allocation(a, b) -> bool`
    SameAllocation {
        /// First reference
        ref_a: Text,
        /// Second reference
        ref_b: Text,
    },

    /// Generation comparison: `generation(a) op generation(b)`
    GenerationCompare {
        /// First reference
        ref_a: Text,
        /// Second reference
        ref_b: Text,
        /// Comparison operator
        op: ComparisonOp,
    },
}

/// Comparison operator for generation predicates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    /// Equal
    Eq,
    /// Not equal
    Ne,
    /// Less than
    Lt,
    /// Less than or equal
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal
    Ge,
}

/// Generation predicate for use in refinement types
///
/// This is a simplified version of CBGRPredicate that's easier to
/// construct and use in the type checker.
#[derive(Debug, Clone)]
pub struct GenerationPredicate {
    /// The predicate kind
    pub kind: CBGRPredicate,
    /// Source location (for error messages)
    pub span: verum_ast::span::Span,
}

impl GenerationPredicate {
    /// Create a generation extraction predicate
    pub fn generation(reference: Text) -> Self {
        Self {
            kind: CBGRPredicate::Generation { reference },
            span: verum_ast::span::Span::default(),
        }
    }

    /// Create an epoch extraction predicate
    pub fn epoch(reference: Text) -> Self {
        Self {
            kind: CBGRPredicate::Epoch { reference },
            span: verum_ast::span::Span::default(),
        }
    }

    /// Create a validity check predicate
    pub fn valid(reference: Text) -> Self {
        Self {
            kind: CBGRPredicate::Valid { reference },
            span: verum_ast::span::Span::default(),
        }
    }

    /// Create a same allocation check predicate
    pub fn same_allocation(ref_a: Text, ref_b: Text) -> Self {
        Self {
            kind: CBGRPredicate::SameAllocation { ref_a, ref_b },
            span: verum_ast::span::Span::default(),
        }
    }

    /// Create a generation comparison predicate
    pub fn generation_compare(ref_a: Text, ref_b: Text, op: ComparisonOp) -> Self {
        Self {
            kind: CBGRPredicate::GenerationCompare { ref_a, ref_b, op },
            span: verum_ast::span::Span::default(),
        }
    }
}

// ==================== Helper Functions ====================

impl CBGRPredicate {
    /// Get all references mentioned in this predicate
    pub fn referenced_variables(&self) -> List<Text> {
        let mut refs = List::new();
        match self {
            CBGRPredicate::Generation { reference }
            | CBGRPredicate::Epoch { reference }
            | CBGRPredicate::Valid { reference } => {
                refs.push(reference.clone());
            }
            CBGRPredicate::SameAllocation { ref_a, ref_b }
            | CBGRPredicate::GenerationCompare { ref_a, ref_b, .. } => {
                refs.push(ref_a.clone());
                refs.push(ref_b.clone());
            }
        }
        refs
    }

    /// Check if this predicate is side-effect free
    pub fn is_pure(&self) -> bool {
        // All CBGR predicates are pure (read-only)
        true
    }
}

impl std::fmt::Display for CBGRPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CBGRPredicate::Generation { reference } => write!(f, "generation({})", reference),
            CBGRPredicate::Epoch { reference } => write!(f, "epoch({})", reference),
            CBGRPredicate::Valid { reference } => write!(f, "valid({})", reference),
            CBGRPredicate::SameAllocation { ref_a, ref_b } => {
                write!(f, "same_allocation({}, {})", ref_a, ref_b)
            }
            CBGRPredicate::GenerationCompare { ref_a, ref_b, op } => {
                write!(f, "generation({}) {} generation({})", ref_a, op, ref_b)
            }
        }
    }
}

impl std::fmt::Display for ComparisonOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComparisonOp::Eq => write!(f, "=="),
            ComparisonOp::Ne => write!(f, "!="),
            ComparisonOp::Lt => write!(f, "<"),
            ComparisonOp::Le => write!(f, "<="),
            ComparisonOp::Gt => write!(f, ">"),
            ComparisonOp::Ge => write!(f, ">="),
        }
    }
}
