//! Verification Context and Boundary Tracking
//!
//! Implements the verification context system that tracks:
//! - Current verification level for each scope
//! - Verification boundaries between trusted/untrusted code
//! - Proof obligations at boundaries
//! - Context propagation through function calls
//!
//! The verification context tracks per-scope verification levels, boundaries between
//! trusted and untrusted code, and proof obligations that arise at those boundaries.
//! Context propagates through function calls so callee verification levels are
//! consistent with caller expectations.

use crate::Error;
use crate::level::{VerificationLevel, VerificationMode};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use verum_common::{List, Map, Maybe, Text, ToText};

/// Unique identifier for a verification scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopeId(u64);

impl ScopeId {
    /// Create a new scope ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Verification scope representing a lexical region with a verification level
///
/// Scopes form a tree structure matching the program's lexical structure.
/// Each scope has a verification level that determines how code in that
/// scope is verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationScope {
    /// Unique identifier for this scope
    pub id: ScopeId,

    /// Parent scope (None for root scope)
    pub parent: Maybe<ScopeId>,

    /// Verification mode for this scope
    pub mode: VerificationMode,

    /// Name/description of this scope (for debugging)
    pub name: Text,

    /// Child scopes
    pub children: List<ScopeId>,

    /// Active proof obligations for this scope
    pub obligations: List<ProofObligationId>,
}

impl VerificationScope {
    /// Create a new verification scope
    pub fn new(id: ScopeId, parent: Maybe<ScopeId>, mode: VerificationMode, name: Text) -> Self {
        Self {
            id,
            parent,
            mode,
            name,
            children: List::new(),
            obligations: List::new(),
        }
    }

    /// Create root scope with runtime verification
    pub fn root() -> Self {
        Self::new(
            ScopeId::new(0),
            Maybe::None,
            VerificationMode::runtime(),
            Text::from("root"),
        )
    }

    /// Get the verification level for this scope
    pub fn level(&self) -> VerificationLevel {
        self.mode.level
    }

    /// Check if this scope allows runtime fallback
    pub fn allows_runtime_fallback(&self) -> bool {
        self.mode.config.allow_runtime_fallback
    }

    /// Add a child scope
    pub fn add_child(&mut self, child_id: ScopeId) {
        self.children.push(child_id);
    }

    /// Add a proof obligation
    pub fn add_obligation(&mut self, obligation_id: ProofObligationId) {
        self.obligations.push(obligation_id);
    }
}

/// Verification context managing all verification scopes
///
/// The context tracks:
/// - All verification scopes in the program
/// - Current scope during analysis
/// - Verification boundaries
/// - Proof obligations
///
/// This is the central data structure for gradual verification analysis.
#[derive(Debug)]
pub struct VerificationContext {
    /// All scopes indexed by ID
    scopes: Arc<RwLock<Map<ScopeId, VerificationScope>>>,

    /// Next scope ID to allocate
    next_scope_id: Arc<RwLock<u64>>,

    /// Current scope during analysis
    current_scope: Arc<RwLock<ScopeId>>,

    /// All verification boundaries
    boundaries: Arc<RwLock<Map<BoundaryId, VerificationBoundary>>>,

    /// Next boundary ID to allocate
    next_boundary_id: Arc<RwLock<u64>>,

    /// All proof obligations
    obligations: Arc<RwLock<Map<ProofObligationId, ProofObligation>>>,

    /// Next obligation ID to allocate
    next_obligation_id: Arc<RwLock<u64>>,
}

impl VerificationContext {
    /// Create a new verification context with root scope
    pub fn new() -> Self {
        let root_scope = VerificationScope::root();
        let root_id = root_scope.id;

        let mut scopes = Map::new();
        scopes.insert(root_id, root_scope);

        Self {
            scopes: Arc::new(RwLock::new(scopes)),
            next_scope_id: Arc::new(RwLock::new(1)),
            current_scope: Arc::new(RwLock::new(root_id)),
            boundaries: Arc::new(RwLock::new(Map::new())),
            next_boundary_id: Arc::new(RwLock::new(0)),
            obligations: Arc::new(RwLock::new(Map::new())),
            next_obligation_id: Arc::new(RwLock::new(0)),
        }
    }

    /// Get the current scope
    pub fn current_scope(&self) -> ScopeId {
        *self.current_scope.read()
    }

    /// Get a scope by ID
    pub fn get_scope(&self, id: ScopeId) -> Maybe<VerificationScope> {
        let scopes = self.scopes.read();
        match scopes.get(&id) {
            Some(scope) => Maybe::Some(scope.clone()),
            None => Maybe::None,
        }
    }

    /// Get the current scope's verification level
    pub fn current_level(&self) -> VerificationLevel {
        let current_id = self.current_scope();
        match self.get_scope(current_id) {
            Maybe::Some(scope) => scope.level(),
            Maybe::None => VerificationLevel::Runtime,
        }
    }

    /// Create a new child scope
    pub fn push_scope(&mut self, mode: VerificationMode, name: Text) -> ScopeId {
        let parent_id = self.current_scope();
        let new_id = ScopeId::new(*self.next_scope_id.read());
        *self.next_scope_id.write() += 1;

        let new_scope = VerificationScope::new(new_id, Maybe::Some(parent_id), mode, name);

        // Add to parent's children
        match self.get_scope(parent_id) {
            Maybe::Some(mut parent) => {
                parent.add_child(new_id);
                self.scopes.write().insert(parent_id, parent);
            }
            Maybe::None => {}
        }

        // Insert new scope
        self.scopes.write().insert(new_id, new_scope);

        // Set as current
        *self.current_scope.write() = new_id;

        new_id
    }

    /// Exit the current scope, returning to parent
    pub fn pop_scope(&mut self) -> Result<(), Error> {
        let current_id = self.current_scope();
        let current = match self.get_scope(current_id) {
            Maybe::Some(scope) => scope,
            Maybe::None => return Err(Error::Internal("current scope not found".to_text())),
        };

        match current.parent {
            Maybe::Some(parent_id) => {
                *self.current_scope.write() = parent_id;
                Ok(())
            }
            Maybe::None => Err(Error::Internal("cannot pop root scope".to_text())),
        }
    }

    /// Register a verification boundary
    pub fn register_boundary(
        &mut self,
        from_level: VerificationLevel,
        to_level: VerificationLevel,
        kind: BoundaryKind,
    ) -> BoundaryId {
        let id = BoundaryId::new(*self.next_boundary_id.read());
        *self.next_boundary_id.write() += 1;

        let boundary = VerificationBoundary {
            id,
            from_level,
            to_level,
            kind,
            obligations: List::new(),
        };

        self.boundaries.write().insert(id, boundary);
        id
    }

    /// Add a proof obligation to a boundary
    pub fn add_obligation_to_boundary(
        &mut self,
        boundary_id: BoundaryId,
        obligation: ProofObligation,
    ) -> Result<ProofObligationId, Error> {
        let obligation_id = obligation.id;
        self.obligations.write().insert(obligation_id, obligation);

        let mut boundaries = self.boundaries.write();
        let boundary = match boundaries.get_mut(&boundary_id) {
            Some(b) => b,
            None => return Err(Error::Internal("boundary not found".to_text())),
        };

        boundary.obligations.push(obligation_id);
        Ok(obligation_id)
    }

    /// Get all boundaries
    pub fn boundaries(&self) -> List<VerificationBoundary> {
        self.boundaries
            .read()
            .values()
            .cloned()
            .collect::<List<_>>()
    }

    /// Get all unfulfilled proof obligations
    pub fn unfulfilled_obligations(&self) -> List<ProofObligation> {
        self.obligations
            .read()
            .values()
            .filter(|o| !o.fulfilled)
            .cloned()
            .collect::<List<_>>()
    }

    /// Mark an obligation as fulfilled
    pub fn fulfill_obligation(&mut self, id: ProofObligationId) -> Result<(), Error> {
        let mut obligations = self.obligations.write();
        let obligation = match obligations.get_mut(&id) {
            Some(o) => o,
            None => return Err(Error::Internal("obligation not found".to_text())),
        };

        obligation.fulfilled = true;
        Ok(())
    }

    /// Check if verification level transition is valid
    pub fn is_valid_transition(&self, from: VerificationLevel, to: VerificationLevel) -> bool {
        use VerificationLevel::*;

        match (from, to) {
            // Same level is always valid
            (Runtime, Runtime) | (Static, Static) | (Proof, Proof) => true,

            // Can always go to more restrictive verification
            (Runtime, Static) | (Runtime, Proof) | (Static, Proof) => true,

            // Going to less restrictive requires proof obligations
            (Static, Runtime) | (Proof, Runtime) | (Proof, Static) => false,
        }
    }
}

impl Default for VerificationContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a verification boundary
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BoundaryId(u64);

impl BoundaryId {
    /// Create a new boundary ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Verification boundary between code at different verification levels
///
/// Boundaries occur when:
/// - Calling from verified code into unverified code
/// - Returning from unverified code to verified code
/// - Crossing module boundaries with different verification levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationBoundary {
    /// Unique identifier
    pub id: BoundaryId,

    /// Source verification level
    pub from_level: VerificationLevel,

    /// Target verification level
    pub to_level: VerificationLevel,

    /// Kind of boundary
    pub kind: BoundaryKind,

    /// Proof obligations at this boundary
    pub obligations: List<ProofObligationId>,
}

impl VerificationBoundary {
    /// Check if this boundary requires proof obligations
    pub fn requires_obligations(&self) -> bool {
        // Transitioning to more restrictive level requires obligations
        use VerificationLevel::*;
        matches!(
            (self.from_level, self.to_level),
            (Runtime, Static) | (Runtime, Proof) | (Static, Proof)
        )
    }

    /// Get the direction of verification change
    pub fn direction(&self) -> BoundaryDirection {
        use VerificationLevel::*;
        match (self.from_level, self.to_level) {
            (Runtime, Static) | (Runtime, Proof) | (Static, Proof) => {
                BoundaryDirection::MoreRestrictive
            }
            (Static, Runtime) | (Proof, Runtime) | (Proof, Static) => {
                BoundaryDirection::LessRestrictive
            }
            _ => BoundaryDirection::Same,
        }
    }
}

/// Direction of verification level change at a boundary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryDirection {
    /// Moving to more restrictive verification
    MoreRestrictive,
    /// Moving to less restrictive verification
    LessRestrictive,
    /// Same verification level
    Same,
}

/// Kind of verification boundary
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryKind {
    /// Function call boundary
    FunctionCall,
    /// Function return boundary
    FunctionReturn,
    /// Module boundary
    Module,
    /// Closure capture boundary
    ClosureCapture,
    /// FFI boundary
    Ffi,
}

/// Unique identifier for a proof obligation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProofObligationId(u64);

impl ProofObligationId {
    /// Create a new obligation ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw ID value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// Proof obligation at a verification boundary
///
/// Represents a property that must be proven when crossing from less
/// restrictive to more restrictive verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofObligation {
    /// Unique identifier
    pub id: ProofObligationId,

    /// Kind of obligation
    pub kind: ObligationKind,

    /// Description of the property to prove
    pub description: Text,

    /// Whether this obligation has been fulfilled
    pub fulfilled: bool,

    /// Associated boundary
    pub boundary_id: Maybe<BoundaryId>,
}

impl ProofObligation {
    /// Create a new proof obligation
    pub fn new(
        id: ProofObligationId,
        kind: ObligationKind,
        description: Text,
        boundary_id: Maybe<BoundaryId>,
    ) -> Self {
        Self {
            id,
            kind,
            description,
            fulfilled: false,
            boundary_id,
        }
    }
}

/// Kind of proof obligation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObligationKind {
    /// Prove a refinement type constraint
    RefinementConstraint,

    /// Prove memory safety (no use-after-free)
    MemorySafety,

    /// Prove no data races
    DataRaceFreedom,

    /// Prove function precondition
    Precondition,

    /// Prove function postcondition
    Postcondition,

    /// Prove loop invariant
    LoopInvariant,

    /// Prove termination
    Termination,

    /// Custom proof obligation
    Custom,
}
