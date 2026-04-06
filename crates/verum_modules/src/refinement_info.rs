//! Refinement type information for module exports.
//!
//! This module stores refinement information without creating a circular dependency
//! with verum_types. The actual RefinementType is constructed by verum_types when needed.
//!
//! Refinement types work across module boundaries: when a type with refinements
//! is exported, the refinement becomes part of the public API contract. All three
//! refinement syntaxes (inline: Int{> 0}, declarative: Text where is_email,
//! sigma-type: x: Int where x > 0) work equivalently across modules.
//! Validation uses three tiers: compile-time (if provable), runtime (if not),
//! or unsafe cast (opt-in, no check).

use serde::{Deserialize, Serialize};
use verum_ast::{Expr, Ident, Span, ty::Type};
use verum_common::{List, Maybe, Text};

/// Refinement information stored with exported types.
///
/// This stores the raw AST components that verum_types can use to reconstruct
/// the full RefinementType. This avoids circular dependencies between verum_modules
/// and verum_types.
///
/// Stores raw AST components for refinement reconstruction by verum_types,
/// avoiding circular dependencies between verum_modules and verum_types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefinementInfo {
    /// The base type being refined
    pub base_type: Type,
    /// The refinement predicate expression
    pub predicate: Expr,
    /// The binding variable name (e.g., "it", "x")
    pub binding_var: Maybe<String>,
    /// Source location
    pub span: Span,
}

impl RefinementInfo {
    /// Create a new refinement info
    pub fn new(base_type: Type, predicate: Expr, binding_var: Option<String>, span: Span) -> Self {
        Self {
            base_type,
            predicate,
            binding_var: binding_var.map(Maybe::Some).unwrap_or(Maybe::None),
            span,
        }
    }

    /// Check if this is a trivial (unrefined) type
    pub fn is_trivial(&self) -> bool {
        // Check if predicate is a literal true
        matches!(
            &self.predicate.kind,
            verum_ast::expr::ExprKind::Literal(verum_ast::literal::Literal {
                kind: verum_ast::literal::LiteralKind::Bool(true),
                ..
            })
        )
    }
}

// =============================================================================
// REFINEMENT CONTRACT - Design-by-Contract for Cross-Module Verification
// =============================================================================

/// A single contract predicate (precondition, postcondition, or invariant).
///
/// Represents a logical assertion that must hold at specific points
/// in program execution.
///
/// # Examples
///
/// ```verum
/// @requires(x > 0)           // Predicate { kind: Requires, expr: x > 0 }
/// @ensures(result >= x)      // Predicate { kind: Ensures, expr: result >= x }
/// @invariant(self.len >= 0)  // Predicate { kind: Invariant, expr: self.len >= 0 }
/// ```
///
/// Used for Design-by-Contract verification at module boundaries and
/// cross-module refinement validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractPredicate {
    /// The kind of contract predicate
    pub kind: PredicateKind,
    /// The predicate expression (AST)
    pub expr: Expr,
    /// Optional label for error messages
    pub label: Maybe<Text>,
    /// Binding variables introduced by this predicate
    pub bindings: List<PredicateBinding>,
    /// Source span for error reporting
    pub span: Span,
}

/// The kind of contract predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PredicateKind {
    /// Precondition: must hold before function execution
    /// ```verum
    /// @requires(x > 0)
    /// ```
    Requires,

    /// Postcondition: must hold after function execution
    /// ```verum
    /// @ensures(result > input)
    /// ```
    Ensures,

    /// Invariant: must hold throughout execution
    /// ```verum
    /// @invariant(self.is_valid())
    /// ```
    Invariant,

    /// Modifies clause: declares what state may be changed
    /// ```verum
    /// @modifies(self.buffer, *ptr)
    /// ```
    Modifies,

    /// Decreases clause: proves termination
    /// ```verum
    /// @decreases(n)
    /// ```
    Decreases,
}

impl PredicateKind {
    /// Get the string representation for error messages
    pub fn as_str(&self) -> &'static str {
        match self {
            PredicateKind::Requires => "requires",
            PredicateKind::Ensures => "ensures",
            PredicateKind::Invariant => "invariant",
            PredicateKind::Modifies => "modifies",
            PredicateKind::Decreases => "decreases",
        }
    }
}

/// A binding introduced by a predicate.
///
/// Enables naming of intermediate values for clearer predicates:
/// ```verum
/// @ensures(old_len: self.len() => self.len() == old_len + 1)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredicateBinding {
    /// The binding name
    pub name: Text,
    /// The bound expression
    pub expr: Expr,
    /// When the binding is captured (Old = before, New = after)
    pub capture: CaptureTime,
}

/// When a binding value is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptureTime {
    /// Captured before function execution (`old(x)`)
    Old,
    /// Captured after function execution (default)
    New,
}

impl ContractPredicate {
    /// Create a new requires predicate
    pub fn requires(expr: Expr, span: Span) -> Self {
        Self {
            kind: PredicateKind::Requires,
            expr,
            label: Maybe::None,
            bindings: List::new(),
            span,
        }
    }

    /// Create a new ensures predicate
    pub fn ensures(expr: Expr, span: Span) -> Self {
        Self {
            kind: PredicateKind::Ensures,
            expr,
            label: Maybe::None,
            bindings: List::new(),
            span,
        }
    }

    /// Create a new invariant predicate
    pub fn invariant(expr: Expr, span: Span) -> Self {
        Self {
            kind: PredicateKind::Invariant,
            expr,
            label: Maybe::None,
            bindings: List::new(),
            span,
        }
    }

    /// Add a label to this predicate
    pub fn with_label(mut self, label: impl Into<Text>) -> Self {
        self.label = Maybe::Some(label.into());
        self
    }

    /// Add bindings to this predicate
    pub fn with_bindings(mut self, bindings: List<PredicateBinding>) -> Self {
        self.bindings = bindings;
        self
    }

    /// Check if this is a precondition
    pub fn is_requires(&self) -> bool {
        matches!(self.kind, PredicateKind::Requires)
    }

    /// Check if this is a postcondition
    pub fn is_ensures(&self) -> bool {
        matches!(self.kind, PredicateKind::Ensures)
    }

    /// Check if this is an invariant
    pub fn is_invariant(&self) -> bool {
        matches!(self.kind, PredicateKind::Invariant)
    }
}

/// Verification status for a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// Not yet verified
    Unverified,
    /// Successfully verified by SMT solver
    Verified,
    /// Verification failed with counterexample
    Failed,
    /// SMT solver timed out
    Timeout,
    /// Verification skipped (e.g., disabled by config)
    Skipped,
    /// Deferred to runtime checking
    RuntimeCheck,
}

impl VerificationStatus {
    /// Check if verification succeeded
    pub fn is_verified(&self) -> bool {
        matches!(self, VerificationStatus::Verified)
    }

    /// Check if verification needs runtime fallback
    pub fn needs_runtime_check(&self) -> bool {
        matches!(
            self,
            VerificationStatus::Unverified
                | VerificationStatus::Timeout
                | VerificationStatus::RuntimeCheck
        )
    }
}

/// A complete contract for a function or type.
///
/// Contracts enable Design-by-Contract programming and are verified at
/// module boundaries to ensure type safety across compilation units.
///
/// # Cross-Module Verification
///
/// When a function is exported:
/// 1. Its contract predicates are stored in the module's export table
/// 2. Callers verify they satisfy preconditions (`requires`)
/// 3. The callee is proven to satisfy postconditions (`ensures`)
/// 4. SMT verification at module boundaries ensures soundness
///
/// # Example
///
/// ```verum
/// @requires(x >= 0, "input must be non-negative")
/// @ensures(result >= 0)
/// @ensures(result * result <= x)
/// public fn sqrt(x: Float) -> Float {
///     // Implementation
/// }
/// ```
///
/// Used for Design-by-Contract verification at module boundaries and
/// cross-module refinement validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefinementContract {
    /// Preconditions (requires clauses) - must hold before execution
    pub requires: List<ContractPredicate>,

    /// Postconditions (ensures clauses) - guaranteed after execution
    pub ensures: List<ContractPredicate>,

    /// Invariants - must hold throughout execution
    pub invariants: List<ContractPredicate>,

    /// Modifies clauses - declares mutable state
    pub modifies: List<ContractPredicate>,

    /// Decreases clauses - proves termination
    pub decreases: List<ContractPredicate>,

    /// Overall verification status
    pub status: VerificationStatus,

    /// When this contract was last verified (Unix timestamp)
    pub verified_at: Maybe<u64>,

    /// Hash of the function body at verification time
    /// Used to detect when re-verification is needed
    pub body_hash: Maybe<u64>,

    /// Parameters referenced by the contract
    pub param_names: List<Text>,

    /// Return value name (usually "result")
    pub result_name: Text,
}

impl RefinementContract {
    /// Create an empty contract
    pub fn new() -> Self {
        Self {
            requires: List::new(),
            ensures: List::new(),
            invariants: List::new(),
            modifies: List::new(),
            decreases: List::new(),
            status: VerificationStatus::Unverified,
            verified_at: Maybe::None,
            body_hash: Maybe::None,
            param_names: List::new(),
            result_name: Text::from("result"),
        }
    }

    /// Create a contract with the given predicates
    pub fn with_predicates(predicates: impl IntoIterator<Item = ContractPredicate>) -> Self {
        let mut contract = Self::new();
        for pred in predicates {
            contract.add_predicate(pred);
        }
        contract
    }

    /// Add a predicate to the appropriate list
    pub fn add_predicate(&mut self, pred: ContractPredicate) {
        match pred.kind {
            PredicateKind::Requires => self.requires.push(pred),
            PredicateKind::Ensures => self.ensures.push(pred),
            PredicateKind::Invariant => self.invariants.push(pred),
            PredicateKind::Modifies => self.modifies.push(pred),
            PredicateKind::Decreases => self.decreases.push(pred),
        }
    }

    /// Add a requires predicate
    pub fn add_requires(&mut self, expr: Expr, span: Span) {
        self.requires.push(ContractPredicate::requires(expr, span));
    }

    /// Add an ensures predicate
    pub fn add_ensures(&mut self, expr: Expr, span: Span) {
        self.ensures.push(ContractPredicate::ensures(expr, span));
    }

    /// Add an invariant predicate
    pub fn add_invariant(&mut self, expr: Expr, span: Span) {
        self.invariants
            .push(ContractPredicate::invariant(expr, span));
    }

    /// Set parameter names for contract resolution
    pub fn with_params(mut self, params: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.param_names = params.into_iter().map(Into::into).collect();
        self
    }

    /// Set the result name
    pub fn with_result_name(mut self, name: impl Into<Text>) -> Self {
        self.result_name = name.into();
        self
    }

    /// Mark as verified
    pub fn mark_verified(mut self) -> Self {
        self.status = VerificationStatus::Verified;
        self.verified_at = Maybe::Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        );
        self
    }

    /// Mark as failed
    pub fn mark_failed(mut self) -> Self {
        self.status = VerificationStatus::Failed;
        self
    }

    /// Mark for runtime checking
    pub fn mark_runtime_check(mut self) -> Self {
        self.status = VerificationStatus::RuntimeCheck;
        self
    }

    /// Check if contract is empty (no predicates)
    pub fn is_empty(&self) -> bool {
        self.requires.is_empty()
            && self.ensures.is_empty()
            && self.invariants.is_empty()
            && self.modifies.is_empty()
            && self.decreases.is_empty()
    }

    /// Check if contract has preconditions
    pub fn has_requires(&self) -> bool {
        !self.requires.is_empty()
    }

    /// Check if contract has postconditions
    pub fn has_ensures(&self) -> bool {
        !self.ensures.is_empty()
    }

    /// Get all predicates as a flat list
    pub fn all_predicates(&self) -> List<&ContractPredicate> {
        let mut all = List::new();
        for p in &self.requires {
            all.push(p);
        }
        for p in &self.ensures {
            all.push(p);
        }
        for p in &self.invariants {
            all.push(p);
        }
        for p in &self.modifies {
            all.push(p);
        }
        for p in &self.decreases {
            all.push(p);
        }
        all
    }

    /// Count total predicates
    pub fn predicate_count(&self) -> usize {
        self.requires.len()
            + self.ensures.len()
            + self.invariants.len()
            + self.modifies.len()
            + self.decreases.len()
    }

    /// Check if re-verification is needed based on body hash
    pub fn needs_reverification(&self, current_body_hash: u64) -> bool {
        match &self.body_hash {
            Maybe::Some(hash) => *hash != current_body_hash,
            Maybe::None => true,
        }
    }

    /// Set the body hash for incremental verification
    pub fn with_body_hash(mut self, hash: u64) -> Self {
        self.body_hash = Maybe::Some(hash);
        self
    }
}

impl Default for RefinementContract {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CONTRACT EXTRACTION FROM AST ATTRIBUTES
// =============================================================================

/// Extract contracts from function attributes.
///
/// Parses `@requires`, `@ensures`, `@invariant`, `@modifies`, `@decreases`
/// attributes into a RefinementContract.
pub fn extract_contract_from_attributes(
    attrs: &[verum_ast::Attribute],
    param_names: &[Ident],
) -> RefinementContract {
    let mut contract = RefinementContract::new();

    // Set parameter names
    contract.param_names = param_names.iter().map(|i| Text::from(i.name.as_str())).collect();

    for attr in attrs {
        let name = attr.name.as_str();
        match name {
            "requires" => {
                if let Maybe::Some(ref args) = attr.args {
                    for expr in args {
                        contract.add_requires(expr.clone(), attr.span);
                    }
                }
            }
            "ensures" => {
                if let Maybe::Some(ref args) = attr.args {
                    for expr in args {
                        contract.add_ensures(expr.clone(), attr.span);
                    }
                }
            }
            "invariant" => {
                if let Maybe::Some(ref args) = attr.args {
                    for expr in args {
                        contract.add_invariant(expr.clone(), attr.span);
                    }
                }
            }
            "modifies" => {
                if let Maybe::Some(ref args) = attr.args {
                    for expr in args {
                        contract.modifies.push(ContractPredicate {
                            kind: PredicateKind::Modifies,
                            expr: expr.clone(),
                            label: Maybe::None,
                            bindings: List::new(),
                            span: attr.span,
                        });
                    }
                }
            }
            "decreases" => {
                if let Maybe::Some(ref args) = attr.args {
                    for expr in args {
                        contract.decreases.push(ContractPredicate {
                            kind: PredicateKind::Decreases,
                            expr: expr.clone(),
                            label: Maybe::None,
                            bindings: List::new(),
                            span: attr.span,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    contract
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::{expr::ExprKind, literal::{Literal, LiteralKind}};

    fn dummy_span() -> Span {
        Span::dummy()
    }

    fn dummy_expr() -> Expr {
        Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(true),
                span: dummy_span(),
            }),
            dummy_span(),
        )
    }

    #[test]
    fn test_empty_contract() {
        let contract = RefinementContract::new();
        assert!(contract.is_empty());
        assert_eq!(contract.predicate_count(), 0);
        assert!(!contract.has_requires());
        assert!(!contract.has_ensures());
    }

    #[test]
    fn test_add_requires() {
        let mut contract = RefinementContract::new();
        contract.add_requires(dummy_expr(), dummy_span());

        assert!(!contract.is_empty());
        assert_eq!(contract.predicate_count(), 1);
        assert!(contract.has_requires());
        assert!(!contract.has_ensures());
    }

    #[test]
    fn test_add_ensures() {
        let mut contract = RefinementContract::new();
        contract.add_ensures(dummy_expr(), dummy_span());

        assert!(!contract.is_empty());
        assert_eq!(contract.predicate_count(), 1);
        assert!(!contract.has_requires());
        assert!(contract.has_ensures());
    }

    #[test]
    fn test_verification_status() {
        let contract = RefinementContract::new().mark_verified();
        assert!(contract.status.is_verified());
        assert!(!contract.status.needs_runtime_check());

        let contract = RefinementContract::new().mark_runtime_check();
        assert!(!contract.status.is_verified());
        assert!(contract.status.needs_runtime_check());
    }

    #[test]
    fn test_body_hash_reverification() {
        let contract = RefinementContract::new().with_body_hash(12345);

        assert!(!contract.needs_reverification(12345));
        assert!(contract.needs_reverification(54321));
    }

    #[test]
    fn test_with_predicates() {
        let predicates = vec![
            ContractPredicate::requires(dummy_expr(), dummy_span()),
            ContractPredicate::ensures(dummy_expr(), dummy_span()),
            ContractPredicate::invariant(dummy_expr(), dummy_span()),
        ];

        let contract = RefinementContract::with_predicates(predicates);
        assert_eq!(contract.predicate_count(), 3);
        assert_eq!(contract.requires.len(), 1);
        assert_eq!(contract.ensures.len(), 1);
        assert_eq!(contract.invariants.len(), 1);
    }

    #[test]
    fn test_all_predicates() {
        let mut contract = RefinementContract::new();
        contract.add_requires(dummy_expr(), dummy_span());
        contract.add_ensures(dummy_expr(), dummy_span());

        let all = contract.all_predicates();
        assert_eq!(all.len(), 2);
    }
}
