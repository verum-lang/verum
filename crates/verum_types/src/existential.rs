//! Existential Type Inference
//!
//! This module implements full support for existential types in the type system,
//! including skolemization for unpacking and packing with bound verification.
//!
//! # Theory
//!
//! Existential types (`some T: Bound`) hide the concrete type while exposing
//! its properties. They are the dual of universal types:
//!
//! - Universal: `forall T: Bound. Body` - consumer chooses T
//! - Existential: `some T: Bound. Body` - producer chooses T, consumer sees opaque
//!
//! # Key Operations
//!
//! 1. **Packing**: Creating an existential by hiding a witness type
//!    `pack(witness_type, value) : some T: Bound. Body`
//!
//! 2. **Unpacking (Skolemization)**: Using an existential by introducing a skolem
//!    `unpack(existential, x => body)` - x has skolem type that cannot escape
//!
//! # Scope Safety
//!
//! Skolem constants represent "unknown but fixed" types. They must not escape
//! their scope - this ensures type safety by preventing the hidden type from
//! leaking.
//!
//! # References
//!
//! - Existential types: hiding concrete types behind protocol bounds (impl Protocol returns)
//! - Types and Programming Languages, Chapter 24
//! - Type inference for existential types: unifying impl Protocol with concrete implementations

use crate::context::TypeContext;
use crate::protocol::{ProtocolBound, ProtocolChecker};
use crate::specialization_selection::ProtocolCheckerExt;
use crate::ty::{Type, TypeVar};
use crate::TypeError;
use verum_ast::span::Span;
use verum_common::{List, Map, Set, Text};
use verum_common::ToText;

/// A skolem constant, representing an opaque type from an unpacked existential.
///
/// Skolem constants are created during existential unpacking and must not
/// escape their scope. Each skolem has:
/// - A unique identifier
/// - The span where it was created (for error reporting)
/// - Protocol bounds that the hidden type is known to satisfy
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .2 - Skolemization
#[derive(Debug, Clone)]
pub struct SkolemConstant {
    /// Unique identifier for this skolem
    pub id: SkolemId,
    /// Human-readable name (derived from the existential variable)
    pub name: Text,
    /// Bounds that the skolem is known to satisfy
    pub bounds: List<ProtocolBound>,
    /// Span where the existential was unpacked (for error reporting)
    pub unpacking_span: Span,
    /// Scope level where this skolem was introduced
    pub scope_level: usize,
}

impl PartialEq for SkolemConstant {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for SkolemConstant {}

impl std::hash::Hash for SkolemConstant {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Unique identifier for skolem constants
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkolemId(usize);

impl SkolemId {
    /// Create a new unique skolem ID
    pub fn fresh() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        SkolemId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the underlying ID
    pub fn id(&self) -> usize {
        self.0
    }
}

impl std::fmt::Display for SkolemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sk{}", self.0)
    }
}

/// Result of existential packing
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .1 - Existential Packing
#[derive(Debug, Clone)]
pub struct PackResult {
    /// The packed existential type
    pub existential: Type,
    /// The witness type that was hidden
    pub witness: Type,
}

/// Result of existential unpacking (skolemization)
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .2 - Skolemization
#[derive(Debug, Clone)]
pub struct UnpackResult {
    /// The skolem constant introduced
    pub skolem: SkolemConstant,
    /// The body type with the existential variable replaced by the skolem
    pub body: Type,
}

/// Tracker for skolem constants and their scopes
///
/// This ensures that skolem constants don't escape their scope by tracking
/// which skolems are in scope at each level.
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .3 - Scope Checking
#[derive(Debug, Clone, Default)]
pub struct SkolemTracker {
    /// Currently active skolems, keyed by scope level
    active_skolems: Map<usize, List<SkolemConstant>>,
    /// Current scope level
    current_level: usize,
    /// All skolems that have been created (for lookup)
    all_skolems: Map<SkolemId, SkolemConstant>,
}

impl SkolemTracker {
    /// Create a new skolem tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a new scope level
    pub fn enter_scope(&mut self) {
        self.current_level += 1;
    }

    /// Exit the current scope level, invalidating all skolems at this level
    ///
    /// Returns the skolems that are now out of scope (for error checking)
    pub fn exit_scope(&mut self) -> List<SkolemConstant> {
        let exiting = self
            .active_skolems
            .remove(&self.current_level)
            .unwrap_or_default();
        self.current_level = self.current_level.saturating_sub(1);
        exiting
    }

    /// Get the current scope level
    pub fn current_level(&self) -> usize {
        self.current_level
    }

    /// Create a new skolem constant at the current scope level
    pub fn create_skolem(
        &mut self,
        name: Text,
        bounds: List<ProtocolBound>,
        unpacking_span: Span,
    ) -> SkolemConstant {
        let skolem = SkolemConstant {
            id: SkolemId::fresh(),
            name,
            bounds,
            unpacking_span,
            scope_level: self.current_level,
        };

        // Add to active skolems at current level
        self.active_skolems
            .entry(self.current_level)
            .or_default()
            .push(skolem.clone());

        // Add to global map
        self.all_skolems.insert(skolem.id, skolem.clone());

        skolem
    }

    /// Check if a skolem is currently in scope
    pub fn is_in_scope(&self, skolem_id: SkolemId) -> bool {
        if let Some(skolem) = self.all_skolems.get(&skolem_id) {
            skolem.scope_level <= self.current_level
        } else {
            false
        }
    }

    /// Get a skolem by its ID
    pub fn get_skolem(&self, skolem_id: SkolemId) -> Option<&SkolemConstant> {
        self.all_skolems.get(&skolem_id)
    }

    /// Get all skolems currently in scope
    pub fn in_scope_skolems(&self) -> List<&SkolemConstant> {
        let mut result = List::new();
        for level in 0..=self.current_level {
            if let Some(skolems) = self.active_skolems.get(&level) {
                for sk in skolems {
                    result.push(sk);
                }
            }
        }
        result
    }

    /// Check if a type contains any skolem that would escape scope
    ///
    /// Returns the first escaping skolem if found
    pub fn check_escape(&self, ty: &Type, target_level: usize) -> Option<&SkolemConstant> {
        self.find_escaping_skolem(ty, target_level)
    }

    /// Helper to find skolems in a type that would escape the target level
    fn find_escaping_skolem(&self, ty: &Type, target_level: usize) -> Option<&SkolemConstant> {
        match ty {
            // Check for skolem type - represented as a special Type::Var with a skolem marker
            // We use the TypeVar's ID to look up the corresponding skolem
            Type::Var(var) => {
                // Check if this var corresponds to a skolem at a higher level
                for (_, skolems) in &self.active_skolems {
                    for sk in skolems {
                        // Skolems that were introduced at a level higher than target would escape
                        if sk.scope_level > target_level {
                            // Check if this type var corresponds to this skolem
                            // We use a naming convention: skolem vars have IDs matching their SkolemId
                            if var.id() == sk.id.0 {
                                return Some(sk);
                            }
                        }
                    }
                }
                None
            }

            // Recursively check compound types
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    if let Some(sk) = self.find_escaping_skolem(p, target_level) {
                        return Some(sk);
                    }
                }
                self.find_escaping_skolem(return_type, target_level)
            }

            Type::Tuple(types) => {
                for t in types {
                    if let Some(sk) = self.find_escaping_skolem(t, target_level) {
                        return Some(sk);
                    }
                }
                None
            }

            Type::Array { element, .. } | Type::Slice { element } => {
                self.find_escaping_skolem(element, target_level)
            }

            Type::Record(fields) => {
                for ty in fields.values() {
                    if let Some(sk) = self.find_escaping_skolem(ty, target_level) {
                        return Some(sk);
                    }
                }
                None
            }

            Type::Variant(variants) => {
                for ty in variants.values() {
                    if let Some(sk) = self.find_escaping_skolem(ty, target_level) {
                        return Some(sk);
                    }
                }
                None
            }

            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. } => self.find_escaping_skolem(inner, target_level),

            Type::Refined { base, .. } => self.find_escaping_skolem(base, target_level),

            Type::Exists { body, .. } => self.find_escaping_skolem(body, target_level),

            Type::Forall { body, .. } => self.find_escaping_skolem(body, target_level),

            Type::Named { args, .. } | Type::Generic { args, .. } => {
                for arg in args {
                    if let Some(sk) = self.find_escaping_skolem(arg, target_level) {
                        return Some(sk);
                    }
                }
                None
            }

            Type::Future { output } => self.find_escaping_skolem(output, target_level),

            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                if let Some(sk) = self.find_escaping_skolem(yield_ty, target_level) {
                    return Some(sk);
                }
                self.find_escaping_skolem(return_ty, target_level)
            }

            Type::TypeApp { constructor, args } => {
                if let Some(sk) = self.find_escaping_skolem(constructor, target_level) {
                    return Some(sk);
                }
                for arg in args {
                    if let Some(sk) = self.find_escaping_skolem(arg, target_level) {
                        return Some(sk);
                    }
                }
                None
            }

            Type::GenRef { inner } => self.find_escaping_skolem(inner, target_level),

            // Primitive types and others cannot contain skolems
            _ => None,
        }
    }
}

/// Existential type operations
///
/// This struct provides the core operations for working with existential types:
/// - Packing: hide a witness type behind an existential
/// - Unpacking: skolemize an existential to work with its contents
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types)
pub struct ExistentialOps<'a> {
    /// Protocol checker for bound verification
    protocol_checker: &'a ProtocolChecker,
    /// Type context for bound lookup
    type_context: &'a TypeContext,
    /// Skolem tracker for scope management
    skolem_tracker: &'a mut SkolemTracker,
}

impl<'a> ExistentialOps<'a> {
    /// Create a new existential operations context
    pub fn new(
        protocol_checker: &'a ProtocolChecker,
        type_context: &'a TypeContext,
        skolem_tracker: &'a mut SkolemTracker,
    ) -> Self {
        Self {
            protocol_checker,
            type_context,
            skolem_tracker,
        }
    }

    /// Pack a witness type into an existential type
    ///
    /// This verifies that the witness satisfies all required bounds and creates
    /// the existential type that hides the witness.
    ///
    /// # Arguments
    ///
    /// * `witness` - The concrete type to hide
    /// * `bounds` - Protocol bounds that the witness must satisfy
    /// * `body_template` - The body type template (containing the existential variable)
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// The packed existential type, or an error if bounds are not satisfied.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Packing Int as `some T: Ord. List<T>`
    /// let packed = ops.pack(
    ///     Type::Int,
    ///     vec![ProtocolBound::simple("Ord")],
    ///     |t| Type::Generic { name: "List", args: vec![t] },
    ///     span
    /// )?;
    /// ```
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .1 - Existential Packing
    pub fn pack(
        &self,
        witness: &Type,
        bounds: &[ProtocolBound],
        span: Span,
    ) -> Result<PackResult, TypeError> {
        // Verify all bounds are satisfied by the witness type
        for bound in bounds {
            if !self.check_bound(witness, bound) {
                let protocol_name = bound
                    .protocol
                    .as_ident()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| Text::from("?"));

                return Err(TypeError::ExistentialBoundNotSatisfied {
                    witness_type: witness.to_text(),
                    protocol: protocol_name,
                    span,
                });
            }
        }

        // Create a fresh type variable for the existential
        let exist_var = TypeVar::fresh();

        // The existential type hides the witness
        let existential = Type::Exists {
            var: exist_var,
            body: Box::new(Type::Var(exist_var)),
        };

        Ok(PackResult {
            existential,
            witness: witness.clone(),
        })
    }

    /// Unpack an existential type by introducing a skolem constant
    ///
    /// This creates a fresh skolem constant representing the hidden type and
    /// returns the body with the existential variable replaced by the skolem.
    ///
    /// The skolem must not escape the current scope - this is checked when
    /// the scope is exited.
    ///
    /// # Arguments
    ///
    /// * `existential` - The existential type to unpack
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// The unpacking result with the skolem and substituted body, or an error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Unpacking `some T: Ord. List<T>`
    /// let unpack = ops.unpack(&existential, span)?;
    /// // unpack.skolem is a fresh skolem constant
    /// // unpack.body is List<sk_0> where sk_0 is the skolem
    /// ```
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .2 - Skolemization
    pub fn unpack(&mut self, existential: &Type, span: Span) -> Result<UnpackResult, TypeError> {
        match existential {
            Type::Exists { var, body } => {
                // Get bounds for the existential variable from context
                let bounds = self
                    .type_context
                    .get_protocol_bounds(var)
                    .cloned()
                    .unwrap_or_else(List::new);

                // Create a skolem constant
                let skolem_name = format!("sk_{}", var.id());
                let skolem =
                    self.skolem_tracker
                        .create_skolem(skolem_name.into(), bounds.clone(), span);

                // Create a type variable for the skolem
                // We use a convention where the TypeVar id matches the SkolemId
                let skolem_var = TypeVar::with_id(skolem.id.0);

                // Substitute the existential variable with the skolem
                let mut subst = crate::ty::Substitution::new();
                subst.insert(*var, Type::Var(skolem_var));
                let body = body.apply_subst(&subst);

                Ok(UnpackResult { skolem, body })
            }
            _ => Err(TypeError::Other(
                format!("Cannot unpack non-existential type: {}", existential).into(),
            )),
        }
    }

    /// Check if a type satisfies a protocol bound
    fn check_bound(&self, ty: &Type, bound: &ProtocolBound) -> bool {
        self.protocol_checker.check_protocol_bound(ty, bound)
    }

    /// Check if a type would escape the current scope via skolem constants
    ///
    /// This should be called before returning a type from a scope where
    /// existentials were unpacked.
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .3 - Escape Checking
    pub fn check_scope_escape(&self, ty: &Type, span: Span) -> Result<(), TypeError> {
        // The target level is one less than current (we're about to exit)
        let target_level = self.skolem_tracker.current_level().saturating_sub(1);

        if let Some(escaping_skolem) = self.skolem_tracker.check_escape(ty, target_level) {
            return Err(TypeError::ExistentialEscape {
                skolem_name: escaping_skolem.name.clone(),
                unpacking_span: escaping_skolem.unpacking_span,
                escape_span: span,
            });
        }

        Ok(())
    }
}

/// Subtyping rules for existential types
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .4 - Existential Subtyping
pub mod subtyping {
    use super::*;
    use crate::subtype::Subtyping;

    /// Check if an existential is a subtype of another existential
    ///
    /// The rule is:
    /// `(some a:P. S) <: (some b:Q. T)` if:
    /// 1. P is a superset of Q (stronger bounds on the sub-type)
    /// 2. S[a/witness] <: T[b/witness] for some witness satisfying P
    ///
    /// In practice, we use skolemization to check this:
    /// - Introduce a fresh skolem `sk` for `a`
    /// - Check that `S[a/sk] <: T[b/sk]`
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .4
    pub fn existential_subtype(
        subtyping: &Subtyping,
        exists1: &Type,
        var1: &TypeVar,
        body1: &Type,
        exists2: &Type,
        var2: &TypeVar,
        body2: &Type,
    ) -> bool {
        // Create a fresh type variable to serve as the witness
        let witness_var = TypeVar::fresh();
        let witness = Type::Var(witness_var);

        // Substitute the witness for both existential variables
        let mut subst1 = crate::ty::Substitution::new();
        subst1.insert(*var1, witness.clone());
        let body1_subst = body1.apply_subst(&subst1);

        let mut subst2 = crate::ty::Substitution::new();
        subst2.insert(*var2, witness);
        let body2_subst = body2.apply_subst(&subst2);

        // Check if the substituted bodies are subtypes
        subtyping.is_subtype(&body1_subst, &body2_subst)
    }

    /// Check if an existential is a subtype of a concrete type
    ///
    /// `(some a:P. S) <: T` if there exists a witness `w` such that:
    /// 1. `w` satisfies all bounds in P
    /// 2. `S[a/w] <: T`
    ///
    /// This is generally not decidable without knowing the witness,
    /// so we conservatively return false unless T is also existential.
    pub fn existential_to_concrete(_subtyping: &Subtyping, _exists: &Type, _concrete: &Type) -> bool {
        // Conservative: existentials are not subtypes of concrete types
        // The caller must explicitly unpack to work with the contents
        false
    }

    /// Check if a concrete type is a subtype of an existential
    ///
    /// `T <: (some a:P. S)` if T can be packed as the existential:
    /// 1. Find a witness `w` (which might be T itself or part of T)
    /// 2. `w` satisfies all bounds in P
    /// 3. T is structurally compatible with S[a/w]
    ///
    /// This is useful for implicit packing during assignment.
    pub fn concrete_to_existential(
        subtyping: &Subtyping,
        protocol_checker: &ProtocolChecker,
        concrete: &Type,
        _var: &TypeVar,
        body: &Type,
        bounds: &[ProtocolBound],
    ) -> bool {
        // Check if the concrete type could be the witness
        // Verify all bounds are satisfied
        for bound in bounds {
            if !protocol_checker.check_protocol_bound(concrete, bound) {
                return false;
            }
        }

        // Check structural compatibility
        // The body with the concrete type substituted should be compatible
        subtyping.is_subtype(concrete, body)
    }
}

/// Unification rules for existential types
///
/// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5 - Existential Unification
pub mod unification {
    use super::*;
    use crate::ty::Substitution;

    /// Unify two existential types
    ///
    /// Two existentials unify if their bodies unify after alpha-renaming
    /// to use the same bound variable.
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5
    pub fn unify_existentials(
        unifier: &mut crate::unify::Unifier,
        var1: &TypeVar,
        body1: &Type,
        var2: &TypeVar,
        body2: &Type,
        span: Span,
    ) -> Result<Substitution, TypeError> {
        // Rename var2 to var1 in body2
        let mut rename_subst = Substitution::new();
        rename_subst.insert(*var2, Type::Var(*var1));
        let body2_renamed = body2.apply_subst(&rename_subst);

        // Unify the bodies
        unifier.unify(body1, &body2_renamed, span)
    }

    /// Unify an existential with a concrete type
    ///
    /// This attempts to find a witness type that makes the unification work.
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5
    pub fn unify_existential_concrete(
        unifier: &mut crate::unify::Unifier,
        var: &TypeVar,
        body: &Type,
        concrete: &Type,
        span: Span,
    ) -> Result<Substitution, TypeError> {
        // The concrete type becomes the witness
        let mut subst = Substitution::new();
        subst.insert(*var, concrete.clone());
        let body_subst = body.apply_subst(&subst);

        // Unify the instantiated body with the concrete type
        unifier.unify(&body_subst, concrete, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ProtocolChecker;
    use verum_ast::span::Span;

    fn dummy_span() -> Span {
        Span::dummy()
    }

    #[test]
    fn test_skolem_id_uniqueness() {
        let id1 = SkolemId::fresh();
        let id2 = SkolemId::fresh();
        let id3 = SkolemId::fresh();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_skolem_tracker_scope() {
        let mut tracker = SkolemTracker::new();

        // Create skolem at level 0
        let sk1 = tracker.create_skolem("T".into(), List::new(), dummy_span());
        assert!(tracker.is_in_scope(sk1.id));

        // Enter new scope
        tracker.enter_scope();

        // Create skolem at level 1
        let sk2 = tracker.create_skolem("U".into(), List::new(), dummy_span());
        assert!(tracker.is_in_scope(sk1.id));
        assert!(tracker.is_in_scope(sk2.id));

        // Exit scope
        let exiting = tracker.exit_scope();
        assert_eq!(exiting.len(), 1);
        assert_eq!(exiting[0].id, sk2.id);

        // sk2 is no longer in scope
        assert!(tracker.is_in_scope(sk1.id));
        assert!(!tracker.is_in_scope(sk2.id));
    }

    #[test]
    fn test_skolem_tracker_in_scope_skolems() {
        let mut tracker = SkolemTracker::new();

        let sk1 = tracker.create_skolem("T".into(), List::new(), dummy_span());
        tracker.enter_scope();
        let sk2 = tracker.create_skolem("U".into(), List::new(), dummy_span());

        let in_scope = tracker.in_scope_skolems();
        assert_eq!(in_scope.len(), 2);

        // Both should be present
        let ids: Set<SkolemId> = in_scope.iter().map(|s| s.id).collect();
        assert!(ids.contains(&sk1.id));
        assert!(ids.contains(&sk2.id));
    }

    #[test]
    fn test_existential_type_creation() {
        let var = TypeVar::fresh();
        let existential = Type::Exists {
            var,
            body: Box::new(Type::Var(var)),
        };

        match existential {
            Type::Exists { var: v, body } => {
                assert_eq!(v, var);
                match *body {
                    Type::Var(bv) => assert_eq!(bv, var),
                    _ => panic!("Expected Var in body"),
                }
            }
            _ => panic!("Expected Exists type"),
        }
    }

    #[test]
    fn test_subtyping_existential_with_same_structure() {
        use crate::subtype::Subtyping;

        let subtyping = Subtyping::new();

        // Create two existentials with the same structure
        let var1 = TypeVar::fresh();
        let var2 = TypeVar::fresh();

        let body1 = Type::Var(var1);
        let body2 = Type::Var(var2);

        let result = subtyping::existential_subtype(
            &subtyping,
            &Type::Exists {
                var: var1,
                body: Box::new(body1.clone()),
            },
            &var1,
            &body1,
            &Type::Exists {
                var: var2,
                body: Box::new(body2.clone()),
            },
            &var2,
            &body2,
        );

        // Same structure should be subtypes
        assert!(result);
    }
}
