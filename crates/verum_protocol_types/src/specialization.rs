//! Specialization Lattice Type Definitions
//!
//! Specialization Lattice for Protocol Implementations:
//! Allows more specific implementations to override more general ones using
//! `@specialize` attribute. Lattice precedence (most specific wins):
//! 1. Concrete type (e.g., List<Int>)
//! 2. Partially specialized (e.g., List<T> where T: Copy)
//! 3. Generic (e.g., List<T>)
//! Soundness rules: specialized methods must have same return types as defaults.
//! Coherence: mutual exclusion via negative bounds (e.g., `T: Send + !Sync`).
//! The actual verification logic is in verum_smt.
//!
//! # Specialization Overview
//!
//! Specialization enables optimization through more specific implementations:
//!
//! ```verum
//! // General implementation
//! impl Display for List<T> where T: Display { ... }
//!
//! // Specialized for Text (more efficient)
//! @specialize
//! impl Display for List<Text> { ... }
//! ```
//!
//! The lattice ensures:
//! - Unique most-specific implementation for each type
//! - No ambiguous overlaps
//! - Antisymmetry (no cycles)
//! - Transitivity

use std::time::Duration;
use verum_ast::ty::Path;
use verum_common::{List, Map, Maybe, Set, Text};

use crate::protocol_base::{ProtocolBound, Type};

// ==================== Core Types ====================

/// Specialization metadata attached to protocol implementations
///
/// Metadata for a specialized protocol implementation. Tracks whether this impl
/// is marked `@specialize`, which default impl it overrides, its precedence level
/// in the specialization lattice, and the conditions under which it applies
/// (exact type match, protocol bound satisfaction, or negative bounds).
#[derive(Debug, Clone)]
pub struct SpecializationInfo {
    /// Whether this impl is marked as specializing
    pub is_specialization: bool,

    /// Which implementation this specializes (if any)
    pub specializes: Maybe<Text>,

    /// Precedence level (higher = more specific)
    pub precedence: usize,

    /// Conditions under which this specialization applies
    pub conditions: List<SpecializationCondition>,
}

/// Condition for specialization to apply
#[derive(Debug, Clone)]
pub enum SpecializationCondition {
    /// Type must be exactly this type
    ExactType {
        /// The exact type required
        ty: Type,
    },

    /// Type must satisfy this constraint
    Constraint {
        /// The constraint that must hold
        bound: ProtocolBound,
    },

    /// Negative constraint (type must NOT satisfy this)
    NegativeConstraint {
        /// The constraint that must NOT hold
        bound: ProtocolBound,
    },
}

/// The specialization lattice for a protocol
///
/// Tracks all implementations and their specificity relationships.
#[derive(Debug, Clone)]
pub struct SpecializationLattice {
    /// Protocol name
    pub protocol: Path,

    /// All implementations (indexed by ID)
    pub implementations: Map<usize, SpecializationNode>,

    /// Specificity ordering: (more_specific, less_specific)
    pub ordering: List<(usize, usize)>,

    /// Root implementations (most general)
    pub roots: Set<usize>,

    /// Leaf implementations (most specific)
    pub leaves: Set<usize>,
}

/// Node in the specialization lattice
#[derive(Debug, Clone)]
pub struct SpecializationNode {
    /// Implementation ID
    pub id: usize,

    /// The type this impl is for
    pub for_type: Type,

    /// Specialization metadata
    pub info: Maybe<SpecializationInfo>,

    /// More specific implementations
    pub specializations: Set<usize>,

    /// More general implementations
    pub generalizations: Set<usize>,
}

// ==================== Verification Types ====================

/// Specialization verification result
#[derive(Debug, Clone)]
pub struct SpecializationVerificationResult {
    /// Whether the specialization lattice is valid
    pub is_coherent: bool,
    /// Verification time
    pub duration: Duration,
    /// Errors found
    pub errors: List<SpecializationError>,
    /// Ambiguous specializations detected
    pub ambiguities: List<Ambiguity>,
    /// Statistics
    pub stats: SpecializationStats,
}

/// Specialization verification error
#[derive(Debug, Clone)]
pub enum SpecializationError {
    /// Ambiguous specialization (multiple equally-specific impls)
    AmbiguousSpecialization {
        /// Type causing ambiguity
        ty: Type,
        /// Protocol name
        protocol: Text,
        /// Candidate implementation IDs
        candidates: List<usize>,
    },

    /// Cycle in specialization lattice
    SpecializationCycle {
        /// Cycle of implementation IDs
        cycle: List<usize>,
    },

    /// Overlapping implementations without specialization
    OverlappingImpls {
        /// First implementation ID
        impl1: usize,
        /// Second implementation ID
        impl2: usize,
        /// Description of overlap
        overlap: Text,
    },

    /// Invalid specialization ordering
    InvalidOrdering {
        /// First implementation ID
        impl1: usize,
        /// Second implementation ID
        impl2: usize,
        /// Reason for invalidity
        reason: Text,
    },

    /// Antisymmetry violation (cycle in ordering)
    AntisymmetryViolation {
        /// First implementation ID
        impl1: usize,
        /// Second implementation ID
        impl2: usize,
    },
}

/// Ambiguous specialization case
#[derive(Debug, Clone)]
pub struct Ambiguity {
    /// The type causing ambiguity
    pub ty: Type,
    /// The protocol
    pub protocol: Text,
    /// Equally-specific implementations
    pub candidates: List<usize>,
    /// Explanation of why they're equally specific
    pub explanation: Text,
}

/// Specialization verification statistics
#[derive(Debug, Clone, Default)]
pub struct SpecializationStats {
    /// Total number of implementations checked
    pub impl_count: usize,
    /// Number of specialization relationships verified
    pub relationships_checked: usize,
    /// Number of overlap checks performed
    pub overlap_checks: usize,
    /// SMT solving time
    pub smt_time: Duration,
}

/// Specificity ordering between implementations
///
/// Determines which implementation is more specific than another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecificityOrdering {
    /// First implementation is more specific
    MoreSpecific,
    /// Second implementation is more specific
    LessSpecific,
    /// Implementations are equally specific (ambiguous)
    Equal,
    /// Implementations are incomparable (no overlap)
    Incomparable,
}

// ==================== Helper Implementations ====================

impl SpecializationLattice {
    /// Create a new empty lattice
    pub fn new(protocol: Path) -> Self {
        Self {
            protocol,
            implementations: Map::new(),
            ordering: List::new(),
            roots: Set::new(),
            leaves: Set::new(),
        }
    }

    /// Add an implementation to the lattice
    pub fn add_impl(&mut self, id: usize, for_type: Type, info: Maybe<SpecializationInfo>) {
        let node = SpecializationNode {
            id,
            for_type,
            info,
            specializations: Set::new(),
            generalizations: Set::new(),
        };
        self.implementations.insert(id, node);
    }

    /// Add a specificity relationship
    pub fn add_ordering(&mut self, more_specific: usize, less_specific: usize) {
        self.ordering.push((more_specific, less_specific));

        // Update node relationships
        if let Some(node) = self.implementations.get_mut(&more_specific) {
            node.generalizations.insert(less_specific);
        }
        if let Some(node) = self.implementations.get_mut(&less_specific) {
            node.specializations.insert(more_specific);
        }
    }

    /// Get all implementations for a type
    pub fn implementations_for(&self, ty: &Type) -> List<usize> {
        let mut result = List::new();
        for (id, node) in self.implementations.iter() {
            // Type matching would be done in verum_types
            result.push(*id);
        }
        result
    }

    /// Check if impl1 is more specific than impl2
    pub fn is_more_specific(&self, impl1: usize, impl2: usize) -> bool {
        self.ordering
            .iter()
            .any(|(more, less)| *more == impl1 && *less == impl2)
    }
}

impl SpecializationNode {
    /// Check if this is a root node (no generalizations)
    pub fn is_root(&self) -> bool {
        self.generalizations.is_empty()
    }

    /// Check if this is a leaf node (no specializations)
    pub fn is_leaf(&self) -> bool {
        self.specializations.is_empty()
    }
}

impl std::fmt::Display for SpecificityOrdering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecificityOrdering::MoreSpecific => write!(f, "more specific"),
            SpecificityOrdering::LessSpecific => write!(f, "less specific"),
            SpecificityOrdering::Equal => write!(f, "equally specific"),
            SpecificityOrdering::Incomparable => write!(f, "incomparable"),
        }
    }
}
