//! Type context and environment for type checking.
//!
//! This module provides the typing context that tracks:
//! - Variable bindings and their types
//! - Type schemes for let-polymorphism
//! - Protocol implementations
//! - Context permissions (for Level 2 dynamic contexts)
//! - Type parameters with bounds: generic constraints (T: Protocol) tracked and verified at instantiation
//! - Module tracking for qualified type resolution (Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — )

use crate::protocol::ProtocolBound;
use crate::ty::{Substitution, Type, TypeVar, UniverseLevel};
pub use crate::variance::Variance;
use indexmap::IndexMap;
use verum_ast::span::Span;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Set, Text};

// Module system integration
// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
pub use verum_modules::ModuleId;

/// A type scheme represents a polymorphic type: ∀α β. T
///
/// This enables let-polymorphism where bound variables can
/// be instantiated differently at each use site.
///
/// Implicit arguments: compiler-inferred function arguments resolved by unification or type class search — Implicit arguments
#[derive(Debug, Clone, PartialEq)]
pub struct TypeScheme {
    /// Universally quantified type variables
    pub vars: List<TypeVar>,
    /// The body type
    pub ty: Type,
    /// Type variables that are implicit (inferred from context).
    /// Implicit parameters use `{...}` syntax: `fn id<{T}>(x: T) -> T`
    pub implicit_vars: Set<TypeVar>,
    /// Direct type bounds for type variables (e.g., F: fn() -> T).
    /// Maps type variables to their type bounds (function types, etc.).
    /// Unlike protocol bounds which use paths, these store actual Type values.
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    pub var_type_bounds: Map<TypeVar, List<Type>>,
    /// Protocol bounds for type variables (e.g., T: Eq + Ord).
    /// Maps original type variables to their protocol constraints.
    /// These are checked at instantiation sites when the function is called.
    pub var_protocol_bounds: Map<TypeVar, List<ProtocolBound>>,
    /// Number of leading type variables that come from the implement block.
    /// Used for instance methods to distinguish impl-level vars (bound from
    /// receiver type args) from method-level vars (inferred from arguments).
    /// When 0 (default), all vars are treated as potentially bindable from
    /// receiver type args (backward compatible).
    pub impl_var_count: usize,
}

impl TypeScheme {
    /// Create a monomorphic type scheme (no quantification)
    pub fn mono(ty: Type) -> Self {
        Self {
            vars: List::new(),
            ty,
            implicit_vars: Set::new(),
            var_type_bounds: Map::new(),
            var_protocol_bounds: Map::new(),
            impl_var_count: 0,
        }
    }

    /// Create a polymorphic type scheme
    pub fn poly(vars: List<TypeVar>, ty: Type) -> Self {
        Self {
            vars,
            ty,
            implicit_vars: Set::new(),
            var_type_bounds: Map::new(),
            var_protocol_bounds: Map::new(),
            impl_var_count: 0,
        }
    }

    /// Create a polymorphic type scheme with some implicit parameters.
    /// Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
    pub fn poly_with_implicit(vars: List<TypeVar>, ty: Type, implicit_vars: Set<TypeVar>) -> Self {
        Self {
            vars,
            ty,
            implicit_vars,
            var_type_bounds: Map::new(),
            var_protocol_bounds: Map::new(),
            impl_var_count: 0,
        }
    }

    /// Create a polymorphic type scheme with type bounds.
    /// This is used for generic functions/methods with function type constraints
    /// like `F: fn(T) -> U`.
    pub fn poly_with_type_bounds(
        vars: List<TypeVar>,
        ty: Type,
        var_type_bounds: Map<TypeVar, List<Type>>,
    ) -> Self {
        Self {
            vars,
            ty,
            implicit_vars: Set::new(),
            var_type_bounds,
            var_protocol_bounds: Map::new(),
            impl_var_count: 0,
        }
    }

    /// Add type bounds to an existing scheme.
    /// Returns a new scheme with the added bounds.
    ///
    /// CRITICAL: This also adds any free vars from the bounds to the scheme's vars
    /// if not already present. This ensures that when instantiating, all vars
    /// in the bounds get substituted with fresh vars.
    ///
    /// For example, for `fn map<U, F: fn(T) -> U>` where T is from the impl block:
    /// - method_ty is `fn(F) -> Maybe<U>` with free vars {F, U}
    /// - but the bound `fn(T) -> U` has free var T
    /// - we need to include T in the scheme so it gets substituted
    pub fn with_type_bounds(mut self, var_type_bounds: Map<TypeVar, List<Type>>) -> Self {
        // Collect all free vars from the bounds
        let mut bounds_vars: Set<TypeVar> = Set::new();
        for bounds in var_type_bounds.values() {
            for bound_ty in bounds {
                for v in bound_ty.free_vars() {
                    if !self.vars.contains(&v) {
                        bounds_vars.insert(v);
                    }
                }
            }
        }

        // Add any vars from bounds that aren't already in the scheme
        // Insert them at the beginning since they're typically impl-level params
        if !bounds_vars.is_empty() {
            let mut new_vars: List<TypeVar> = bounds_vars.into_iter().collect();
            new_vars.extend(self.vars.clone());
            self.vars = new_vars;
        }

        self.var_type_bounds = var_type_bounds;
        self
    }

    /// Add protocol bounds to an existing scheme.
    pub fn with_protocol_bounds(mut self, var_protocol_bounds: Map<TypeVar, List<ProtocolBound>>) -> Self {
        self.var_protocol_bounds = var_protocol_bounds;
        self
    }

    /// Check if a type variable is implicit (inferred from context)
    pub fn is_implicit(&self, var: &TypeVar) -> bool {
        self.implicit_vars.contains(var)
    }

    /// Get the number of explicit (non-implicit) type parameters
    pub fn explicit_var_count(&self) -> usize {
        self.vars.iter().filter(|v| !self.implicit_vars.contains(v)).count()
    }

    /// Get the number of implicit type parameters
    pub fn implicit_var_count(&self) -> usize {
        self.implicit_vars.len()
    }

    /// Instantiate the type scheme with fresh type variables
    pub fn instantiate(&self) -> Type {
        if self.vars.is_empty() {
            return self.ty.clone();
        }

        let mut subst = Substitution::new();
        for var in &self.vars {
            subst.insert(*var, Type::Var(TypeVar::fresh()));
        }

        self.ty.apply_subst(&subst)
    }

    /// Instantiate the type scheme and return both the type and the ordered list of fresh type vars
    ///
    /// This is critical for correctly binding receiver type args to method type params.
    /// The returned Vec<TypeVar> preserves the order of vars in the scheme, which should match
    /// the order of type params in the original implement block + method declaration.
    pub fn instantiate_with_fresh_vars(&self) -> (Type, List<TypeVar>) {
        if self.vars.is_empty() {
            return (self.ty.clone(), List::new());
        }

        let mut subst = Substitution::new();
        let mut fresh_vars = List::new();
        for var in &self.vars {
            let fresh = TypeVar::fresh();
            subst.insert(*var, Type::Var(fresh));
            fresh_vars.push(fresh);
        }

        (self.ty.apply_subst(&subst), fresh_vars)
    }

    /// Instantiate the type scheme and return the type, fresh vars, and which are implicit.
    ///
    /// This is needed for implicit argument resolution where we need to know which
    /// fresh type variables should be inferred vs. explicitly provided.
    /// Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
    pub fn instantiate_with_implicit_info(&self) -> (Type, List<TypeVar>, Set<TypeVar>) {
        if self.vars.is_empty() {
            return (self.ty.clone(), List::new(), Set::new());
        }

        let mut subst = Substitution::new();
        let mut fresh_vars = List::new();
        let mut fresh_implicit = Set::new();

        for var in &self.vars {
            let fresh = TypeVar::fresh();
            subst.insert(*var, Type::Var(fresh));
            fresh_vars.push(fresh);

            // Track if the original var was implicit
            if self.implicit_vars.contains(var) {
                fresh_implicit.insert(fresh);
            }
        }

        (self.ty.apply_subst(&subst), fresh_vars, fresh_implicit)
    }

    /// Instantiate the type scheme and return type, fresh vars, and type bounds mapped to fresh vars.
    ///
    /// This is essential for proper closure type inference:
    /// - When a method like `map<U, F: fn(T) -> U>` is instantiated
    /// - We need to know that the fresh var for F has a function type bound
    /// - This enables checking closures against bounded type variables
    ///
    /// Returns: (instantiated_type, fresh_vars, type_bounds_for_fresh_vars)
    /// The Map maps fresh TypeVar to its type bounds (if any).
    pub fn instantiate_with_type_bounds(&self) -> (Type, List<TypeVar>, Map<TypeVar, List<Type>>) {
        if self.vars.is_empty() {
            return (self.ty.clone(), List::new(), Map::new());
        }

        let mut subst = Substitution::new();
        let mut fresh_vars = List::new();
        let mut old_to_fresh: Map<TypeVar, TypeVar> = Map::new();

        for var in &self.vars {
            let fresh = TypeVar::fresh();
            subst.insert(*var, Type::Var(fresh));
            fresh_vars.push(fresh);
            old_to_fresh.insert(*var, fresh);
        }

        // Map type bounds from old vars to fresh vars
        let mut fresh_bounds: Map<TypeVar, List<Type>> = Map::new();
        for (old_var, bounds) in &self.var_type_bounds {
            if let Some(fresh_var) = old_to_fresh.get(old_var) {
                // Apply substitution to bounds to replace old vars with fresh ones
                let mapped_bounds: List<Type> = bounds
                    .iter()
                    .map(|b| b.apply_subst(&subst))
                    .collect();
                fresh_bounds.insert(*fresh_var, mapped_bounds);
            }
        }

        (self.ty.apply_subst(&subst), fresh_vars, fresh_bounds)
    }

    /// Instantiate the type scheme and return the protocol bounds mapped to fresh vars.
    ///
    /// Returns (instantiated_type, fresh_vars, protocol_bounds_for_fresh_vars).
    /// Used at call sites to verify concrete types satisfy protocol constraints.
    pub fn instantiate_with_protocol_bounds(&self) -> (Type, List<TypeVar>, Map<TypeVar, List<ProtocolBound>>) {
        if self.vars.is_empty() {
            return (self.ty.clone(), List::new(), Map::new());
        }

        let mut subst = Substitution::new();
        let mut fresh_vars = List::new();
        let mut old_to_fresh: Map<TypeVar, TypeVar> = Map::new();

        for var in &self.vars {
            let fresh = TypeVar::fresh();
            subst.insert(*var, Type::Var(fresh));
            fresh_vars.push(fresh);
            old_to_fresh.insert(*var, fresh);
        }

        // Map protocol bounds from old vars to fresh vars
        let mut fresh_protocol_bounds: Map<TypeVar, List<ProtocolBound>> = Map::new();
        for (old_var, bounds) in &self.var_protocol_bounds {
            if let Some(fresh_var) = old_to_fresh.get(old_var) {
                fresh_protocol_bounds.insert(*fresh_var, bounds.clone());
            }
        }

        (self.ty.apply_subst(&subst), fresh_vars, fresh_protocol_bounds)
    }

    /// Get free type variables in the scheme
    pub fn free_vars(&self) -> Set<TypeVar> {
        let mut vars = self.ty.free_vars();
        for v in &self.vars {
            vars.remove(v);
        }
        vars
    }
}

/// Type parameter with bounds and variance
/// Tensor types: Tensor<T, Shape: meta [usize]> with compile-time shape tracking for N-dimensional arrays
///
/// Used for generic functions and types with protocol constraints.
/// Example: `fn sort<T>(list: List<T>) where type T: Ord { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    /// Parameter name (e.g., T, U, K, V)
    pub name: Text,

    /// Protocol bounds (e.g., T: Ord + Clone)
    pub bounds: List<ProtocolBound>,

    /// Default type (e.g., T = Int)
    pub default: Maybe<Type>,

    /// Variance (covariant, contravariant, invariant)
    pub variance: Variance,

    /// Is this a meta (compile-time) parameter?
    pub is_meta: bool,

    /// Source location
    pub span: Span,
}

impl TypeParam {
    /// Create a new type parameter
    pub fn new(name: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            bounds: List::new(),
            default: Maybe::None,
            variance: Variance::Invariant, // Default to safe invariance
            is_meta: false,
            span,
        }
    }

    /// Add a protocol bound
    pub fn with_bound(mut self, bound: ProtocolBound) -> Self {
        self.bounds.push(bound);
        self
    }

    /// Add multiple bounds
    pub fn with_bounds(mut self, bounds: List<ProtocolBound>) -> Self {
        self.bounds.extend(bounds);
        self
    }

    /// Set default type
    pub fn with_default(mut self, default: Type) -> Self {
        self.default = Maybe::Some(default);
        self
    }

    /// Mark as meta (compile-time) parameter
    pub fn meta(mut self) -> Self {
        self.is_meta = true;
        self
    }

    /// Set variance
    pub fn with_variance(mut self, variance: Variance) -> Self {
        self.variance = variance;
        self
    }
}

// =============================================================================
// UNIVERSE HIERARCHY TRACKING (Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — )
// =============================================================================

/// Universe variable identifier for universe polymorphism
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UniverseVar(u32);

impl UniverseVar {
    /// Create a fresh universe variable
    pub fn fresh() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        UniverseVar(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the ID of this universe variable
    pub fn id(&self) -> u32 {
        self.0
    }
}

/// Universe constraint for universe level checking
/// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
#[derive(Debug, Clone, PartialEq)]
pub enum UniverseConstraint {
    /// Level u must be less than or equal to level v (u ≤ v)
    LessOrEqual(UniverseLevel, UniverseLevel),
    /// Level u must be strictly less than level v (u < v)
    StrictlyLess(UniverseLevel, UniverseLevel),
    /// Level u must equal level v (u = v)
    Equal(UniverseLevel, UniverseLevel),
    /// Level is the maximum of two levels: w = max(u, v)
    Max(UniverseLevel, UniverseLevel, UniverseLevel),
    /// Level is the successor of another: v = u + 1
    Successor(UniverseLevel, UniverseLevel),
}

impl UniverseConstraint {
    /// Check if this constraint is satisfied given a substitution.
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    ///
    /// Returns:
    /// - `true` if the constraint is definitely satisfied or cannot be falsified
    /// - `false` if the constraint is definitely violated
    ///
    /// For constraints involving unresolved variables, we use structural analysis:
    /// - Variables with equal IDs are considered equal
    /// - Succ(v) > v is always true
    /// - Max constraints propagate through resolved values
    pub fn is_satisfied(&self, subst: &UniverseSubstitution) -> bool {
        match self {
            UniverseConstraint::LessOrEqual(u, v) => {
                let u_resolved = subst.resolve(u);
                let v_resolved = subst.resolve(v);
                Self::check_less_or_equal(&u_resolved, &v_resolved)
            }
            UniverseConstraint::StrictlyLess(u, v) => {
                let u_resolved = subst.resolve(u);
                let v_resolved = subst.resolve(v);
                Self::check_strictly_less(&u_resolved, &v_resolved)
            }
            UniverseConstraint::Equal(u, v) => {
                let u_resolved = subst.resolve(u);
                let v_resolved = subst.resolve(v);
                Self::check_equal(&u_resolved, &v_resolved)
            }
            UniverseConstraint::Max(w, u, v) => {
                let w_resolved = subst.resolve(w);
                let u_resolved = subst.resolve(u);
                let v_resolved = subst.resolve(v);
                Self::check_max(&w_resolved, &u_resolved, &v_resolved)
            }
            UniverseConstraint::Successor(v, u) => {
                let v_resolved = subst.resolve(v);
                let u_resolved = subst.resolve(u);
                Self::check_successor(&v_resolved, &u_resolved)
            }
        }
    }

    /// Check u <= v with structural analysis
    fn check_less_or_equal(u: &UniverseLevel, v: &UniverseLevel) -> bool {
        // Equal levels satisfy <=
        if u == v {
            return true;
        }

        match (u, v) {
            // Concrete comparison
            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => a <= b,

            // v <= Succ(v) is always true
            (UniverseLevel::Variable(v1), UniverseLevel::Succ(v2)) if v1 == v2 => true,

            // Concrete <= Succ (since Succ >= 1)
            (UniverseLevel::Concrete(a), UniverseLevel::Succ(_)) => *a <= 1,

            // Max(a,b) <= c if both a <= c and b <= c
            (UniverseLevel::Max(a, b), UniverseLevel::Concrete(c)) => a <= c && b <= c,

            // Succ(u) <= Succ(v) if u <= v
            (UniverseLevel::Succ(u1), UniverseLevel::Succ(u2)) => u1 <= u2,

            // Same variable is equal
            (UniverseLevel::Variable(v1), UniverseLevel::Variable(v2)) if v1 == v2 => true,

            // For unresolved different variables, assume satisfiable
            _ => true,
        }
    }

    /// Check u < v with structural analysis
    fn check_strictly_less(u: &UniverseLevel, v: &UniverseLevel) -> bool {
        // Equal levels don't satisfy <
        if u == v {
            return false;
        }

        match (u, v) {
            // Concrete comparison
            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => a < b,

            // Concrete 0 is always < Succ(anything)
            (UniverseLevel::Concrete(0), UniverseLevel::Succ(_)) => true,

            // v < Succ(v) is always true
            (UniverseLevel::Variable(v1), UniverseLevel::Succ(v2)) if v1 == v2 => true,

            // Succ(u) < Succ(v) if u < v
            (UniverseLevel::Succ(u1), UniverseLevel::Succ(u2)) => u1 < u2,

            // Same variable is not strictly less
            (UniverseLevel::Variable(v1), UniverseLevel::Variable(v2)) if v1 == v2 => false,

            // For other cases with variables, assume satisfiable
            _ => true,
        }
    }

    /// Check u = v with structural analysis
    fn check_equal(u: &UniverseLevel, v: &UniverseLevel) -> bool {
        // Direct equality
        if u == v {
            return true;
        }

        match (u, v) {
            // Concrete must match exactly
            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => a == b,

            // Same variable is equal
            (UniverseLevel::Variable(v1), UniverseLevel::Variable(v2)) => v1 == v2,

            // Succ(v) = Succ(w) if v = w
            (UniverseLevel::Succ(v1), UniverseLevel::Succ(v2)) => v1 == v2,

            // Max(a,b) = Max(c,d) if sets are equal (order doesn't matter)
            (UniverseLevel::Max(a, b), UniverseLevel::Max(c, d)) => {
                (a == c && b == d) || (a == d && b == c)
            }

            // For mixed cases, cannot determine
            _ => false,
        }
    }

    /// Check w = max(u, v) with structural analysis
    fn check_max(w: &UniverseLevel, u: &UniverseLevel, v: &UniverseLevel) -> bool {
        match (w, u, v) {
            // All concrete: direct computation
            (
                UniverseLevel::Concrete(wc),
                UniverseLevel::Concrete(uc),
                UniverseLevel::Concrete(vc),
            ) => *wc == (*uc).max(*vc),

            // w = max(u, u) = u
            (w, u, v) if u == v => Self::check_equal(w, u),

            // max(u, v) represented as Max(u_var, v_var)
            (UniverseLevel::Max(a, b), UniverseLevel::Variable(u), UniverseLevel::Variable(v)) => {
                (*a == *u && *b == *v) || (*a == *v && *b == *u)
            }

            // Partial concrete cases
            (UniverseLevel::Concrete(wc), UniverseLevel::Concrete(uc), _) => *wc >= *uc,
            (UniverseLevel::Concrete(wc), _, UniverseLevel::Concrete(vc)) => *wc >= *vc,

            // For other cases involving variables, assume satisfiable
            _ => true,
        }
    }

    /// Check v = u + 1 (successor) with structural analysis
    fn check_successor(v: &UniverseLevel, u: &UniverseLevel) -> bool {
        match (v, u) {
            // Concrete successor
            (UniverseLevel::Concrete(vc), UniverseLevel::Concrete(uc)) => *vc == *uc + 1,

            // Succ(u) = u + 1 by definition
            (UniverseLevel::Succ(v_inner), UniverseLevel::Variable(u_var)) => v_inner == u_var,

            // Chain of successors: Succ(a) = Succ(b) + 1 requires a = b + 1
            (UniverseLevel::Succ(a), UniverseLevel::Succ(b)) => *a == *b + 1,

            // For other cases, assume satisfiable
            _ => true,
        }
    }

    /// Get the variables involved in this constraint.
    pub fn variables(&self) -> Vec<u32> {
        match self {
            UniverseConstraint::LessOrEqual(u, v)
            | UniverseConstraint::StrictlyLess(u, v)
            | UniverseConstraint::Equal(u, v)
            | UniverseConstraint::Successor(u, v) => {
                let mut vars = u.variables();
                vars.extend(v.variables());
                vars
            }
            UniverseConstraint::Max(w, u, v) => {
                let mut vars = w.variables();
                vars.extend(u.variables());
                vars.extend(v.variables());
                vars
            }
        }
    }
}

/// Universe substitution maps universe variables to concrete levels
#[derive(Debug, Clone, Default)]
pub struct UniverseSubstitution {
    /// Mapping from universe variable IDs to universe levels
    bindings: Map<u32, UniverseLevel>,
}

impl UniverseSubstitution {
    /// Create a new empty substitution
    pub fn new() -> Self {
        Self {
            bindings: Map::new(),
        }
    }

    /// Insert a binding for a universe variable
    pub fn insert(&mut self, var: u32, level: UniverseLevel) {
        self.bindings.insert(var, level);
    }

    /// Resolve a universe level by applying substitutions
    pub fn resolve(&self, level: &UniverseLevel) -> UniverseLevel {
        match level {
            UniverseLevel::Variable(v) => {
                if let Some(bound_level) = self.bindings.get(v) {
                    // Recursively resolve in case bound_level contains variables
                    self.resolve(bound_level)
                } else {
                    *level
                }
            }
            UniverseLevel::Succ(v) => {
                if let Some(bound_level) = self.bindings.get(v) {
                    self.resolve(bound_level).succ()
                } else {
                    *level
                }
            }
            UniverseLevel::Max(a, b) => {
                let a_resolved = if let Some(a_level) = self.bindings.get(a) {
                    self.resolve(a_level)
                } else {
                    UniverseLevel::Variable(*a)
                };
                let b_resolved = if let Some(b_level) = self.bindings.get(b) {
                    self.resolve(b_level)
                } else {
                    UniverseLevel::Variable(*b)
                };
                a_resolved.max(b_resolved)
            }
            UniverseLevel::Concrete(_) => *level,
        }
    }

    /// Merge another substitution into this one
    pub fn merge(&mut self, other: &UniverseSubstitution) {
        for (var, level) in &other.bindings {
            self.bindings.insert(*var, *level);
        }
    }
}

/// Universe context for tracking universe levels and constraints
/// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
#[derive(Debug, Clone)]
pub struct UniverseContext {
    /// Accumulated universe constraints
    constraints: List<UniverseConstraint>,
    /// Current substitution (solution to constraints)
    substitution: UniverseSubstitution,
    /// Counter for fresh universe variables
    next_var: u32,
}

impl UniverseContext {
    /// Read-only access to the accumulated constraints.
    pub fn constraints(&self) -> &List<UniverseConstraint> {
        &self.constraints
    }

    /// Create a new universe context
    pub fn new() -> Self {
        Self {
            constraints: List::new(),
            substitution: UniverseSubstitution::new(),
            next_var: 0,
        }
    }

    /// Generate a fresh universe variable
    pub fn fresh_universe_var(&mut self) -> UniverseLevel {
        let var = self.next_var;
        self.next_var += 1;
        UniverseLevel::Variable(var)
    }

    /// Add a universe constraint
    pub fn add_constraint(&mut self, constraint: UniverseConstraint) {
        self.constraints.push(constraint);
    }

    /// Add a cumulative constraint: Type_i : Type_{i+1}
    pub fn add_cumulative(&mut self, lower: UniverseLevel, upper: UniverseLevel) {
        self.add_constraint(UniverseConstraint::StrictlyLess(lower, upper));
    }

    /// Constrain that a type at level u forms a type at level max(u1, u2, ...)
    /// For example: (A: Type_u1) -> (B: Type_u2) : Type_max(u1, u2)
    pub fn add_max_constraint(&mut self, result: UniverseLevel, levels: List<UniverseLevel>) {
        if levels.is_empty() {
            return;
        }

        if levels.len() == 1 {
            self.add_constraint(UniverseConstraint::Equal(result, levels[0]));
            return;
        }

        // For multiple levels, create a chain of max constraints
        let mut current = levels[0];
        for level in levels.iter().skip(1) {
            let fresh = self.fresh_universe_var();
            self.add_constraint(UniverseConstraint::Max(fresh, current, *level));
            current = fresh;
        }
        self.add_constraint(UniverseConstraint::Equal(result, current));
    }

    /// Solve universe constraints using iterative constraint propagation.
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe polymorphism
    ///
    /// The algorithm uses a multi-phase approach:
    /// 1. Propagate equality constraints (unification)
    /// 2. Propagate successor constraints (v = u + 1)
    /// 3. Propagate max constraints (w = max(u, v))
    /// 4. Propagate ordering constraints (u < v, u <= v)
    /// 5. Assign concrete levels to remaining variables using lower bounds
    ///
    /// Returns Ok(()) if all constraints are satisfiable, Err otherwise.
    pub fn solve(&mut self) -> Result<(), Text> {
        let max_iterations = 100;
        let mut iteration = 0;

        // Phase 1: Iterative constraint propagation
        loop {
            let mut changed = false;
            iteration += 1;

            if iteration > max_iterations {
                return Err("Universe constraint solving did not converge".into());
            }

            for constraint in &self.constraints.clone() {
                match constraint {
                    // === Equality Constraints ===
                    UniverseConstraint::Equal(u, v) => {
                        let u_resolved = self.substitution.resolve(u);
                        let v_resolved = self.substitution.resolve(v);

                        if u_resolved != v_resolved {
                            match (&u_resolved, &v_resolved) {
                                // Variable = Concrete: bind variable
                                (
                                    UniverseLevel::Variable(var),
                                    concrete @ UniverseLevel::Concrete(_),
                                )
                                | (
                                    concrete @ UniverseLevel::Concrete(_),
                                    UniverseLevel::Variable(var),
                                ) => {
                                    self.substitution.insert(*var, *concrete);
                                    changed = true;
                                }
                                // Concrete = Concrete: must be equal
                                (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => {
                                    if a != b {
                                        return Err(format!(
                                            "Universe constraint violation: Type_{} ≠ Type_{}",
                                            a, b
                                        )
                                        .into());
                                    }
                                }
                                // Variable = Variable: unify (prefer lower ID)
                                (UniverseLevel::Variable(var1), UniverseLevel::Variable(var2)) => {
                                    if var1 != var2 {
                                        let (from, to) = if var1 > var2 {
                                            (*var1, *var2)
                                        } else {
                                            (*var2, *var1)
                                        };
                                        self.substitution.insert(from, UniverseLevel::Variable(to));
                                        changed = true;
                                    }
                                }
                                // Variable = Succ/Max: bind to expression
                                (UniverseLevel::Variable(var), other)
                                | (other, UniverseLevel::Variable(var)) => {
                                    self.substitution.insert(*var, *other);
                                    changed = true;
                                }
                                // Succ = Succ: propagate to inner
                                (UniverseLevel::Succ(v1), UniverseLevel::Succ(v2)) => {
                                    if v1 != v2 {
                                        self.substitution.insert(*v1, UniverseLevel::Variable(*v2));
                                        changed = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // === Strictly Less Constraints ===
                    UniverseConstraint::StrictlyLess(u, v) => {
                        let u_resolved = self.substitution.resolve(u);
                        let v_resolved = self.substitution.resolve(v);

                        match (u_resolved, v_resolved) {
                            // Both concrete: verify
                            (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => {
                                if a >= b {
                                    return Err(format!(
                                        "Universe constraint violation: Type_{} must be < Type_{}",
                                        a, b
                                    )
                                    .into());
                                }
                            }
                            // u concrete, v variable: v >= u + 1
                            (UniverseLevel::Concrete(a), UniverseLevel::Variable(var)) => {
                                if self.substitution.bindings.get(&var).is_none() {
                                    self.substitution
                                        .insert(var, UniverseLevel::Concrete(a + 1));
                                    changed = true;
                                }
                            }
                            // u variable, v concrete: u <= v - 1
                            (UniverseLevel::Variable(var), UniverseLevel::Concrete(b)) => {
                                if b == 0 {
                                    return Err(
                                        "Universe constraint violation: u < Type₀ is impossible"
                                            .into(),
                                    );
                                }
                                if self.substitution.bindings.get(&var).is_none() {
                                    self.substitution.insert(var, UniverseLevel::Concrete(0));
                                    changed = true;
                                }
                            }
                            _ => {}
                        }
                    }

                    // === Less Or Equal Constraints ===
                    UniverseConstraint::LessOrEqual(u, v) => {
                        let u_resolved = self.substitution.resolve(u);
                        let v_resolved = self.substitution.resolve(v);

                        if u_resolved != v_resolved {
                            match (&u_resolved, &v_resolved) {
                                (UniverseLevel::Concrete(a), UniverseLevel::Concrete(b)) => {
                                    if a > b {
                                        return Err(format!(
                                            "Universe constraint violation: Type_{} must be <= Type_{}",
                                            a, b
                                        )
                                        .into());
                                    }
                                }
                                (UniverseLevel::Concrete(a), UniverseLevel::Variable(var)) => {
                                    if self.substitution.bindings.get(var).is_none() {
                                        self.substitution.insert(*var, UniverseLevel::Concrete(*a));
                                        changed = true;
                                    }
                                }
                                (UniverseLevel::Variable(var), UniverseLevel::Concrete(_)) => {
                                    if self.substitution.bindings.get(var).is_none() {
                                        self.substitution.insert(*var, UniverseLevel::Concrete(0));
                                        changed = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // === Successor Constraints ===
                    UniverseConstraint::Successor(v, u) => {
                        let u_resolved = self.substitution.resolve(u);
                        let v_resolved = self.substitution.resolve(v);

                        match (&v_resolved, &u_resolved) {
                            // v variable, u concrete: v = u + 1
                            (UniverseLevel::Variable(var), UniverseLevel::Concrete(uc)) => {
                                self.substitution
                                    .insert(*var, UniverseLevel::Concrete(uc + 1));
                                changed = true;
                            }
                            // v concrete, u variable: u = v - 1
                            (UniverseLevel::Concrete(vc), UniverseLevel::Variable(var)) => {
                                if *vc == 0 {
                                    return Err(
                                        "Universe constraint violation: Succ(u) = 0 is impossible"
                                            .into(),
                                    );
                                }
                                self.substitution
                                    .insert(*var, UniverseLevel::Concrete(vc - 1));
                                changed = true;
                            }
                            // Both concrete: verify
                            (UniverseLevel::Concrete(vc), UniverseLevel::Concrete(uc)) => {
                                if *vc != *uc + 1 {
                                    return Err(format!(
                                        "Universe constraint violation: Type_{} ≠ Type_{} + 1",
                                        vc, uc
                                    )
                                    .into());
                                }
                            }
                            _ => {}
                        }
                    }

                    // === Max Constraints ===
                    UniverseConstraint::Max(w, u, v) => {
                        let w_resolved = self.substitution.resolve(w);
                        let u_resolved = self.substitution.resolve(u);
                        let v_resolved = self.substitution.resolve(v);

                        match (&w_resolved, &u_resolved, &v_resolved) {
                            // w variable, u and v concrete
                            (
                                UniverseLevel::Variable(var),
                                UniverseLevel::Concrete(uc),
                                UniverseLevel::Concrete(vc),
                            ) => {
                                self.substitution
                                    .insert(*var, UniverseLevel::Concrete((*uc).max(*vc)));
                                changed = true;
                            }
                            // All concrete: verify
                            (
                                UniverseLevel::Concrete(wc),
                                UniverseLevel::Concrete(uc),
                                UniverseLevel::Concrete(vc),
                            ) => {
                                if *wc != (*uc).max(*vc) {
                                    return Err(format!(
                                        "Universe constraint violation: max({}, {}) = {}, not {}",
                                        uc,
                                        vc,
                                        (*uc).max(*vc),
                                        wc
                                    )
                                    .into());
                                }
                            }
                            // w and u concrete, v variable: infer v
                            (
                                UniverseLevel::Concrete(wc),
                                UniverseLevel::Concrete(uc),
                                UniverseLevel::Variable(var),
                            ) => {
                                if *uc > *wc {
                                    return Err(format!(
                                        "Universe constraint violation: max({}, v) = {} but {} > {}",
                                        uc, wc, uc, wc
                                    )
                                    .into());
                                } else if *uc < *wc && self.substitution.bindings.get(var).is_none()
                                {
                                    self.substitution.insert(*var, UniverseLevel::Concrete(*wc));
                                    changed = true;
                                }
                            }
                            // w and v concrete, u variable (symmetric)
                            (
                                UniverseLevel::Concrete(wc),
                                UniverseLevel::Variable(var),
                                UniverseLevel::Concrete(vc),
                            ) => {
                                if *vc > *wc {
                                    return Err(format!(
                                        "Universe constraint violation: max(u, {}) = {} but {} > {}",
                                        vc, wc, vc, wc
                                    )
                                    .into());
                                } else if *vc < *wc && self.substitution.bindings.get(var).is_none()
                                {
                                    self.substitution.insert(*var, UniverseLevel::Concrete(*wc));
                                    changed = true;
                                }
                            }
                            // w variable, u and v same: w = u
                            (UniverseLevel::Variable(w_var), _, _) if u_resolved == v_resolved => {
                                self.substitution.insert(*w_var, u_resolved);
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }

        // Phase 2: Assign concrete levels to remaining unconstrained variables
        self.assign_remaining_variables();

        // Phase 3: Verify all constraints are satisfied
        for constraint in &self.constraints {
            if !constraint.is_satisfied(&self.substitution) {
                return Err(format!("Universe constraint not satisfied: {:?}", constraint).into());
            }
        }

        Ok(())
    }

    /// Assign concrete levels to remaining unconstrained variables.
    /// Uses the minimum valid value (typically 0) for unconstrained variables.
    fn assign_remaining_variables(&mut self) {
        // Collect all variable IDs mentioned in constraints
        let mut all_vars = std::collections::HashSet::new();
        for constraint in &self.constraints {
            for var in constraint.variables() {
                all_vars.insert(var);
            }
        }

        // Add local variables from next_var counter
        for var in 0..self.next_var {
            all_vars.insert(var);
        }

        // Assign concrete level to any unbound variable
        for var in all_vars {
            let level = UniverseLevel::Variable(var);
            let resolved = self.substitution.resolve(&level);

            match resolved {
                UniverseLevel::Variable(_) => {
                    // Unbound variable: assign 0
                    self.substitution.insert(var, UniverseLevel::Concrete(0));
                }
                UniverseLevel::Succ(inner_var) => {
                    // Succ of unbound: assign inner to 0
                    if self.substitution.bindings.get(&inner_var).is_none() {
                        self.substitution
                            .insert(inner_var, UniverseLevel::Concrete(0));
                    }
                }
                UniverseLevel::Max(a, b) => {
                    // Max of unbound: assign both to 0
                    if self.substitution.bindings.get(&a).is_none() {
                        self.substitution.insert(a, UniverseLevel::Concrete(0));
                    }
                    if self.substitution.bindings.get(&b).is_none() {
                        self.substitution.insert(b, UniverseLevel::Concrete(0));
                    }
                }
                _ => {}
            }
        }
    }

    /// Get the resolved level for a universe level
    pub fn resolve(&self, level: &UniverseLevel) -> UniverseLevel {
        self.substitution.resolve(level)
    }

    /// Get the universe level of a type
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    pub fn universe_of(&mut self, ty: &Type) -> Result<UniverseLevel, Text> {
        match ty {
            // Type₀ : Type₁
            Type::Universe { level } => Ok(level.succ()),

            // Prop : Type₀
            Type::Prop => Ok(UniverseLevel::TYPE),

            // Primitives are in Type₀
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text => {
                Ok(UniverseLevel::TYPE)
            }

            // Named types - lookup their definition
            Type::Named { .. } => Ok(UniverseLevel::TYPE), // Default assumption

            // Function types: (x: A) -> B : Type_max(level(A), level(B))
            Type::Function {
                params,
                return_type,
                ..
            } => {
                let mut levels = List::new();
                for param in params {
                    levels.push(self.universe_of(param)?);
                }
                levels.push(self.universe_of(return_type)?);

                let result = self.fresh_universe_var();
                self.add_max_constraint(result, levels);
                Ok(result)
            }

            // Tuple types: (A, B) : Type_max(level(A), level(B))
            Type::Tuple(types) => {
                let mut levels = List::new();
                for ty in types {
                    levels.push(self.universe_of(ty)?);
                }

                if levels.is_empty() {
                    Ok(UniverseLevel::TYPE)
                } else {
                    let result = self.fresh_universe_var();
                    self.add_max_constraint(result, levels);
                    Ok(result)
                }
            }

            // Reference types inherit the level of the referent
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => self.universe_of(inner),

            // Refinement types inherit the level of the base type
            Type::Refined { base, .. } => self.universe_of(base),

            // Dependent pairs: Sigma<A, B> : Type_max(level(A), level(B(x)))
            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                let fst_level = self.universe_of(fst_type)?;
                let snd_level = self.universe_of(snd_type)?;
                let result = self.fresh_universe_var();
                self.add_max_constraint(result, List::from(vec![fst_level, snd_level]));
                Ok(result)
            }

            // Type variables - generate fresh universe variable
            Type::Var(_) => Ok(self.fresh_universe_var()),

            // Other types default to Type₀
            _ => Ok(UniverseLevel::TYPE),
        }
    }

    /// Check that a type can be used at a given universe level
    pub fn check_universe(&mut self, ty: &Type, expected_level: UniverseLevel) -> Result<(), Text> {
        let actual_level = self.universe_of(ty)?;
        self.add_constraint(UniverseConstraint::LessOrEqual(
            actual_level,
            expected_level,
        ));
        Ok(())
    }

    /// Get the current substitution (solved variable bindings).
    pub fn substitution(&self) -> &UniverseSubstitution {
        &self.substitution
    }
}

impl Default for UniverseContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Type environment maps variables to type schemes.
///
/// This supports lexical scoping through a chain of environments.
/// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Module awareness for type resolution
///
/// PERF NOTE: Uses Box for parent. The child() method still clones, but
/// push_scope/pop_scope provide zero-copy scope management for the common
/// case of lexical scoping. Use push_scope/pop_scope when possible.
#[derive(Debug, Clone)]
pub struct TypeEnv {
    /// Current scope bindings
    bindings: IndexMap<Text, TypeScheme>,
    /// Parent environment (for nested scopes)
    parent: Option<Box<TypeEnv>>,
    /// Type parameters in scope with their bounds
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    type_params: Map<Text, TypeParam>,
    /// Current module context (Maybe for backward compatibility)
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Track current module during type checking
    current_module: Maybe<ModuleId>,
    /// Universe context for dependent type checking
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe hierarchy tracking
    universe_ctx: UniverseContext,
}

impl TypeEnv {
    /// Create a new empty environment
    pub fn new() -> Self {
        Self {
            bindings: IndexMap::new(),
            parent: None,
            type_params: Map::new(),
            current_module: Maybe::None,
            universe_ctx: UniverseContext::new(),
        }
    }

    /// Create a child environment (nested scope)
    ///
    /// NOTE: This clones the current environment. For better performance,
    /// use push_scope/pop_scope which avoids cloning.
    pub fn child(&self) -> Self {
        Self {
            bindings: IndexMap::new(),
            parent: Some(Box::new(self.clone())),
            type_params: Map::new(),
            current_module: self.current_module, // Inherit module context
            universe_ctx: self.universe_ctx.clone(), // Inherit universe context
        }
    }

    /// Push a new scope (mutates the current environment to have a new child scope)
    ///
    /// PERF: This is the efficient way to create nested scopes - no cloning!
    /// The current environment is moved into parent, not cloned.
    /// Inherits current_module and universe_ctx like child() does.
    pub fn push_scope(&mut self) {
        let current_module = self.current_module;
        let universe_ctx = self.universe_ctx.clone();
        let old = std::mem::take(self);
        self.parent = Some(Box::new(old));
        self.current_module = current_module;
        self.universe_ctx = universe_ctx;
    }

    /// Pop the current scope and restore the parent
    ///
    /// PERF: Zero-copy restoration of parent scope.
    pub fn pop_scope(&mut self) {
        if let Some(parent) = self.parent.take() {
            *self = *parent;
        }
    }

    /// Set the current module context
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn set_current_module(&mut self, module_id: ModuleId) {
        self.current_module = Maybe::Some(module_id);
    }

    /// Get the current module context
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn current_module(&self) -> Maybe<ModuleId> {
        self.current_module
    }

    /// Insert a binding in the current scope
    pub fn insert(&mut self, name: impl Into<Text>, scheme: TypeScheme) {
        let name_text: Text = name.into();
        self.bindings.insert(name_text, scheme);
    }

    /// Insert a monomorphic type
    pub fn insert_mono(&mut self, name: impl Into<Text>, ty: Type) {
        self.insert(name, TypeScheme::mono(ty));
    }

    /// Remove a binding from the current scope (only).
    ///
    /// Does NOT reach into parent scopes — caller discipline: only use
    /// this to evict a binding owned by the current scope (e.g. a stdlib
    /// variant-constructor that a user type declaration should shadow).
    /// Returns `true` if a binding with `name` was present and removed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.bindings.shift_remove(&Text::from(name)).is_some()
    }

    /// Look up a variable, searching parent scopes if needed
    pub fn lookup(&self, name: &str) -> Option<&TypeScheme> {
        self.bindings
            .get(&Text::from(name))
            .or_else(|| self.parent.as_ref().and_then(|p| p.lookup(name)))
    }

    /// Collect all names visible in this scope (current + parents).
    /// Used by error messages to compute "did you mean?" suggestions.
    /// Deduplicates inner-shadowing outer (inner scope wins, matching
    /// regular lookup semantics).
    pub fn visible_names(&self) -> Vec<Text> {
        let mut seen = indexmap::IndexSet::new();
        let mut scope = Some(self);
        while let Some(e) = scope {
            for k in e.bindings.keys() {
                seen.insert(k.clone());
            }
            scope = e.parent.as_deref();
        }
        seen.into_iter().collect()
    }

    /// Get all free type variables in the environment
    pub fn free_vars(&self) -> Set<TypeVar> {
        let mut vars = Set::new();
        for scheme in self.bindings.values() {
            for v in scheme.free_vars() {
                vars.insert(v);
            }
        }
        if let Some(parent) = &self.parent {
            for v in parent.free_vars() {
                vars.insert(v);
            }
        }
        vars
    }

    /// Generalize a type relative to this environment
    ///
    /// This creates a type scheme by quantifying over free variables
    /// that don't appear in the environment (let-polymorphism).
    pub fn generalize(&self, ty: Type) -> TypeScheme {
        let env_vars = self.free_vars();
        let ty_vars = ty.free_vars();

        // Only quantify over vars not in environment
        let mut quantified = List::new();
        for v in ty_vars {
            if !env_vars.contains(&v) {
                quantified.push(v);
            }
        }

        if quantified.is_empty() {
            TypeScheme::mono(ty)
        } else {
            TypeScheme::poly(quantified, ty)
        }
    }

    /// Generalize a type into a type scheme, tracking which vars are implicit.
    /// Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
    ///
    /// This enables functions with implicit parameters: `fn id<{T}>(x: T) -> T`
    pub fn generalize_with_implicit(
        &self,
        ty: Type,
        implicit_var_names: &Set<TypeVar>,
    ) -> TypeScheme {
        let env_vars = self.free_vars();
        let ty_vars = ty.free_vars();

        // Only quantify over vars not in environment
        let mut quantified = List::new();
        let mut implicit_quantified = Set::new();

        for v in ty_vars {
            if !env_vars.contains(&v) {
                quantified.push(v);
                // Track if this var was marked as implicit
                if implicit_var_names.contains(&v) {
                    implicit_quantified.insert(v);
                }
            }
        }

        if quantified.is_empty() {
            TypeScheme::mono(ty)
        } else {
            TypeScheme::poly_with_implicit(quantified, ty, implicit_quantified)
        }
    }

    /// Get the number of bindings in current scope
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Check if the current scope is empty
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    // ==================== Type Parameter Management ====================

    /// Add type parameter to environment
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    pub fn add_type_param(&mut self, param: TypeParam) {
        self.type_params.insert(param.name.clone(), param);
    }

    /// Get type parameter by name
    pub fn get_type_param(&self, name: &str) -> Maybe<&TypeParam> {
        // Check current scope first
        if let Some(param) = self.type_params.get(&name.into()) {
            return Maybe::Some(param);
        }

        // Then check parent scopes
        if let Some(parent) = &self.parent {
            return parent.get_type_param(name);
        }

        Maybe::None
    }

    /// Check if type satisfies parameter bounds
    /// Generic bounds checking: verifying type arguments satisfy protocol constraints at instantiation sites
    ///
    /// Performs actual bounds checking using the provided ProtocolChecker.
    /// Returns the bounds that need to be satisfied if a protocol checker is not provided.
    pub fn check_param_bounds(
        &self,
        ty: &Type,
        param_name: &str,
        protocol_checker: Option<&crate::protocol::ProtocolChecker>,
    ) -> Result<(), Text> {
        if let Maybe::Some(param) = self.get_type_param(param_name)
            && !param.bounds.is_empty()
        {
            // If a protocol checker is provided, verify bounds
            if let Some(checker) = protocol_checker {
                // Convert bounds to slice for check_bounds
                let bounds_vec: std::vec::Vec<_> = param.bounds.iter().cloned().collect();
                if let Err(protocol_err) = checker.check_bounds(ty, &bounds_vec) {
                    return Err(Text::from(format!(
                        "Type '{}' does not satisfy bounds for parameter '{}': {}",
                        ty, param_name, protocol_err
                    )));
                }
            }
            // Without protocol checker, we rely on caller to perform checking
        }
        Ok(())
    }

    /// Get the bounds for a type parameter (for external checking)
    /// Generic bounds checking: verifying type arguments satisfy protocol constraints at instantiation sites
    ///
    /// Returns the protocol bounds for a type parameter, or None if not found.
    pub fn get_param_bounds(&self, param_name: &str) -> Maybe<List<ProtocolBound>> {
        self.get_type_param(param_name)
            .map(|param| param.bounds.clone())
    }

    /// Get all type parameters in scope (including parent scopes)
    pub fn all_type_params(&self) -> List<TypeParam> {
        let mut params = List::new();

        // Collect from parent first (so they can be overridden)
        if let Some(parent) = &self.parent {
            params.extend(parent.all_type_params());
        }

        // Add current scope params
        for param in self.type_params.values() {
            // Remove duplicates by name
            params.retain(|p: &TypeParam| p.name != param.name);
            params.push(param.clone());
        }

        params
    }

    // ==================== Universe Hierarchy Tracking ====================
    // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter

    /// Get a mutable reference to the universe context
    pub fn universe_ctx_mut(&mut self) -> &mut UniverseContext {
        &mut self.universe_ctx
    }

    /// Get an immutable reference to the universe context
    pub fn universe_ctx(&self) -> &UniverseContext {
        &self.universe_ctx
    }

    /// Generate a fresh universe variable
    pub fn fresh_universe_var(&mut self) -> UniverseLevel {
        self.universe_ctx.fresh_universe_var()
    }

    /// Add a universe constraint
    pub fn add_universe_constraint(&mut self, constraint: UniverseConstraint) {
        self.universe_ctx.add_constraint(constraint);
    }

    /// Get the universe level of a type
    pub fn universe_of(&mut self, ty: &Type) -> Result<UniverseLevel, Text> {
        self.universe_ctx.universe_of(ty)
    }

    /// Check that a type can be used at a given universe level
    pub fn check_universe(&mut self, ty: &Type, expected_level: UniverseLevel) -> Result<(), Text> {
        self.universe_ctx.check_universe(ty, expected_level)
    }

    /// Solve accumulated universe constraints
    pub fn solve_universe_constraints(&mut self) -> Result<(), Text> {
        self.universe_ctx.solve()
    }

    /// Resolve a universe level with current substitutions
    pub fn resolve_universe(&self, level: &UniverseLevel) -> UniverseLevel {
        self.universe_ctx.resolve(level)
    }

    /// Infer universe level for a let-binding generalization
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe polymorphism for let-bindings
    pub fn infer_universe_for_binding(&mut self, ty: &Type) -> Result<UniverseLevel, Text> {
        // Compute the universe level of the type
        let level = self.universe_of(ty)?;

        // For let-polymorphism, we generalize over universe variables
        // that don't appear in the environment
        let resolved = self.resolve_universe(&level);

        Ok(resolved)
    }

    /// Check universe cumulativity: Type_i : Type_{i+1}
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Cumulative universe hierarchy
    pub fn check_cumulative(
        &mut self,
        lower: UniverseLevel,
        upper: UniverseLevel,
    ) -> Result<(), Text> {
        self.universe_ctx.add_cumulative(lower, upper);
        Ok(())
    }
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Type context for the entire type checker.
///
/// This maintains global state including:
/// - Current type environment
/// - Protocol implementations
/// - Named types
/// - Type aliases
/// - Context permissions (for Level 2 dynamic contexts)
/// - Module-qualified type definitions (Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — )
/// - Inductive type constructors (Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1)
#[derive(Debug, Clone)]
pub struct TypeContext {
    /// Current typing environment
    pub env: TypeEnv,
    /// Named type definitions (unqualified for backward compatibility)
    pub type_defs: Map<Text, Type>,
    /// Type aliases map: alias_name -> target_type
    /// Used for resolving `type Int is i64;` style aliases
    /// When looking up a type, we follow alias chains until we reach a non-alias type
    pub type_aliases: Map<Text, Type>,
    /// Module-qualified type definitions
    /// Maps (module_id, type_name) -> Type
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Qualified type lookup
    pub module_type_defs: Map<(ModuleId, Text), Type>,
    /// Module-qualified type aliases
    /// Maps (module_id, alias_name) -> target_type
    pub module_type_aliases: Map<(ModuleId, Text), Type>,
    /// Protocol implementations (type -> protocol -> impl)
    pub protocol_impls: Map<Text, Map<Text, ProtocolImpl>>,
    /// Allowed contexts in current scope
    pub allowed_contexts: Set<Text>,
    /// Inductive type constructors for exhaustiveness checking
    /// Maps type name -> list of constructors
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Inductive Types
    pub inductive_constructors: Map<Text, List<crate::ty::InductiveConstructor>>,
    /// Protocol bounds on type variables
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - GAT where clause constraints
    ///
    /// Maps TypeVar -> List<ProtocolBound> for constraint checking during
    /// GAT instantiation and generic function type checking.
    /// Example: `fn sort<T: Ord>(list: List<T>) -> List<T>` binds T -> [Ord]
    pub type_var_bounds: Map<TypeVar, List<ProtocolBound>>,

    /// Audit-A4: meta-parameter / const-generic environment.
    ///
    /// Maps a meta-param name (e.g. `N` in `fn foo<N: meta usize>()`)
    /// to its current binding. Populated during generic-param
    /// processing in `infer.rs` at the `GenericParamKind::Meta` and
    /// `GenericParamKind::Const` arms. Consulted during refinement
    /// substitution so a predicate like `where len(arr) == N`
    /// rewrites `N` to its concrete value at instantiation time
    /// (rather than translating to an unbound Z3 variable that the
    /// solver couldn't satisfy).
    ///
    /// `Bound(value)` means a concrete instantiation has been seen
    /// (e.g. `foo::<5>()` sets `N -> Bound(MetaValue::Int(5))`).
    /// `Symbolic` means the meta-param is in scope but unbound — its
    /// refinement bounds are tracked symbolically through the SMT
    /// solver. Removed from the map when the generic-scope exits.
    pub meta_param_environment: Map<Text, MetaParamBinding>,
}

/// Audit-A4: binding of a meta / const-generic parameter.
///
/// The environment in `TypeContext::meta_param_environment` carries
/// these so refinement-predicate substitution can either inline a
/// concrete value (`Bound`) or preserve the symbolic reference
/// (`Symbolic`) for SMT to constrain.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaParamBinding {
    /// Concrete instantiation — used to inline the value into
    /// refinement predicates and to short-circuit SMT translation
    /// for paths that reference this meta-param.
    Bound(verum_ast::MetaValue),
    /// Meta-param is declared but not yet bound to a concrete value.
    /// Refinement predicates referencing it are passed through to
    /// SMT verbatim so the solver can reason about the bound.
    Symbolic,
}

impl TypeContext {
    /// Create a new type context with language primitives only.
    ///
    /// This is the STDLIB-AGNOSTIC constructor that includes only:
    /// - Language primitives (Bool, Unit)
    /// - Compiler intrinsics (transmute, alloc, free, etc.)
    /// - CBGR type aliases (RawPtr, Epoch)
    ///
    /// **STDLIB TYPES ARE NOT INCLUDED** - they must be loaded from:
    /// - core/*.vr source files (via pipeline.load_stdlib_modules())
    /// - Pre-compiled VBC archives (via CoreMetadata)
    ///
    /// Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files
    pub fn new() -> Self {
        let mut ctx = Self::new_minimal();

        // Add only language primitives (NOT stdlib types)
        ctx.add_primitive_constructors();
        // Add CBGR type aliases (RawPtr, u32, Epoch)
        ctx.add_cbgr_type_aliases();
        // Add protocol implementations for primitive types (Int, Text, Bool, Float)
        ctx.add_primitive_protocol_impls();
        ctx
    }

    /// Create a minimal type context without any types.
    ///
    /// Used for stdlib bootstrap where types are registered dynamically
    /// as stdlib .vr files are parsed.
    ///
    /// Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files
    pub fn new_minimal() -> Self {
        Self {
            env: TypeEnv::new(),
            type_defs: Map::new(),
            type_aliases: Map::new(),
            module_type_defs: Map::new(),
            module_type_aliases: Map::new(),
            protocol_impls: Map::new(),
            allowed_contexts: Set::new(),
            type_var_bounds: Map::new(),
            meta_param_environment: Map::new(),
            inductive_constructors: Map::new(),
        }
    }

    /// Register constructors for an inductive type
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Inductive Types
    pub fn register_inductive_type(
        &mut self,
        type_name: impl Into<Text>,
        constructors: List<crate::ty::InductiveConstructor>,
    ) {
        self.inductive_constructors
            .insert(type_name.into(), constructors);
    }

    /// Get constructors for an inductive type (for exhaustiveness checking)
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Dependent Pattern Matching
    pub fn get_constructors(
        &self,
        type_name: &Text,
    ) -> Maybe<&List<crate::ty::InductiveConstructor>> {
        self.inductive_constructors.get(type_name)
    }

    /// Add ONLY language primitive type constructors.
    ///
    /// STDLIB-AGNOSTIC: This method adds ONLY true language primitives:
    /// - Bool (true | false)
    /// - Unit (())
    ///
    /// All other types (Maybe, Result, List, Map, Set, Heap, Shared)
    /// MUST come from stdlib source files, NOT hardcoded here.
    fn add_primitive_constructors(&mut self) {
        use crate::ty::{InductiveConstructor, Type};

        // Bool constructors: true | false
        // Bool is a language primitive with special compiler support
        let bool_constructors = List::from_iter([
            InductiveConstructor::unit("true".into(), Type::Bool),
            InductiveConstructor::unit("false".into(), Type::Bool),
        ]);
        self.register_inductive_type(WKT::Bool.as_str(), bool_constructors);

        // Unit constructor: ()
        // Unit is a language primitive representing the absence of a value
        let unit_constructors =
            List::from_iter([InductiveConstructor::unit("()".into(), Type::Unit)]);
        self.register_inductive_type("Unit", unit_constructors);
    }

    /// Add low-level type aliases for CBGR memory safety system.
    ///
    /// STDLIB-AGNOSTIC: These are language-level type aliases, NOT stdlib types.
    /// They define raw pointer and epoch types used by the CBGR memory model.
    ///
    /// NOTE: Intrinsic FUNCTIONS (transmute, alloc, free, etc.) are registered
    /// in TypeChecker::register_primitives() to keep all function signatures
    /// in one place. This function only defines TYPE ALIASES.
    ///
    /// Type aliases:
    /// - RawPtr: raw mutable pointer (void* equivalent)
    /// - u32: 32-bit unsigned integer (for CBGR generation counters)
    /// - Epoch: epoch counter type for CBGR tracking
    pub(crate) fn add_cbgr_type_aliases(&mut self) {
        use crate::ty::Type;

        // RawPtr is an alias for raw mutable pointer (void* equivalent)
        // CBGR implementation: epoch-based generation tracking, acquire-release memory ordering, lock-free ABA-protected maps, ThinRef 16 bytes, FatRef 24 bytes — #raw-pointers
        self.define_alias(
            "RawPtr",
            Type::Pointer {
                mutable: true,
                inner: Box::new(Type::Int),
            },
        );

        // u32 type alias: use UInt32 (4 bytes) rather than Int (8 bytes)
        // to match the type system's from_le_bytes/to_le_bytes byte counts.
        // CBGR generation counters that need the full range should use Int directly.
        self.define_alias("u32", Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new("UInt32", verum_ast::span::Span::dummy())),
            args: verum_common::List::new(),
        });

        // Epoch type for CBGR epoch tracking
        self.define_alias("Epoch", Type::Int);
    }

    /// Add protocol implementations for language primitive types.
    ///
    /// IMPORTANT: These are NOT stdlib types - Int, Text, Bool, Float are
    /// language primitives with built-in protocol implementations.
    ///
    /// Protocols:
    /// - Eq: equality comparison (==, !=)
    /// - Ord: ordering comparison (<, <=, >, >=)
    /// - Add: addition (+)
    /// - Sub: subtraction (-)
    /// - Mul: multiplication (*)
    /// - Div: division (/)
    ///
    /// These are part of the language specification, not stdlib.
    fn add_primitive_protocol_impls(&mut self) {
        // Int implements all numeric protocols
        self.impl_protocol(WKT::Int.as_str(), "Eq");
        self.impl_protocol(WKT::Int.as_str(), "Ord");
        self.impl_protocol(WKT::Int.as_str(), "Add");
        self.impl_protocol(WKT::Int.as_str(), "Sub");
        self.impl_protocol(WKT::Int.as_str(), "Mul");
        self.impl_protocol(WKT::Int.as_str(), "Div");

        // Float implements all numeric protocols
        self.impl_protocol(WKT::Float.as_str(), "Eq");
        self.impl_protocol(WKT::Float.as_str(), "Ord");
        self.impl_protocol(WKT::Float.as_str(), "Add");
        self.impl_protocol(WKT::Float.as_str(), "Sub");
        self.impl_protocol(WKT::Float.as_str(), "Mul");
        self.impl_protocol(WKT::Float.as_str(), "Div");

        // Text implements Eq (equality comparison)
        // Note: Text does NOT implement Add - concatenation uses ++ operator
        self.impl_protocol(WKT::Text.as_str(), "Eq");
        self.impl_protocol(WKT::Text.as_str(), "Ord");

        // Bool implements Eq
        self.impl_protocol(WKT::Bool.as_str(), "Eq");

        // Unit implements Eq (trivially - () == () is always true)
        self.impl_protocol("Unit", "Eq");
    }

    /// Register a stdlib type dynamically from parsed source.
    ///
    /// STDLIB-AGNOSTIC: This method is used by the compilation pipeline
    /// to register types discovered in core/*.vr source files.
    ///
    /// Example: When parsing core/base/maybe.vr, the compiler calls:
    /// `ctx.register_stdlib_type("Maybe", maybe_type, maybe_constructors)`
    pub fn register_stdlib_type(
        &mut self,
        type_name: impl Into<Text>,
        type_def: crate::ty::Type,
        constructors: List<crate::ty::InductiveConstructor>,
    ) {
        let name = type_name.into();
        self.define_type(name.as_str(), type_def);
        self.register_inductive_type(name, constructors);
    }

    /// Register a generic stdlib type with type parameters.
    ///
    /// STDLIB-AGNOSTIC: For generic types like Maybe<T>, Result<T,E>, List<T>.
    pub fn register_generic_stdlib_type(
        &mut self,
        type_name: impl Into<Text>,
        type_def: crate::ty::Type,
        constructors: List<crate::ty::InductiveConstructor>,
        type_params: indexmap::IndexMap<Text, crate::ty::Type>,
    ) {
        let name = type_name.into();
        self.define_type(name.as_str(), type_def);
        self.register_inductive_type(name.clone(), constructors);
        // Register type params for substitution
        let params_key = format!("__type_params_{}", name);
        self.define_type(params_key.as_str(), crate::ty::Type::Record(type_params));
    }

    /// Set the current module context
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn set_current_module(&mut self, module_id: ModuleId) {
        self.env.set_current_module(module_id);
    }

    /// Get the current module context
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
    pub fn current_module(&self) -> Maybe<ModuleId> {
        self.env.current_module()
    }

    /// Add a protocol to the registry
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Complete Protocol System
    /// Protocol coherence: ensuring unique implementations across the program, orphan rules, overlap detection — .1 - Coherence Rules
    ///
    /// This registers a protocol in the type context, enabling:
    /// - Type checking of protocol implementations
    /// - Protocol bound verification on generic types
    /// - VTable generation for dynamic dispatch
    ///
    /// The protocol registry tracks:
    /// - Protocol name -> set of implementing types
    /// - Each implementing type maps to its implementation details
    fn add_protocol(&mut self, name: &str) {
        let protocol_name = Text::from(name);
        if self.protocol_impls.get(&protocol_name).is_none() {
            // Initialize the protocol with an empty implementation map
            // Implementations are added via impl_protocol() calls
            self.protocol_impls.insert(protocol_name, Map::new());
        }
    }

    fn impl_protocol(&mut self, ty: &str, protocol: &str) {
        let protocol_key = Text::from(protocol);
        if self.protocol_impls.get(&protocol_key).is_none() {
            self.protocol_impls.insert(protocol_key.clone(), Map::new());
        }

        if let Some(proto_map) = self.protocol_impls.get_mut(&protocol_key) {
            proto_map.insert(
                Text::from(ty),
                ProtocolImpl {
                    protocol: Text::from(protocol),
                    for_type: Text::from(ty),
                },
            );
        }
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self) {
        self.env.push_scope();
    }

    /// Exit the current scope
    pub fn exit_scope(&mut self) {
        self.env.pop_scope();
    }

    /// Add a type definition (unqualified, for backward compatibility)
    pub fn define_type(&mut self, name: impl Into<Text>, ty: Type) {
        let name = name.into();
        self.type_defs.insert(name, ty);
    }

    /// Remove a type definition (for cleanup of temporary type parameters)
    pub fn remove_type(&mut self, name: &Text) {
        self.type_defs.remove(name);
    }

    /// Iterate over all type definitions.
    ///
    /// Returns an iterator over (name, type) pairs. Used by two-pass type
    /// resolution to verify that no placeholder types remain after resolution.
    pub fn all_types(&self) -> impl Iterator<Item = (&Text, &Type)> {
        self.type_defs.iter()
    }

    /// Define a type in a specific module
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Module-qualified type definitions
    pub fn define_module_type(&mut self, module_id: ModuleId, name: impl Into<Text>, ty: Type) {
        let name = name.into();
        // Store in both qualified and unqualified maps for compatibility
        self.module_type_defs
            .insert((module_id, name.clone()), ty.clone());
        // Also store unqualified if in current module
        if let Maybe::Some(current) = self.current_module()
            && current == module_id
        {
            self.type_defs.insert(name, ty);
        }
    }

    /// Look up a type definition (unqualified)
    /// Follows alias chains automatically
    pub fn lookup_type(&self, name: &str) -> Option<&Type> {
        // First check if it's a direct type definition
        if let Some(ty) = self.type_defs.get(&Text::from(name)) {
            return Option::Some(ty);
        }

        // Then check if it's an alias and resolve it
        self.resolve_alias(name)
    }

    /// Look up a type definition, preferring the module-qualified version.
    /// Tries `module_path.name` first, then falls back to unqualified `name`.
    pub fn lookup_type_in_module(&self, module_path: &str, name: &str) -> Option<&Type> {
        if !module_path.is_empty() && module_path != "cog" {
            let qualified = Text::from(format!("{}.{}", module_path, name));
            if let Some(ty) = self.type_defs.get(&qualified) {
                return Option::Some(ty);
            }
        }
        self.lookup_type(name)
    }

    /// Generalize a type with ordered type parameter names
    ///
    /// This creates a type scheme by quantifying over type variables in the specified order.
    /// Uses lookup_type to find type params which are stored in type_defs (not env.bindings).
    ///
    /// This is critical for correct type argument binding in generic method calls:
    /// - For `implement<L, R> Either<L, R> { fn map_left<L2>(...) }` with receiver `Either<Int, Text>`
    /// - We need vars = [L, R, L2] so that Int binds to L and Text binds to R
    /// - Using unordered free_vars() could produce [L2, R, L] causing wrong bindings
    ///
    /// CRITICAL: For methods, ALL impl type params MUST be included in the scheme even if
    /// they don't appear in the function type (because self was excluded). This ensures
    /// proper alignment: receiver_type_args[i] binds to scheme.vars[i].
    /// Example: `implement<T> Maybe<T> { fn and<U>(self, other: Maybe<U>) -> Maybe<U> }`
    /// - method_ty = fn(Maybe<U>) -> Maybe<U> (self excluded, so T not in free_vars)
    /// - ordered_params = ["T", "U"]
    /// - scheme.vars MUST be [T, U] for correct binding, not just [U]
    pub fn generalize_ordered(&self, ty: Type, ordered_param_names: &[Text]) -> TypeScheme {
        use crate::ty::TypeVar;

        let ty_vars = ty.free_vars();

        // Build ordered list of type vars by looking up each name in type_defs
        // CRITICAL: Include ALL ordered params, even if they don't appear in free_vars.
        // This maintains positional alignment for method call binding where
        // receiver_type_args[i] must bind to scheme.vars[i].
        //
        // BEWARE: this variant resolves TypeVars by *name*. If the
        // calling context has shadowed one of `ordered_param_names`
        // with a later `define_type` (e.g. method-level params
        // shadowing impl-level params with the same spelling), the
        // shadowed var is picked up instead of the intended impl-level
        // one and positional binding breaks. Prefer
        // `generalize_with_vars` below when both impl-level and
        // method-level params are in play.
        let mut quantified = List::new();
        for name in ordered_param_names {
            if let Option::Some(param_ty) = self.lookup_type(name.as_str()) {
                if let Type::Var(v) = param_ty {
                    // Include ALL type params from ordered list, not just those in free_vars
                    // This ensures correct alignment during method instantiation
                    quantified.push(*v);
                }
            }
        }

        // Add any remaining free vars not in the ordered list (should be rare)
        for v in ty_vars.iter() {
            if !quantified.contains(v) {
                quantified.push(*v);
            }
        }

        if quantified.is_empty() {
            TypeScheme::mono(ty)
        } else {
            TypeScheme::poly(quantified, ty)
        }
    }

    /// Generalize with an explicitly-ordered list of TypeVars — no
    /// name lookup, no possibility of shadow collision. Use this
    /// when impl-level and method-level type parameters might share
    /// a name (e.g. `impl<T, F: fn()->T> …` and
    /// `fn map<B, F: fn(Self.Item)->B>`): the caller already holds
    /// the impl and method TypeVars separately, so pass them in the
    /// correct order directly.
    ///
    /// `ordered_vars` should list impl-level TypeVars first, in
    /// declaration order, followed by method-level TypeVars in
    /// declaration order. Any additional free_vars not already in
    /// the list are appended, matching `generalize_ordered`'s
    /// fallback behaviour.
    pub fn generalize_with_vars(
        &self,
        ty: Type,
        ordered_vars: &[crate::ty::TypeVar],
    ) -> TypeScheme {
        let ty_vars = ty.free_vars();

        let mut quantified = List::new();
        for v in ordered_vars {
            // Preserve positional alignment: duplicates (same name at
            // both impl and method level but we've been handed both
            // TypeVars) are kept distinct.
            quantified.push(*v);
        }

        for v in ty_vars.iter() {
            if !quantified.contains(v) {
                quantified.push(*v);
            }
        }

        if quantified.is_empty() {
            TypeScheme::mono(ty)
        } else {
            TypeScheme::poly(quantified, ty)
        }
    }

    /// Register a type alias: `type Alias is Target;`
    pub fn define_alias(&mut self, alias_name: impl Into<Text>, target_type: Type) {
        let alias_name = alias_name.into();
        self.type_aliases.insert(alias_name, target_type);
    }

    /// Remove a type alias (used when user types override stdlib aliases)
    pub fn remove_alias(&mut self, alias_name: &str) {
        let key: Text = alias_name.into();
        self.type_aliases.remove(&key);
    }

    /// Register a module-qualified type alias
    pub fn define_module_alias(
        &mut self,
        module_id: ModuleId,
        alias_name: impl Into<Text>,
        target_type: Type,
    ) {
        let alias_name = alias_name.into();
        self.module_type_aliases
            .insert((module_id, alias_name.clone()), target_type.clone());

        // Also store unqualified if in current module
        if let Maybe::Some(current) = self.current_module()
            && current == module_id
        {
            self.type_aliases.insert(alias_name, target_type);
        }
    }

    /// Resolve a type alias to its target type
    /// Follows alias chains: if `type A is B; type B is C;`, resolves A -> C
    /// Returns None if name is not an alias
    pub fn resolve_alias(&self, name: &str) -> Option<&Type> {
        let mut current_name = Text::from(name);
        let mut visited = std::collections::HashSet::new();
        const MAX_ALIAS_DEPTH: usize = 100;

        // Follow alias chain until we find a non-alias type
        for _ in 0..MAX_ALIAS_DEPTH {
            // Check for cycles
            if visited.contains(&current_name) {
                // Cyclic alias detected - return None
                return Option::None;
            }
            visited.insert(current_name.clone());

            // Look up the current name in aliases
            if let Some(target_type) = self.type_aliases.get(&current_name) {
                // Found an alias - check if the target is also an alias
                match target_type {
                    Type::Named { path, args } if args.is_empty() => {
                        // Target is a simple named type - might be another alias
                        if let Some(ident) = path.as_ident() {
                            current_name = Text::from(ident.name.as_str());
                            continue;
                        } else {
                            // Complex path - not a simple alias chain
                            return Option::Some(target_type);
                        }
                    }
                    _ => {
                        // Target is not a named type - end of alias chain
                        return Option::Some(target_type);
                    }
                }
            } else {
                // Not an alias - check if it's a regular type definition
                return self.type_defs.get(&current_name);
            }
        }

        // Max depth exceeded - likely infinite loop
        Option::None
    }

    /// Resolve a module-qualified alias
    pub fn resolve_module_alias(
        &self,
        module_id: ModuleId,
        name: &str,
    ) -> Option<&Type> {
        let key = (module_id, Text::from(name));
        if let Some(target_type) = self.module_type_aliases.get(&key) {
            return Option::Some(target_type);
        }
        Option::None
    }

    /// Look up a type definition in a specific module
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Qualified type lookup (Module.Type)
    pub fn lookup_module_type(
        &self,
        module_id: ModuleId,
        name: &str,
    ) -> Option<&Type> {
        self.module_type_defs
            .get(&(module_id, Text::from(name)))
    }

    /// Look up a qualified type by path (e.g., "Module.Type" or "module::type")
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Path-based type resolution
    ///
    /// Supports both dot (.) and double-colon (::) separators for qualified paths.
    /// Resolution strategy:
    /// 1. Parse the path into module segments and type name
    /// 2. Try to resolve the module from available module_type_defs
    /// 3. Look up the type in that module
    /// 4. Fall back to unqualified lookup if module resolution fails
    pub fn lookup_qualified_type(&self, path: &str) -> Option<&Type> {
        // Normalize separators: support both "." and "::"
        let normalized = path.replace("::", ".");

        // Parse qualified path: split by dots
        let parts: Vec<&str> = normalized.split('.').collect();

        if parts.len() < 2 {
            // No qualification - just a simple name
            return self.lookup_type(path);
        }

        // Last part is the type name, everything before is the module path
        let type_name = parts[parts.len() - 1];

        // Try to find a module that matches any suffix of the module path
        // This handles cases where we have "std.collections.Map" but only "collections" is registered
        for start_idx in 0..(parts.len() - 1) {
            // Build module path from parts[start_idx..parts.len()-1]
            let module_parts: Vec<&str> = parts[start_idx..parts.len() - 1].to_vec();
            let module_path = module_parts.join(".");

            // Search for a matching module in module_type_defs
            for ((module_id, name), ty) in &self.module_type_defs {
                if name.as_str() == type_name {
                    // Check if this module matches our module path
                    // For now, we do a simple check - in a full implementation,
                    // this would use the ModuleRegistry to resolve module paths to IDs

                    // Try looking up the module by path to see if IDs match
                    if let Option::Some(current_mod) = self.current_module()
                        && *module_id == current_mod
                    {
                        // Found it in current module
                        return Option::Some(ty);
                    }

                    // Return the type if we found any match
                    // In a full implementation with registry access, we'd verify
                    // the module path actually resolves to this module_id
                    return Option::Some(ty);
                }
            }
        }

        // Fall back to unqualified lookup
        // This maintains backward compatibility and handles cases where
        // the module registry isn't available or the path couldn't be resolved
        self.lookup_type(type_name)
    }

    /// Get the module ID that defines a type
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Reverse module lookup
    ///
    /// This allows determining which module a type belongs to,
    /// useful for visibility checking and qualified name generation.
    pub fn get_type_module(&self, type_name: &str) -> Option<ModuleId> {
        // Search through module_type_defs for the type
        for ((module_id, name), _ty) in &self.module_type_defs {
            if name.as_str() == type_name {
                return Option::Some(*module_id);
            }
        }
        Option::None
    }

    /// Look up a type with visibility checking
    /// Visibility and access control: private (default), public, cog-public, module-scoped
    ///
    /// Returns the type only if it's accessible from the current module.
    /// Private types are only accessible within their defining module.
    pub fn lookup_type_visible(
        &self,
        module_id: ModuleId,
        name: &str,
        from_module: ModuleId,
    ) -> Option<&Type> {
        // First check if the type exists
        if let Option::Some(ty) = self.lookup_module_type(module_id, name) {
            // For now, assume all types are public
            // Future: integrate with verum_modules::VisibilityChecker
            // to check if `from_module` can access types in `module_id`

            // Same module - always visible
            if module_id == from_module {
                return Option::Some(ty);
            }

            // Different module - check visibility
            // For now, we follow standard visibility rules:
            // - Public items are always visible
            // - Private items are only visible in the same module
            // Future: Integrate with full visibility checker with AST metadata

            // Assume types are public by default (conservative approach)
            // Private types would need to be marked explicitly in metadata
            Option::Some(ty)
        } else {
            Option::None
        }
    }

    /// Add a protocol bound to a type variable
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - GAT where clause constraints
    ///
    /// This method tracks protocol bounds on type variables for later verification.
    /// Used primarily during GAT instantiation to apply where clause constraints.
    ///
    /// # Protocol Bound Tracking
    ///
    /// When processing generic declarations like:
    /// ```verum
    /// fn sort<T: Ord + Clone>(list: List<T>) -> List<T>
    /// ```
    ///
    /// This method is called twice:
    /// 1. `add_protocol_bound(T_var, Ord)`
    /// 2. `add_protocol_bound(T_var, Clone)`
    ///
    /// The bounds are accumulated and later verified during type checking when
    /// the generic is instantiated with concrete types.
    ///
    /// # GAT Integration
    ///
    /// For GAT where clauses:
    /// ```verum
    /// protocol Container {
    ///     type Item<T> where T: Clone + Debug
    /// }
    /// ```
    ///
    /// The bounds are tracked per-GAT instantiation to ensure constraints are
    /// satisfied when the associated type is resolved.
    pub fn add_protocol_bound(&mut self, var: TypeVar, bound: crate::protocol::ProtocolBound) {
        if let Some(bounds) = self.type_var_bounds.get_mut(&var) {
            // Add to existing bounds list
            bounds.push(bound);
        } else {
            // Create new bounds list for this type variable
            self.type_var_bounds.insert(var, List::from_iter([bound]));
        }
    }

    /// Get all protocol bounds for a type variable
    /// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4 - GAT where clause constraints
    ///
    /// Returns the list of protocol bounds that must be satisfied when the
    /// type variable is instantiated with a concrete type.
    pub fn get_protocol_bounds(&self, var: &TypeVar) -> Maybe<&List<ProtocolBound>> {
        self.type_var_bounds.get(var)
    }

    /// Check if a type variable has a specific protocol bound
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Complete Protocol System
    ///
    /// Returns true if the type variable is bounded by the given protocol.
    pub fn has_protocol_bound(&self, var: &TypeVar, protocol: &Text) -> bool {
        if let Some(bounds) = self.type_var_bounds.get(var) {
            bounds.iter().any(|b| {
                // Compare protocol path to the given protocol name
                // First try as_ident for simple single-segment paths
                if let Some(ident) = b.protocol.as_ident() {
                    return ident.name.as_str() == protocol.as_str();
                }
                // For multi-segment paths, check the last segment
                match b.protocol.segments.last() {
                    Some(verum_ast::ty::PathSegment::Name(ident)) => {
                        ident.name.as_str() == protocol.as_str()
                    }
                    _ => false,
                }
            })
        } else {
            false
        }
    }

    /// Remove all protocol bounds for a type variable
    /// Used when exiting a scope where the type variable was defined
    pub fn clear_protocol_bounds(&mut self, var: &TypeVar) {
        self.type_var_bounds.remove(var);
    }

    /// Check if a type implements a protocol
    pub fn implements_protocol(&self, ty: &str, protocol: &str) -> bool {
        if let Some(impls) = self.protocol_impls.get(&Text::from(protocol)) {
            impls.get(&Text::from(ty)).is_some()
        } else {
            false
        }
    }

    /// Add a context permission
    pub fn allow_context(&mut self, context: impl Into<Text>) {
        self.allowed_contexts.insert(context.into());
    }

    /// Check if a context is allowed
    pub fn is_context_allowed(&self, context: &str) -> bool {
        self.allowed_contexts.contains(&Text::from(context))
    }

    // allow_effect() and is_effect_allowed() removed in v6.0-BALANCED
    // Use allow_context() and is_context_allowed() instead

    /// Generate accessor functions for record/struct types
    /// Cross-field refinements on structs: "type T is { f1: A, f2: B } where constraint(f1, f2)" — .2.1 lines 1839-1857
    ///
    /// For each field in a record, generates a function:
    /// `fn field_name(self: RecordType) -> FieldType { self.field_name }`
    ///
    /// These accessors are used in inline refinement syntax:
    /// `type ValidUser is User{age(it) >= 18 && email(it).contains("@")}`
    pub fn generate_accessors(&mut self, type_name: &str, ty: &Type) -> Result<(), Text> {
        match ty {
            Type::Record(fields) => {
                for (field_name, field_ty) in fields {
                    // Create accessor function type:
                    // fn field_name(self: RecordType) -> FieldType
                    let accessor_name = format!("{}.{}", type_name, field_name);

                    // The self parameter has the record type
                    let self_ty = Type::Named {
                        path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                            type_name,
                            verum_ast::span::Span::dummy(),
                        )),
                        args: List::new(),
                    };

                    let accessor_ty = Type::function(List::from(vec![self_ty]), field_ty.clone());

                    // Register the accessor as a monomorphic function
                    self.env.insert_mono(accessor_name, accessor_ty);
                }
                Ok(())
            }
            Type::Named { path: _, args: _ } => {
                // Look up the named type and generate accessors for it
                if let Option::Some(resolved_ty) =
                    self.lookup_type(type_name).cloned()
                {
                    self.generate_accessors(type_name, &resolved_ty)
                } else {
                    Ok(()) // Not a record type, nothing to generate
                }
            }
            _ => Ok(()), // Not a record type, nothing to generate
        }
    }

    /// Register a type definition and generate accessors
    /// Cross-field refinements on structs: "type T is { f1: A, f2: B } where constraint(f1, f2)" — .2.1 lines 1839-1857
    pub fn define_type_with_accessors(
        &mut self,
        name: impl Into<Text>,
        ty: Type,
    ) -> Result<(), Text> {
        let name = name.into();
        // Generate accessors before defining the type
        // (so the type is available for accessor signatures)
        self.generate_accessors(name.as_str(), &ty)?;

        // Define the type
        self.define_type(name, ty);

        Ok(())
    }

    // ==================== Universe Hierarchy Tracking ====================
    // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter

    /// Get a mutable reference to the universe context
    pub fn universe_ctx_mut(&mut self) -> &mut UniverseContext {
        self.env.universe_ctx_mut()
    }

    /// Get an immutable reference to the universe context
    pub fn universe_ctx(&self) -> &UniverseContext {
        self.env.universe_ctx()
    }

    /// Generate a fresh universe variable
    pub fn fresh_universe_var(&mut self) -> UniverseLevel {
        self.env.fresh_universe_var()
    }

    /// Add a universe constraint
    pub fn add_universe_constraint(&mut self, constraint: UniverseConstraint) {
        self.env.add_universe_constraint(constraint);
    }

    /// Get the universe level of a type
    pub fn universe_of(&mut self, ty: &Type) -> Result<UniverseLevel, Text> {
        self.env.universe_of(ty)
    }

    /// Check that a type can be used at a given universe level
    pub fn check_universe(&mut self, ty: &Type, expected_level: UniverseLevel) -> Result<(), Text> {
        self.env.check_universe(ty, expected_level)
    }

    /// Solve accumulated universe constraints
    pub fn solve_universe_constraints(&mut self) -> Result<(), Text> {
        self.env.solve_universe_constraints()
    }

    /// Resolve a universe level with current substitutions
    pub fn resolve_universe(&self, level: &UniverseLevel) -> UniverseLevel {
        self.env.resolve_universe(level)
    }

    /// Infer universe level for a let-binding generalization
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe polymorphism for let-bindings
    pub fn infer_universe_for_binding(&mut self, ty: &Type) -> Result<UniverseLevel, Text> {
        self.env.infer_universe_for_binding(ty)
    }

    /// Check universe cumulativity: Type_i : Type_{i+1}
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Cumulative universe hierarchy
    pub fn check_cumulative(
        &mut self,
        lower: UniverseLevel,
        upper: UniverseLevel,
    ) -> Result<(), Text> {
        self.env.check_cumulative(lower, upper)
    }

    /// Type check a universe type (Type_i)
    /// Ensures that Type_i : Type_{i+1}
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    pub fn check_universe_type(&mut self, level: UniverseLevel) -> Result<UniverseLevel, Text> {
        // Type_i has type Type_{i+1}
        let result = level.succ();

        // Add cumulative constraint
        self.add_universe_constraint(UniverseConstraint::StrictlyLess(level, result));

        Ok(result)
    }

    /// Check function type formation with universe polymorphism
    /// (x: A) -> B : Type_max(level(A), level(B))
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
    pub fn check_function_universe(
        &mut self,
        param_types: &List<Type>,
        return_type: &Type,
    ) -> Result<UniverseLevel, Text> {
        let mut levels = List::new();

        // Get universe levels of all parameters
        for param_ty in param_types {
            levels.push(self.universe_of(param_ty)?);
        }

        // Get universe level of return type
        levels.push(self.universe_of(return_type)?);

        // Result is max of all levels
        let result = self.fresh_universe_var();
        self.env
            .universe_ctx_mut()
            .add_max_constraint(result, levels);

        Ok(result)
    }

    /// Check dependent pair (Sigma type) formation
    /// (x: A, B(x)) : Type_max(level(A), level(B))
    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
    pub fn check_sigma_universe(
        &mut self,
        base_type: &Type,
        dependent_type: &Type,
    ) -> Result<UniverseLevel, Text> {
        let base_level = self.universe_of(base_type)?;
        let dep_level = self.universe_of(dependent_type)?;

        let result = self.fresh_universe_var();
        self.env
            .universe_ctx_mut()
            .add_max_constraint(result, List::from(vec![base_level, dep_level]));

        Ok(result)
    }
}

impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A protocol implementation record
#[derive(Debug, Clone)]
pub struct ProtocolImpl {
    pub protocol: Text,
    pub for_type: Text,
}

// =============================================================================
// DEFINITE ASSIGNMENT ANALYSIS
// Spec: L0-critical/memory-safety/uninitialized - Partial initialization detection
// =============================================================================

/// Initialization state for definite assignment analysis.
///
/// Tracks whether a variable is fully initialized, partially initialized,
/// or completely uninitialized. This enables detection of partial initialization
/// errors at compile time.
///
/// # Example States
///
/// ```verum
/// let x: Int;                  // x: Uninitialized
/// let tuple: (Int, Int, Int);  // tuple: Uninitialized
/// tuple.0 = 1;                 // tuple: PartiallyInitialized(Tuple { [0] })
/// tuple.1 = 2;                 // tuple: PartiallyInitialized(Tuple { [0, 1] })
/// tuple.2 = 3;                 // tuple: FullyInitialized
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum InitState {
    /// Variable is completely uninitialized - no elements/fields have been assigned
    Uninitialized,
    /// Variable is partially initialized - some but not all elements/fields assigned
    PartiallyInitialized(PartialInit),
    /// Variable is fully initialized - all elements/fields assigned (or initialized via literal)
    FullyInitialized,
}

impl InitState {
    /// Check if this variable is fully initialized
    pub fn is_fully_initialized(&self) -> bool {
        matches!(self, InitState::FullyInitialized)
    }

    /// Check if this variable is completely uninitialized
    pub fn is_uninitialized(&self) -> bool {
        matches!(self, InitState::Uninitialized)
    }

    /// Check if a specific field is initialized (for records)
    pub fn is_field_initialized(&self, field: &Text) -> bool {
        match self {
            InitState::FullyInitialized => true,
            InitState::Uninitialized => false,
            InitState::PartiallyInitialized(partial) => match partial {
                PartialInit::Record { initialized, .. } => initialized.contains(field),
                _ => false,
            },
        }
    }

    /// Check if a specific index is initialized (for tuples/arrays)
    pub fn is_index_initialized(&self, index: usize) -> bool {
        match self {
            InitState::FullyInitialized => true,
            InitState::Uninitialized => false,
            InitState::PartiallyInitialized(partial) => match partial {
                PartialInit::Tuple { initialized, .. }
                | PartialInit::Array { initialized, .. } => initialized.contains(&index),
                PartialInit::Record { .. } => false,
            },
        }
    }

    /// Get missing fields for error reporting
    pub fn missing_fields(&self) -> List<Text> {
        match self {
            InitState::Uninitialized => List::new(), // All missing but we don't know which
            InitState::FullyInitialized => List::new(),
            InitState::PartiallyInitialized(partial) => match partial {
                PartialInit::Record {
                    initialized,
                    required,
                } => required.difference(initialized).cloned().collect(),
                _ => List::new(),
            },
        }
    }

    /// Get missing indices for error reporting
    pub fn missing_indices(&self) -> List<usize> {
        match self {
            InitState::Uninitialized => List::new(), // All missing but we don't know which
            InitState::FullyInitialized => List::new(),
            InitState::PartiallyInitialized(partial) => match partial {
                PartialInit::Tuple { initialized, total } => {
                    (0..*total).filter(|i| !initialized.contains(i)).collect()
                }
                PartialInit::Array { initialized, len } => {
                    (0..*len).filter(|i| !initialized.contains(i)).collect()
                }
                _ => List::new(),
            },
        }
    }
}

/// Details of partial initialization for compound types.
#[derive(Debug, Clone, PartialEq)]
pub enum PartialInit {
    /// Tuple with some elements initialized
    ///
    /// Example: `tuple.0 = 1; tuple.1 = 2;` with total=3 gives initialized={0, 1}
    Tuple {
        /// Indices that have been initialized
        initialized: Set<usize>,
        /// Total number of elements in the tuple
        total: usize,
    },
    /// Array with some elements initialized
    ///
    /// Example: `arr[0] = 10; arr[1] = 20;` with len=5 gives initialized={0, 1}
    Array {
        /// Indices that have been initialized (only tracks statically known indices)
        initialized: Set<usize>,
        /// Total length of the array
        len: usize,
    },
    /// Record/struct with some fields initialized
    ///
    /// Example: `person.name = "Alice";` gives initialized={"name"}, required={"name", "age", "email"}
    Record {
        /// Fields that have been initialized
        initialized: Set<Text>,
        /// All required fields
        required: Set<Text>,
    },
}

impl PartialInit {
    /// Create partial init state for a tuple with given size
    pub fn tuple(total: usize) -> Self {
        PartialInit::Tuple {
            initialized: Set::new(),
            total,
        }
    }

    /// Create partial init state for an array with given length
    pub fn array(len: usize) -> Self {
        PartialInit::Array {
            initialized: Set::new(),
            len,
        }
    }

    /// Create partial init state for a record with given required fields
    pub fn record(required: Set<Text>) -> Self {
        PartialInit::Record {
            initialized: Set::new(),
            required,
        }
    }

    /// Mark a tuple/array index as initialized
    pub fn initialize_index(&mut self, index: usize) {
        match self {
            PartialInit::Tuple { initialized, .. } | PartialInit::Array { initialized, .. } => {
                initialized.insert(index);
            }
            PartialInit::Record { .. } => {}
        }
    }

    /// Mark a record field as initialized
    pub fn initialize_field(&mut self, field: Text) {
        if let PartialInit::Record { initialized, .. } = self {
            initialized.insert(field);
        }
    }

    /// Check if all elements/fields are now initialized
    pub fn is_complete(&self) -> bool {
        match self {
            PartialInit::Tuple { initialized, total } => initialized.len() == *total,
            PartialInit::Array { initialized, len } => initialized.len() == *len,
            PartialInit::Record {
                initialized,
                required,
            } => required.is_subset(initialized),
        }
    }
}

/// Initialization tracker for definite assignment analysis.
///
/// Tracks the initialization state of variables in the current scope.
/// Used during type checking to detect use of uninitialized or
/// partially initialized variables.
#[derive(Debug, Clone, Default)]
pub struct InitTracker {
    /// Maps variable names to their initialization state
    states: Map<Text, InitState>,
    /// Parent tracker (for nested scopes)
    parent: Maybe<Box<InitTracker>>,
}

impl InitTracker {
    /// Create a new initialization tracker
    pub fn new() -> Self {
        Self {
            states: Map::new(),
            parent: Maybe::None,
        }
    }

    /// Create a child tracker for a nested scope
    pub fn child(&self) -> Self {
        Self {
            states: Map::new(),
            parent: Maybe::Some(Box::new(self.clone())),
        }
    }

    /// Push a new scope (for entering blocks)
    pub fn push_scope(&mut self) {
        let old = std::mem::take(self);
        self.parent = Maybe::Some(Box::new(old));
    }

    /// Pop the current scope (for exiting blocks)
    pub fn pop_scope(&mut self) {
        if let Maybe::Some(parent) = self.parent.take() {
            *self = *parent;
        }
    }

    /// Register a new variable as uninitialized
    pub fn register_uninitialized(&mut self, name: impl Into<Text>) {
        self.states.insert(name.into(), InitState::Uninitialized);
    }

    /// Register a new variable as fully initialized (e.g., from literal or full assignment)
    pub fn register_initialized(&mut self, name: impl Into<Text>) {
        self.states.insert(name.into(), InitState::FullyInitialized);
    }

    /// Register a variable with partial initialization tracking for a tuple
    pub fn register_tuple(&mut self, name: impl Into<Text>, size: usize) {
        self.states.insert(
            name.into(),
            InitState::PartiallyInitialized(PartialInit::tuple(size)),
        );
    }

    /// Register a variable with partial initialization tracking for an array
    pub fn register_array(&mut self, name: impl Into<Text>, len: usize) {
        self.states.insert(
            name.into(),
            InitState::PartiallyInitialized(PartialInit::array(len)),
        );
    }

    /// Register a variable with partial initialization tracking for a record
    pub fn register_record(&mut self, name: impl Into<Text>, required_fields: Set<Text>) {
        self.states.insert(
            name.into(),
            InitState::PartiallyInitialized(PartialInit::record(required_fields)),
        );
    }

    /// Mark a field of a variable as initialized
    pub fn initialize_field(&mut self, var_name: &Text, field: Text) {
        if let Some(state) = self.states.get_mut(var_name) {
            match state {
                InitState::Uninitialized => {
                    // This shouldn't happen for records, but handle defensively
                }
                InitState::PartiallyInitialized(partial) => {
                    partial.initialize_field(field);
                    if partial.is_complete() {
                        *state = InitState::FullyInitialized;
                    }
                }
                InitState::FullyInitialized => {
                    // Already fully initialized, no-op
                }
            }
        } else if let Maybe::Some(ref mut parent) = self.parent {
            parent.initialize_field(var_name, field);
        }
    }

    /// Mark an index of a variable as initialized
    pub fn initialize_index(&mut self, var_name: &Text, index: usize) {
        if let Some(state) = self.states.get_mut(var_name) {
            match state {
                InitState::Uninitialized => {
                    // This shouldn't happen for tuples/arrays that need tracking
                }
                InitState::PartiallyInitialized(partial) => {
                    partial.initialize_index(index);
                    if partial.is_complete() {
                        *state = InitState::FullyInitialized;
                    }
                }
                InitState::FullyInitialized => {
                    // Already fully initialized, no-op
                }
            }
        } else if let Maybe::Some(ref mut parent) = self.parent {
            parent.initialize_index(var_name, index);
        }
    }

    /// Mark a variable as fully initialized (from whole-value assignment)
    pub fn mark_fully_initialized(&mut self, var_name: &Text) {
        if self.states.contains_key(var_name) {
            self.states
                .insert(var_name.clone(), InitState::FullyInitialized);
        } else if let Maybe::Some(ref mut parent) = self.parent {
            parent.mark_fully_initialized(var_name);
        }
    }

    /// Get the initialization state of a variable
    pub fn get_state(&self, var_name: &Text) -> Maybe<&InitState> {
        if let Some(state) = self.states.get(var_name) {
            Maybe::Some(state)
        } else if let Maybe::Some(ref parent) = self.parent {
            parent.get_state(var_name)
        } else {
            Maybe::None
        }
    }

    /// Check if a variable is fully initialized
    pub fn is_fully_initialized(&self, var_name: &Text) -> bool {
        match self.get_state(var_name) {
            Maybe::Some(state) => state.is_fully_initialized(),
            Maybe::None => true, // If not tracked, assume initialized (for non-tracked vars)
        }
    }

    /// Check if a specific field is initialized
    pub fn is_field_initialized(&self, var_name: &Text, field: &Text) -> bool {
        match self.get_state(var_name) {
            Maybe::Some(state) => state.is_field_initialized(field),
            Maybe::None => true, // If not tracked, assume initialized
        }
    }

    /// Check if a specific index is initialized
    pub fn is_index_initialized(&self, var_name: &Text, index: usize) -> bool {
        match self.get_state(var_name) {
            Maybe::Some(state) => state.is_index_initialized(index),
            Maybe::None => true, // If not tracked, assume initialized
        }
    }
}

// Tests moved to tests/ directory
