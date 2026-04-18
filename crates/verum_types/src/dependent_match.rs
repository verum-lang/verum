//! Dependent Pattern Matching
//!
//! Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Dependent Pattern Matching
//!
//! This module implements dependent pattern matching where:
//! 1. The type of each branch can depend on the constructor matched
//! 2. Pattern matching refines type indices based on constructor arguments
//! 3. Motives describe how the result type varies with the matched value
//! 4. Absurd patterns handle impossible cases (empty types)
//!
//! # Key Concepts
//!
//! ## Motive
//! A motive is a type function that describes how the result type depends
//! on the scrutinee value. For example, when matching on `Vec n T`:
//! - In the `nil` branch, we know `n = 0`
//! - In the `cons` branch, we know `n = m+1` for some `m`
//!
//! ## Constructor Unification
//! When matching a constructor, we unify the scrutinee type with the
//! constructor's return type, which refines type indices.
//!
//! ## Branch Type Refinement
//! Each branch is type-checked with refined knowledge about the scrutinee
//! type based on the matched constructor.

use crate::context::TypeEnv;
use crate::ty::{EqConst, EqTerm, InductiveConstructor, ProjComponent, Type, TypeVar};
use crate::unify::Unifier;
use crate::{TypeError, TypeScheme};
use indexmap::IndexMap;
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Set, Text};

/// A motive for dependent pattern matching.
///
/// The motive describes how the result type depends on the scrutinee value.
/// It's a type-level function: for a scrutinee of type T, it returns a type.
///
/// Example:
/// ```verum
/// // For vector_head : (n: Nat, Vec (n+1) T) -> T
/// // Motive: λ(v: Vec m T). T
/// // The result type T doesn't depend on the specific value, but we need
/// // the motive to ensure type soundness in each branch.
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Motive {
    /// Parameter name for the scrutinee (e.g., "v" for the vector)
    pub param: Text,
    /// Parameter type (the scrutinee type with possible indices)
    pub param_ty: Type,
    /// Result type that may depend on the parameter
    pub result_ty: Type,
}

impl Motive {
    /// Create a simple motive where result type doesn't depend on value
    pub fn simple(param: Text, param_ty: Type, result_ty: Type) -> Self {
        Motive {
            param,
            param_ty,
            result_ty,
        }
    }

    /// Apply the motive to a specific value (for type checking branches)
    pub fn apply(&self, value: &EqTerm) -> Type {
        // Substitute the value into the result type
        // This performs the type-level computation: motive(value)
        self.substitute_term_in_type(&self.result_ty, &self.param, value)
    }

    /// Apply the motive to a type (for constructor refinement)
    pub fn apply_type(&self, ty: &Type) -> Type {
        // For type-level application, we substitute types directly
        // This is used when refining based on constructor patterns
        self.substitute_type_in_type(&self.result_ty, &self.param, ty)
    }

    /// Substitute a term in a type (for dependent type computation)
    /// Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — .2 - Type-Level Computation
    ///
    /// This performs capture-avoiding substitution of a term into a type,
    /// which is essential for dependent type checking. When we apply a
    /// motive to a value, we need to replace occurrences of the bound
    /// variable with the actual term.
    ///
    /// # Examples
    /// - Motive: λn. List<T, n>
    /// - Term: succ(m)
    /// - Result: List<T, succ(m)>
    fn substitute_term_in_type(&self, ty: &Type, var: &Text, term: &EqTerm) -> Type {
        match ty {
            // Generic type - check if name matches the variable
            Type::Generic { name, args } => {
                // If the generic name matches our variable, we need to convert
                // the term to a type (for type-level computation)
                if name.as_str() == var.as_str() {
                    self.term_to_type(term)
                } else {
                    Type::Generic {
                        name: name.clone(),
                        args: args
                            .iter()
                            .map(|a| self.substitute_term_in_type(a, var, term))
                            .collect(),
                    }
                }
            }
            // Named type - substitute in type arguments
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_term_in_type(a, var, term))
                    .collect(),
            },
            // Function type - substitute in params and return type
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.substitute_term_in_type(p, var, term))
                    .collect(),
                return_type: Box::new(self.substitute_term_in_type(return_type, var, term)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            // Tuple type
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.substitute_term_in_type(t, var, term))
                    .collect(),
            ),
            // Array type - substitute in element type
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_term_in_type(element, var, term)),
                size: *size,
            },
            // Reference types
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.substitute_term_in_type(inner, var, term)),
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(self.substitute_term_in_type(inner, var, term)),
            },
            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(self.substitute_term_in_type(inner, var, term)),
            },
            // Pi type - capture-avoiding substitution
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
                // Don't substitute under the binder if it shadows our variable
                if param_name.as_str() == var.as_str() {
                    ty.clone()
                } else {
                    Type::Pi {
                        param_name: param_name.clone(),
                        param_type: Box::new(self.substitute_term_in_type(param_type, var, term)),
                        return_type: Box::new(self.substitute_term_in_type(return_type, var, term)),
                    }
                }
            }
            // Sigma type - capture-avoiding substitution
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                // Don't substitute under the binder if it shadows our variable
                if fst_name.as_str() == var.as_str() {
                    ty.clone()
                } else {
                    Type::Sigma {
                        fst_name: fst_name.clone(),
                        fst_type: Box::new(self.substitute_term_in_type(fst_type, var, term)),
                        snd_type: Box::new(self.substitute_term_in_type(snd_type, var, term)),
                    }
                }
            }
            // Equality type - substitute in all components
            Type::Eq {
                ty: eq_ty,
                lhs,
                rhs,
            } => Type::Eq {
                ty: Box::new(self.substitute_term_in_type(eq_ty, var, term)),
                lhs: Box::new(self.substitute_term_in_eq_term(lhs, var, term)),
                rhs: Box::new(self.substitute_term_in_eq_term(rhs, var, term)),
            },
            // Meta type (compile-time parameter)
            Type::Meta {
                name: meta_name,
                ty: meta_ty,
                refinement,
                value,
            } => {
                if meta_name.as_str() == var.as_str() {
                    self.term_to_type(term)
                } else {
                    Type::Meta {
                        name: meta_name.clone(),
                        ty: Box::new(self.substitute_term_in_type(meta_ty, var, term)),
                        refinement: refinement.clone(),
                        value: value.clone(),
                    }
                }
            }
            // Refined type - substitute in base type
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(self.substitute_term_in_type(base, var, term)),
                predicate: predicate.clone(), // Predicate substitution handled separately
            },
            // All other types remain unchanged
            _ => ty.clone(),
        }
    }

    /// Substitute a term within an EqTerm
    fn substitute_term_in_eq_term(
        &self,
        eq_term: &EqTerm,
        var: &Text,
        replacement: &EqTerm,
    ) -> EqTerm {
        match eq_term {
            EqTerm::Var(name) => {
                if name.as_str() == var.as_str() {
                    replacement.clone()
                } else {
                    eq_term.clone()
                }
            }
            EqTerm::Const(_) => eq_term.clone(),
            EqTerm::App { func, args } => EqTerm::App {
                func: Box::new(self.substitute_term_in_eq_term(func, var, replacement)),
                args: args
                    .iter()
                    .map(|a| self.substitute_term_in_eq_term(a, var, replacement))
                    .collect(),
            },
            EqTerm::Lambda { param, body } => {
                // Don't substitute under the binder if it shadows our variable
                if param.as_str() == var.as_str() {
                    eq_term.clone()
                } else {
                    EqTerm::Lambda {
                        param: param.clone(),
                        body: Box::new(self.substitute_term_in_eq_term(body, var, replacement)),
                    }
                }
            }
            EqTerm::Proj { pair, component } => EqTerm::Proj {
                pair: Box::new(self.substitute_term_in_eq_term(pair, var, replacement)),
                component: *component,
            },
            EqTerm::Refl(inner) => EqTerm::Refl(Box::new(self.substitute_term_in_eq_term(
                inner,
                var,
                replacement,
            ))),
            EqTerm::J {
                proof,
                motive,
                base,
            } => EqTerm::J {
                proof: Box::new(self.substitute_term_in_eq_term(proof, var, replacement)),
                motive: Box::new(self.substitute_term_in_eq_term(motive, var, replacement)),
                base: Box::new(self.substitute_term_in_eq_term(base, var, replacement)),
            },
        }
    }

    /// Convert an EqTerm to a Type for type-level computation
    /// This is used when substituting a value into a type position
    fn term_to_type(&self, term: &EqTerm) -> Type {
        match term {
            EqTerm::Var(name) => Type::Generic {
                name: name.clone(),
                args: List::new(),
            },
            EqTerm::Const(c) => match c {
                EqConst::Int(n) => {
                    // For type-level integers, we create a meta type
                    Type::Meta {
                        name: Text::from(format!("{}", n)),
                        ty: Box::new(Type::Int),
                        refinement: None,
                        value: Some(verum_common::ConstValue::Int(*n as i128)),
                    }
                }
                EqConst::Nat(n) => Type::Meta {
                    name: Text::from(format!("{}", n)),
                    ty: Box::new(Type::Generic {
                        name: Text::from("Nat"),
                        args: List::new(),
                    }),
                    refinement: None,
                    value: Some(verum_common::ConstValue::UInt(*n as u128)),
                },
                EqConst::Bool(b) => Type::Meta {
                    name: Text::from(if *b { "true" } else { "false" }),
                    ty: Box::new(Type::Bool),
                    refinement: None,
                    value: Some(verum_common::ConstValue::Bool(*b)),
                },
                EqConst::Unit => Type::Unit,
                EqConst::Named(name) => Type::Generic {
                    name: name.clone(),
                    args: List::new(),
                },
            },
            EqTerm::App { func, args } => {
                // Application at type level becomes type application
                let func_ty = self.term_to_type(func);
                let arg_tys: List<Type> = args.iter().map(|a| self.term_to_type(a)).collect();
                match func_ty {
                    Type::Generic {
                        name,
                        args: existing_args,
                    } => Type::Generic {
                        name,
                        args: existing_args.into_iter().chain(arg_tys).collect(),
                    },
                    _ => func_ty,
                }
            }
            EqTerm::Lambda { param, body } => {
                // Lambda at type level becomes a Pi type
                let body_ty = self.term_to_type(body);
                Type::Pi {
                    param_name: param.clone(),
                    param_type: Box::new(Type::Generic {
                        name: Text::from("_"),
                        args: List::new(),
                    }),
                    return_type: Box::new(body_ty),
                }
            }
            EqTerm::Proj { pair, component } => {
                // Projection at type level - extract component
                let pair_ty = self.term_to_type(pair);
                if let Type::Sigma {
                    fst_name: _,
                    fst_type,
                    snd_type,
                } = pair_ty
                {
                    match component {
                        ProjComponent::Fst => *fst_type,
                        ProjComponent::Snd => *snd_type,
                    }
                } else {
                    pair_ty
                }
            }
            EqTerm::Refl(inner) => {
                // Reflexivity proof at type level becomes equality type
                let inner_ty = self.term_to_type(inner);
                Type::Eq {
                    ty: Box::new(Type::Generic {
                        name: Text::from("_"),
                        args: List::new(),
                    }),
                    lhs: inner.clone(),
                    rhs: inner.clone(),
                }
            }
            EqTerm::J { .. } => {
                // J eliminator produces a type based on the motive
                // For simplicity, return unit; full impl would evaluate the motive
                Type::Unit
            }
        }
    }

    /// Substitute a type variable in a type
    fn substitute_type_in_type(&self, ty: &Type, var: &Text, replacement: &Type) -> Type {
        match ty {
            Type::Generic { name, args } => {
                if name.as_str() == var.as_str() {
                    return replacement.clone();
                }
                Type::Generic {
                    name: name.clone(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_type_in_type(a, var, replacement))
                        .collect(),
                }
            }
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_type_in_type(a, var, replacement))
                    .collect(),
            },
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.substitute_type_in_type(p, var, replacement))
                    .collect(),
                return_type: Box::new(self.substitute_type_in_type(return_type, var, replacement)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.substitute_type_in_type(t, var, replacement))
                    .collect(),
            ),
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_type_in_type(element, var, replacement)),
                size: *size,
            },
            _ => ty.clone(),
        }
    }
}

/// A refined type environment after matching a constructor.
///
/// When we match a constructor, we learn facts about type indices.
/// For example, matching `cons(head, tail)` on `Vec n T` tells us
/// that `n = m+1` for the length of `tail` being `m`.
#[derive(Debug, Clone)]
pub struct ConstructorRefinement {
    /// The matched constructor
    pub constructor: InductiveConstructor,
    /// Type substitutions learned from matching (e.g., n -> succ(m))
    pub index_subst: IndexMap<Text, Type>,
    /// Equality constraints (e.g., n = m+1)
    pub constraints: List<(Type, Type)>,
}

impl ConstructorRefinement {
    /// Create an empty refinement (no constraints)
    pub fn empty(constructor: InductiveConstructor) -> Self {
        ConstructorRefinement {
            constructor,
            index_subst: IndexMap::new(),
            constraints: List::new(),
        }
    }

    /// Apply the refinement to a type
    pub fn refine_type(&self, ty: &Type) -> Type {
        // Apply index substitutions to the type
        let mut result = ty.clone();
        for (var, replacement) in &self.index_subst {
            result = self.substitute_in_type(&result, var, replacement);
        }
        result
    }

    /// Substitute a type variable in a type
    fn substitute_in_type(&self, ty: &Type, var: &Text, replacement: &Type) -> Type {
        match ty {
            // Check if this is a generic type that references our variable
            Type::Generic { name, args } => {
                // If the name matches, replace with the replacement type
                if name.as_str() == var.as_str() {
                    return replacement.clone();
                }
                // Otherwise, recursively substitute in arguments
                Type::Generic {
                    name: name.clone(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_in_type(a, var, replacement))
                        .collect(),
                }
            }
            Type::Named { path, args } => {
                // Check if this is a named type that references our variable
                // (for indexed types like Vec<T, n> where n might be a variable)
                Type::Named {
                    path: path.clone(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_in_type(a, var, replacement))
                        .collect(),
                }
            }
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.substitute_in_type(p, var, replacement))
                    .collect(),
                return_type: Box::new(self.substitute_in_type(return_type, var, replacement)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.substitute_in_type(t, var, replacement))
                    .collect(),
            ),
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_in_type(element, var, replacement)),
                size: *size,
            },
            Type::Reference { mutable, inner } => Type::Reference {
                inner: Box::new(self.substitute_in_type(inner, var, replacement)),
                mutable: *mutable,
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                inner: Box::new(self.substitute_in_type(inner, var, replacement)),
                mutable: *mutable,
            },
            Type::Pointer { mutable, inner } => Type::Pointer {
                inner: Box::new(self.substitute_in_type(inner, var, replacement)),
                mutable: *mutable,
            },
            // Dependent types: Pi, Sigma, Equality
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
                // Don't substitute if the variable is shadowed by the Pi binder
                if param_name.as_str() == var.as_str() {
                    return ty.clone();
                }
                Type::Pi {
                    param_name: param_name.clone(),
                    param_type: Box::new(self.substitute_in_type(param_type, var, replacement)),
                    return_type: Box::new(self.substitute_in_type(return_type, var, replacement)),
                }
            }
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                // Don't substitute if the variable is shadowed by the Sigma binder
                if fst_name.as_str() == var.as_str() {
                    return ty.clone();
                }
                Type::Sigma {
                    fst_name: fst_name.clone(),
                    fst_type: Box::new(self.substitute_in_type(fst_type, var, replacement)),
                    snd_type: Box::new(self.substitute_in_type(snd_type, var, replacement)),
                }
            }
            Type::Eq {
                lhs,
                rhs,
                ty: eq_ty,
            } => Type::Eq {
                lhs: lhs.clone(), // Terms are handled separately
                rhs: rhs.clone(),
                ty: Box::new(self.substitute_in_type(eq_ty, var, replacement)),
            },
            // For all other types, return unchanged
            _ => ty.clone(),
        }
    }

    /// Check if this refinement proves a type is uninhabited (absurd case)
    pub fn is_absurd(&self) -> bool {
        // Check if any constraint is unsatisfiable
        for (lhs, rhs) in &self.constraints {
            if self.is_unsatisfiable_constraint(lhs, rhs) {
                return true;
            }
        }
        false
    }

    /// Check if a constraint is unsatisfiable (e.g., `Zero = Succ(n)`).
    //
    // A constraint `lhs = rhs` is unsatisfiable when the two types are
    // proven definitionally distinct. This implementation handles four
    // sound, *syntactic* forms of contradiction without requiring a
    // constructor registry lookup (which `ConstructorRefinement` does not
    // have access to):
    //
    //   1. **Distinct named constructor paths** — two `Type::Named` with
    //      different final path segments cannot coincide. This covers
    //      types introduced via `type T is A | B;` and used as
    //      `T::A` vs `T::B`.
    //
    //   2. **Distinct concrete `Meta` values** — two `Type::Meta` with
    //      different `name` fields and no further refinement are
    //      compile-time constants of distinct value; they cannot be
    //      equal (e.g. `Meta{0}` vs `Meta{1}` on the same underlying
    //      `Nat`).
    //
    //   3. **Cross-form Named vs Generic** — a `Type::Named` whose last
    //      path segment is `N` cannot be equal to a `Type::Generic` with
    //      name `M` when `N ≠ M`.
    //
    //   4. **Generic-head disjointness with structural evidence** — two
    //      `Type::Generic` with different names are disjoint *when at
    //      least one side has non-empty arguments*. The presence of
    //      arguments is structural evidence that the side is a
    //      constructor application (not an alias or a type variable);
    //      two distinct constructor applications cannot be equal. Two
    //      0-arity Generics are conservatively *not* proven disjoint
    //      because bare names like `Foo` vs `Bar` may be type variables
    //      or aliases that later unify.
    //
    //  This rule is sound because `ConstructorRefinement` constraints
    //  are accumulated during pattern matching over already-elaborated
    //  types — aliases are resolved before reaching this layer, so two
    //  distinct Generic heads with args really are distinct
    //  constructors. The "both 0-ary, different names" case remains
    //  conservative to protect raw type variables.
    fn is_unsatisfiable_constraint(&self, lhs: &Type, rhs: &Type) -> bool {
        use verum_ast::ty::PathSegment;

        match (lhs, rhs) {
            // --- Rule 1: Named vs Named with distinct constructor paths ---
            (Type::Named { path: p1, .. }, Type::Named { path: p2, .. }) => {
                p1 != p2 && Self::paths_are_disjoint_constructors(p1, p2)
            }

            // --- Rule 2: distinct compile-time Meta values ---
            // Two `Meta { name }` with different names encode different
            // concrete singletons (e.g. `0` vs `1` on `Nat`). If both
            // have no residual refinement, they are provably disjoint.
            (
                Type::Meta {
                    name: n1,
                    refinement: r1,
                    ..
                },
                Type::Meta {
                    name: n2,
                    refinement: r2,
                    ..
                },
            ) => n1 != n2 && r1.is_none() && r2.is_none(),

            // --- Rule 3: cross-form Named vs Generic ---
            // A Named path whose head differs from the Generic's name
            // cannot refer to the same constructor.
            (Type::Generic { name: n, .. }, Type::Named { path, .. })
            | (Type::Named { path, .. }, Type::Generic { name: n, .. }) => {
                match path.segments.last() {
                    Some(PathSegment::Name(ident)) => ident.name.as_str() != n.as_str(),
                    _ => false,
                }
            }

            // --- Rule 4: Generic vs Generic with structural evidence ---
            // Two Generics with different names are disjoint when at
            // least one side carries arguments. Bare-name Generic vs
            // bare-name Generic is conservatively not disjoint (could be
            // type variables).
            (
                Type::Generic {
                    name: n1,
                    args: a1,
                },
                Type::Generic {
                    name: n2,
                    args: a2,
                },
            ) => n1 != n2 && (!a1.is_empty() || !a2.is_empty()),

            // All other combinations (primitives, variables, ...) are
            // conservatively *not proven disjoint*.
            _ => false,
        }
    }

    /// Two `Path`s represent disjoint constructors when their final
    /// segments (the constructor head) differ. The path prefix is
    /// considered the enclosing type; only the head determines identity
    /// at the constructor level. This matches the behaviour expected by
    /// the `test_absurd_*` tests in `dependent_types_tests.rs`.
    fn paths_are_disjoint_constructors(
        p1: &verum_ast::ty::Path,
        p2: &verum_ast::ty::Path,
    ) -> bool {
        use verum_ast::ty::PathSegment;
        match (p1.segments.last(), p2.segments.last()) {
            (Some(PathSegment::Name(a)), Some(PathSegment::Name(b))) => a.name != b.name,
            // Conservatively: if either head is missing or non-Name
            // (e.g. Self, super, a generic segment), we can't decide
            // disjointness, so we say "not proven disjoint".
            _ => false,
        }
    }
}

/// Dependent pattern matching checker.
pub struct DependentPatternChecker<'a> {
    /// Type environment
    env: &'a mut TypeEnv,
    /// Unifier for constraint solving
    unifier: &'a mut Unifier,
    /// Inductive constructors registry for exhaustiveness checking
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Dependent Pattern Matching
    inductive_constructors: &'a Map<Text, List<InductiveConstructor>>,
}

impl<'a> DependentPatternChecker<'a> {
    /// Create a new dependent pattern checker
    pub fn new(
        env: &'a mut TypeEnv,
        unifier: &'a mut Unifier,
        inductive_constructors: &'a Map<Text, List<InductiveConstructor>>,
    ) -> Self {
        DependentPatternChecker {
            env,
            unifier,
            inductive_constructors,
        }
    }

    /// Infer a motive from a match expression.
    ///
    /// The motive describes how the result type varies with the scrutinee.
    /// If all branches have the same type independent of the scrutinee,
    /// we create a simple (constant) motive.
    ///
    /// Algorithm:
    /// 1. Check if result_ty contains references to scrutinee indices
    /// 2. If yes, create a dependent motive
    /// 3. If no, create a simple (constant) motive
    pub fn infer_motive(
        &mut self,
        scrutinee_ty: &Type,
        result_ty: &Type,
    ) -> Result<Motive, TypeError> {
        // Analyze the result type to see if it depends on scrutinee
        let is_dependent = self.type_depends_on_scrutinee(result_ty, scrutinee_ty);

        if is_dependent {
            // Create a dependent motive
            // The result type references scrutinee indices
            Ok(Motive {
                param: Text::from("scrutinee"),
                param_ty: scrutinee_ty.clone(),
                result_ty: result_ty.clone(),
            })
        } else {
            // Create a simple (constant) motive
            Ok(Motive::simple(
                Text::from("scrutinee"),
                scrutinee_ty.clone(),
                result_ty.clone(),
            ))
        }
    }

    /// Check if a type depends on the scrutinee
    ///
    /// This analyzes whether the result type contains references to
    /// indices from the scrutinee type.
    fn type_depends_on_scrutinee(&self, result_ty: &Type, scrutinee_ty: &Type) -> bool {
        // Extract index variables from scrutinee type
        let scrutinee_vars = self.extract_type_variables(scrutinee_ty);

        // Check if result type references any of these variables
        self.type_references_any(result_ty, &scrutinee_vars)
    }

    /// Extract type variables from a type
    fn extract_type_variables(&self, ty: &Type) -> Set<Text> {
        let mut vars = Set::new();

        match ty {
            Type::Generic { name, args } => {
                vars.insert(name.clone());
                for arg in args {
                    for v in self.extract_type_variables(arg) {
                        vars.insert(v);
                    }
                }
            }
            Type::Named { args, .. } => {
                for arg in args {
                    for v in self.extract_type_variables(arg) {
                        vars.insert(v);
                    }
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    for v in self.extract_type_variables(param) {
                        vars.insert(v);
                    }
                }
                for v in self.extract_type_variables(return_type) {
                    vars.insert(v);
                }
            }
            Type::Tuple(tys) => {
                for ty in tys {
                    for v in self.extract_type_variables(ty) {
                        vars.insert(v);
                    }
                }
            }
            Type::Array { element, .. } => {
                for v in self.extract_type_variables(element) {
                    vars.insert(v);
                }
            }
            _ => {}
        }

        vars
    }

    /// Check if a type references any of the given variables
    fn type_references_any(&self, ty: &Type, vars: &Set<Text>) -> bool {
        match ty {
            Type::Generic { name, args } => {
                if vars.contains(name) {
                    return true;
                }
                for arg in args {
                    if self.type_references_any(arg, vars) {
                        return true;
                    }
                }
                false
            }
            Type::Named { args, .. } => {
                for arg in args {
                    if self.type_references_any(arg, vars) {
                        return true;
                    }
                }
                false
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    if self.type_references_any(param, vars) {
                        return true;
                    }
                }
                self.type_references_any(return_type, vars)
            }
            Type::Tuple(tys) => tys.iter().any(|t| self.type_references_any(t, vars)),
            Type::Array { element, .. } => self.type_references_any(element, vars),
            _ => false,
        }
    }

    /// Refine the scrutinee type based on a pattern match.
    ///
    /// When we match a constructor pattern, we learn information about
    /// type indices. For example, matching `cons(h, t)` on `Vec n T`
    /// tells us that `n ≥ 1` and `t : Vec (n-1) T`.
    pub fn refine_on_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        _span: Span,
    ) -> Result<Option<ConstructorRefinement>, TypeError> {
        match &pattern.kind {
            PatternKind::Variant { path, data } => {
                // Look up the constructor for this variant
                let constructor_name = if let Some(segment) = path.segments.last() {
                    match segment {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str(),
                        _ => {
                            return Ok(None);
                        }
                    }
                } else {
                    return Ok(None);
                };

                // Extract constructor argument types and constraints
                let (args, constraints) =
                    self.extract_constructor_info(constructor_name, scrutinee_ty, data)?;

                let constructor = InductiveConstructor {
                    name: Text::from(constructor_name),
                    type_params: List::new(),
                    args,
                    return_type: Box::new(scrutinee_ty.clone()),
                };

                let mut refinement = ConstructorRefinement::empty(constructor);
                refinement.constraints = constraints;

                // Compute index substitutions from unification
                // This is where we learn that matching Cons means n = m + 1
                self.compute_index_substitutions(&mut refinement, scrutinee_ty)?;

                Ok(Some(refinement))
            }

            // Literal patterns can refine to singleton types
            PatternKind::Literal(lit) => {
                // For literals, we know the exact value
                // This can be used for refinement types
                // For example, matching 0 tells us n = 0
                Ok(None) // For now, literals don't create refinements
            }

            // Non-constructor patterns don't refine indices
            _ => Ok(None),
        }
    }

    /// Extract constructor information from the environment
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Inductive Types
    ///
    /// This looks up the constructor's type signature and extracts
    /// argument types and any index constraints. For dependent types,
    /// this is critical for computing index refinements.
    fn extract_constructor_info(
        &self,
        constructor_name: &str,
        scrutinee_ty: &Type,
        pattern_data: &Option<verum_ast::pattern::VariantPatternData>,
    ) -> Result<(List<Box<Type>>, List<(Type, Type)>), TypeError> {
        let mut args = List::new();
        let mut constraints = List::new();

        // First, try to get the type name from the scrutinee
        let type_name = self.extract_type_name(scrutinee_ty);

        // Look up the constructor in the inductive constructors registry
        if let Some(constructors) = self.inductive_constructors.get(&type_name) {
            // Find the matching constructor by name
            for constructor in constructors {
                if constructor.name.as_str() == constructor_name {
                    // Found the constructor! Extract its argument types
                    for arg_type in &constructor.args {
                        // Apply any type arguments from the scrutinee type
                        let specialized_arg =
                            self.specialize_constructor_arg(arg_type, scrutinee_ty);
                        args.push(Box::new(specialized_arg));
                    }

                    // Extract index constraints by unifying constructor return type
                    // with the scrutinee type
                    constraints =
                        self.extract_index_constraints(&constructor.return_type, scrutinee_ty);

                    return Ok((args, constraints));
                }
            }
        }

        // If constructor not found in registry, try to extract types from variant type
        if let Type::Variant(variants) = scrutinee_ty
            && let Some(variant_ty) = variants.get(&Text::from(constructor_name))
        {
            if *variant_ty != Type::Unit {
                // Extract tuple types if it's a tuple
                if let Type::Tuple(tys) = variant_ty {
                    for ty in tys {
                        args.push(Box::new(ty.clone()));
                    }
                } else {
                    // Single argument
                    args.push(Box::new(variant_ty.clone()));
                }
            }
            return Ok((args, constraints));
        }

        // Fallback: infer argument types from the pattern structure
        if let Some(data) = pattern_data {
            use verum_ast::pattern::VariantPatternData;
            match data {
                VariantPatternData::Tuple(patterns) => {
                    for _pat in patterns {
                        // If we can't look up the constructor, we use the type from
                        // the pattern if it has a type annotation, otherwise use a fresh variable
                        args.push(Box::new(Type::Var(TypeVar::fresh())));
                    }
                }
                VariantPatternData::Record { fields, .. } => {
                    for _field in fields {
                        args.push(Box::new(Type::Var(TypeVar::fresh())));
                    }
                }
            }
        }

        Ok((args, constraints))
    }

    /// Extract the type name from a type for registry lookup
    fn extract_type_name(&self, ty: &Type) -> Text {
        match ty {
            Type::Bool => Text::from(WKT::Bool.as_str()),
            Type::Int => Text::from(WKT::Int.as_str()),
            Type::Unit => Text::from("Unit"),
            Type::Generic { name, .. } => name.clone(),
            Type::Named { path, .. } => {
                if let Some(segment) = path.segments.last() {
                    match segment {
                        verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                        _ => Text::from("Unknown"),
                    }
                } else {
                    Text::from("Unknown")
                }
            }
            _ => Text::from("Unknown"),
        }
    }

    /// Specialize a constructor argument type with the scrutinee's type arguments
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Constructor specialization
    fn specialize_constructor_arg(&self, arg_type: &Type, scrutinee_ty: &Type) -> Type {
        // Extract type arguments from scrutinee (e.g., [T, n] from List<T, n>)
        let type_args: List<Type> = match scrutinee_ty {
            Type::Generic { args, .. } => args.clone(),
            Type::Named { args, .. } => args.clone(),
            _ => List::new(),
        };

        // Substitute type parameters in the argument type
        // This handles cases like: arg type is T, scrutinee is List<Int, n>
        // Result: Int
        self.substitute_type_params(arg_type, &type_args)
    }

    /// Substitute type parameters in a type
    fn substitute_type_params(&self, ty: &Type, type_args: &List<Type>) -> Type {
        match ty {
            Type::Generic { name, args } => {
                // Check if this is a type parameter (single letter like T, U, V, A, B)
                if name.as_str().len() == 1 && name.as_str().chars().all(|c| c.is_uppercase()) {
                    // Try to find the corresponding type argument
                    let param_idx = match name.as_str() {
                        "T" | "A" => 0,
                        "U" | "B" => 1,
                        "V" | "C" => 2,
                        _ => return ty.clone(),
                    };
                    if let Some(arg) = type_args.get(param_idx) {
                        return arg.clone();
                    }
                }
                // Recursively substitute in arguments
                Type::Generic {
                    name: name.clone(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_type_params(a, type_args))
                        .collect(),
                }
            }
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| self.substitute_type_params(a, type_args))
                    .collect(),
            },
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.substitute_type_params(t, type_args))
                    .collect(),
            ),
            Type::Array { element, size } => Type::Array {
                element: Box::new(self.substitute_type_params(element, type_args)),
                size: *size,
            },
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.substitute_type_params(inner, type_args)),
            },
            _ => ty.clone(),
        }
    }

    /// Extract index constraints by comparing constructor return type with scrutinee
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Index Refinement
    fn extract_index_constraints(
        &self,
        constructor_return: &Type,
        scrutinee_ty: &Type,
    ) -> List<(Type, Type)> {
        let mut constraints = List::new();

        // Get type arguments from both types
        let (ctor_args, scr_args) = match (constructor_return, scrutinee_ty) {
            (Type::Generic { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Generic { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            _ => return constraints,
        };

        // Compare corresponding type arguments to extract constraints
        for (ctor_arg, scr_arg) in ctor_args.iter().zip(scr_args.iter()) {
            // If the constructor argument is different from the scrutinee argument,
            // we have a constraint that they must be equal
            if ctor_arg != scr_arg {
                constraints.push((ctor_arg.clone(), scr_arg.clone()));
            }
        }

        constraints
    }

    /// Compute index substitutions from unifying constructor with scrutinee
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Index Substitution
    ///
    /// When we match a constructor against a scrutinee type, we learn
    /// facts about type indices. For example:
    /// - Matching Nil against List<T, n> tells us n = 0
    /// - Matching Cons against List<T, n> tells us n = succ(m) for some m
    fn compute_index_substitutions(
        &mut self,
        refinement: &mut ConstructorRefinement,
        scrutinee_ty: &Type,
    ) -> Result<(), TypeError> {
        // Get the constructor's return type
        let ctor_return = &refinement.constructor.return_type;

        // Extract type arguments from both constructor return type and scrutinee
        let (ctor_args, scr_args) = match (ctor_return.as_ref(), scrutinee_ty) {
            (Type::Generic { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Generic { args: a1, .. }, Type::Named { args: a2, .. }) => (a1, a2),
            (Type::Named { args: a1, .. }, Type::Generic { args: a2, .. }) => (a1, a2),
            _ => return Ok(()), // Non-indexed types don't have substitutions
        };

        // For each type argument pair, try to extract a substitution
        for (ctor_arg, scr_arg) in ctor_args.iter().zip(scr_args.iter()) {
            // If the scrutinee argument is a type variable, and the constructor
            // argument is a more specific type, record the substitution
            if let Type::Generic { name, args } = scr_arg
                && args.is_empty()
            {
                // This is a type variable in the scrutinee
                // Record that it should be substituted with the constructor's type
                refinement
                    .index_subst
                    .insert(name.clone(), ctor_arg.clone());
            }

            // Also check for Meta types (compile-time parameters)
            if let Type::Meta { name, .. } = scr_arg {
                refinement
                    .index_subst
                    .insert(name.clone(), ctor_arg.clone());
            }

            // Add equality constraint if types differ
            if ctor_arg != scr_arg {
                refinement
                    .constraints
                    .push((ctor_arg.clone(), scr_arg.clone()));
            }
        }

        // Structural: nullary constructors refine size indices to 0
        if refinement.constructor.args.is_empty() {
            // Any nullary constructor sets size-typed indices to 0
            for arg in scr_args.iter() {
                if self.is_size_type_param(arg) {
                    let zero = Type::Meta {
                        name: Text::from("0"),
                        ty: Box::new(Type::Generic {
                            name: Text::from("Nat"),
                            args: List::new(),
                        }),
                        refinement: None,
                        value: Some(verum_common::ConstValue::UInt(0)),
                    };
                    if let Type::Generic { name, .. } = arg {
                        refinement.index_subst.insert(name.clone(), zero.clone());
                    } else if let Type::Meta { name, .. } = arg {
                        refinement.index_subst.insert(name.clone(), zero.clone());
                    }
                }
            }
        }

        // Structural: constructors with args refine size indices to succ(n)
        if !refinement.constructor.args.is_empty() {
            for arg in scr_args.iter() {
                if self.is_size_type_param(arg) {
                    let pred_var = TypeVar::fresh();
                    let pred = Type::Var(pred_var);
                    let succ = Type::Generic {
                        name: Text::from("Succ"),
                        args: List::from_iter([pred.clone()]),
                    };
                    if let Type::Generic { name, .. } = arg {
                        refinement.index_subst.insert(name.clone(), succ);
                    } else if let Type::Meta { name, .. } = arg {
                        refinement.index_subst.insert(name.clone(), succ);
                    }
                }
            }
        }

        Ok(())
    }

    /// Check if a type represents a size/length parameter
    fn is_size_type_param(&self, ty: &Type) -> bool {
        match ty {
            Type::Generic { name, args } => {
                // Common size parameter names
                let name_str = name.as_str().to_lowercase();
                (name_str == "n"
                    || name_str == "len"
                    || name_str == "size"
                    || name_str == "length"
                    || name_str == "count")
                    && args.is_empty()
            }
            Type::Meta {
                name, ty: inner_ty, ..
            } => {
                // Check if it's a Nat or usize meta parameter
                if let Type::Generic {
                    name: inner_name, ..
                } = inner_ty.as_ref()
                {
                    inner_name.as_str() == "Nat" || inner_name.as_str() == "usize"
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Check if a pattern match is exhaustive for a dependent type.
    ///
    /// For dependent types, exhaustiveness checking must account for
    /// type indices. Some patterns may be absurd (impossible) due to
    /// index constraints.
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Pattern Coverage for Indexed Types
    ///
    /// # Example
    /// ```verum
    /// // For List<T, n>, different cases required based on n:
    /// fn head<T, n: meta Nat>(xs: List<T, Succ(n): meta Nat>) -> T =
    ///     match xs {
    ///         Cons(x, _) => x
    ///         // No Nil case needed - type ensures non-empty (n = Succ(m))
    ///     }
    /// ```
    ///
    /// NOTE: This method delegates to the unified exhaustiveness checker in
    /// `exhaustiveness::dependent` for the matrix-based algorithm while
    /// preserving dependent type index awareness.
    pub fn check_exhaustiveness(
        &mut self,
        scrutinee_ty: &Type,
        patterns: &[Pattern],
    ) -> Result<bool, TypeError> {
        // Delegate to the unified dependent exhaustiveness checker
        use crate::exhaustiveness::check_exhaustiveness_unified;

        let result =
            check_exhaustiveness_unified(patterns, scrutinee_ty, self.env, self.inductive_constructors)?;

        if result.base.is_exhaustive {
            Ok(true)
        } else {
            // Generate detailed error message from witnesses
            let missing = result
                .base
                .uncovered_witnesses
                .iter()
                .map(|w| format!("{}", w))
                .collect::<Vec<_>>()
                .join(", ");

            Err(TypeError::Other(Text::from(format!(
                "Non-exhaustive pattern match: missing cases: {}",
                missing
            ))))
        }
    }

    /// Legacy check_exhaustiveness implementation (deprecated)
    ///
    /// Use check_exhaustiveness which delegates to the unified checker.
    #[deprecated(note = "Use check_exhaustiveness which uses the unified checker")]
    pub fn check_exhaustiveness_legacy(
        &mut self,
        scrutinee_ty: &Type,
        patterns: &[Pattern],
    ) -> Result<bool, TypeError> {
        // Step 1: Enumerate all constructors for the scrutinee type
        let constructors = self.get_type_constructors(scrutinee_ty)?;

        if constructors.is_empty() {
            // Not an inductive type, no exhaustiveness check needed
            return Ok(true);
        }

        // Step 2: Check that each constructor is covered
        let mut uncovered_constructors = List::new();

        for constructor in &constructors {
            let mut is_covered = false;

            for pattern in patterns {
                if self.pattern_covers_constructor(pattern, &constructor.name)? {
                    // Check if this coverage is absurd (impossible)
                    let refinement_opt =
                        self.refine_on_pattern(pattern, scrutinee_ty, pattern.span)?;

                    if let Some(ref refinement) = refinement_opt {
                        if !refinement.is_absurd() {
                            is_covered = true;
                            break;
                        }
                    } else {
                        is_covered = true;
                        break;
                    }
                }
            }

            if !is_covered {
                // Check if this constructor is impossible given the indices
                // For example, Nil is impossible for List<T, Succ(n)>
                if !self.is_constructor_possible(constructor, scrutinee_ty)? {
                    // Constructor is impossible due to index constraints
                    // Not truly uncovered - it's an absurd case
                    continue;
                }
                uncovered_constructors.push(constructor.name.clone());
            }
        }

        if uncovered_constructors.is_empty() {
            Ok(true)
        } else {
            // In a full implementation, we would generate a detailed error
            // showing which constructors are not covered
            Err(TypeError::Other(Text::from(format!(
                "Non-exhaustive pattern match: missing constructors: {}",
                uncovered_constructors
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))))
        }
    }

    /// Check if a constructor is possible given the scrutinee type's indices.
    ///
    /// For indexed types like List<T, n>, some constructors may be impossible
    /// based on the index values. For example:
    /// - Nil is impossible if n = Succ(m) (non-zero length)
    /// - Cons is impossible if n = Zero (zero length)
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Index-aware exhaustiveness
    fn is_constructor_possible(
        &self,
        constructor: &InductiveConstructor,
        scrutinee_ty: &Type,
    ) -> Result<bool, TypeError> {
        // Extract indices from scrutinee type
        let scrutinee_indices = match scrutinee_ty {
            Type::Generic { args, .. } | Type::Named { args, .. } => args,
            _ => return Ok(true), // Non-indexed type, all constructors possible
        };

        // Extract indices from constructor return type
        let ctor_indices = match constructor.return_type.as_ref() {
            Type::Generic { args, .. } | Type::Named { args, .. } => args,
            _ => return Ok(true),
        };

        // Check if indices are compatible
        for (scrutinee_idx, ctor_idx) in scrutinee_indices.iter().zip(ctor_indices.iter()) {
            // Check for obvious incompatibilities
            // For example: scrutinee has Succ(n), constructor has Zero
            if self.are_indices_incompatible(scrutinee_idx, ctor_idx) {
                return Ok(false); // Constructor impossible
            }
        }

        Ok(true)
    }

    /// Check if two type indices are incompatible.
    ///
    /// This detects cases where indices clearly contradict each other,
    /// such as Zero vs Succ(n), or different concrete values.
    fn are_indices_incompatible(&self, idx1: &Type, idx2: &Type) -> bool {
        match (idx1, idx2) {
            // Zero vs Succ - incompatible
            (Type::Generic { name: n1, args: a1 }, Type::Generic { name: n2, args: a2 }) => {
                if (n1.as_str() == "Zero" && n2.as_str() == "Succ")
                    || (n1.as_str() == "Succ" && n2.as_str() == "Zero")
                {
                    return true;
                }
                false
            }
            // Different concrete meta values - incompatible
            (Type::Meta { name: n1, .. }, Type::Meta { name: n2, .. }) => n1 != n2,
            _ => false,
        }
    }

    /// Check pattern coverage for indexed types.
    ///
    /// For indexed types, we need to verify that patterns cover all possible
    /// index values, not just all constructors.
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Pattern Coverage for Indexed Types
    ///
    /// # Example
    /// ```verum
    /// // Coverage depends on the length index:
    /// fn safe_head<T, n>(xs: List<T, n>) -> Maybe<T> =
    ///     match xs {
    ///         Nil => None          // Case: n = 0
    ///         Cons(x, _) => Some(x) // Case: n = Succ(m)
    ///     }
    /// // Both constructors covered for all n
    /// ```
    pub fn check_indexed_coverage(
        &mut self,
        scrutinee_ty: &Type,
        patterns: &[Pattern],
    ) -> Result<CoverageReport, TypeError> {
        let constructors = self.get_type_constructors(scrutinee_ty)?;

        // Build coverage matrix: which patterns cover which constructors
        let mut coverage_matrix: Map<Text, List<usize>> = Map::new();

        for (pattern_idx, pattern) in patterns.iter().enumerate() {
            for constructor in &constructors {
                if self.pattern_covers_constructor(pattern, &constructor.name)? {
                    coverage_matrix
                        .entry(constructor.name.clone())
                        .or_default()
                        .push(pattern_idx);
                }
            }
        }

        // Check for uncovered constructors
        let mut uncovered = List::new();
        for constructor in &constructors {
            if !coverage_matrix.contains_key(&constructor.name) {
                // Check if constructor is impossible given indices
                if self.is_constructor_possible(constructor, scrutinee_ty)? {
                    uncovered.push(constructor.name.clone());
                }
            }
        }

        // Check for redundant patterns (patterns that never match)
        let mut redundant = List::new();
        for (pattern_idx, pattern) in patterns.iter().enumerate() {
            if self.is_pattern_redundant(pattern, scrutinee_ty, pattern_idx, patterns)? {
                redundant.push(pattern_idx);
            }
        }

        Ok(CoverageReport {
            is_exhaustive: uncovered.is_empty(),
            uncovered_constructors: uncovered,
            redundant_patterns: redundant,
        })
    }

    /// Check if a pattern is redundant (never matches).
    ///
    /// A pattern is redundant if:
    /// 1. It's absurd (impossible due to index constraints)
    /// 2. It's already covered by earlier patterns
    fn is_pattern_redundant(
        &mut self,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        pattern_idx: usize,
        all_patterns: &[Pattern],
    ) -> Result<bool, TypeError> {
        // Check if pattern is absurd
        let refinement_opt = self.refine_on_pattern(pattern, scrutinee_ty, pattern.span)?;
        if let Some(ref refinement) = refinement_opt
            && refinement.is_absurd()
        {
            return Ok(true); // Absurd pattern is redundant
        }

        // Check if earlier patterns subsume this one
        for earlier_pattern in &all_patterns[..pattern_idx] {
            if self.pattern_subsumes(earlier_pattern, pattern)? {
                return Ok(true); // Subsumed by earlier pattern
            }
        }

        Ok(false)
    }

    /// Check if one pattern subsumes another (covers all its cases).
    fn pattern_subsumes(&self, p1: &Pattern, p2: &Pattern) -> Result<bool, TypeError> {
        match (&p1.kind, &p2.kind) {
            // Wildcard and identifier patterns subsume everything
            (PatternKind::Wildcard, _) | (PatternKind::Ident { .. }, _) => Ok(true),

            // Same variant pattern - check recursively
            (
                PatternKind::Variant {
                    path: path1,
                    data: data1,
                },
                PatternKind::Variant {
                    path: path2,
                    data: data2,
                },
            ) => {
                if path1 == path2 {
                    // Same constructor - check if payloads subsume
                    match (data1, data2) {
                        (None, None) => Ok(true),
                        (Some(_), None) => Ok(false),
                        (None, Some(_)) => Ok(false),
                        (Some(_), Some(_)) => {
                            // Would need to check payload patterns recursively
                            Ok(false) // Conservative: assume not subsumed
                        }
                    }
                } else {
                    Ok(false)
                }
            }

            _ => Ok(false), // Conservative: assume not subsumed
        }
    }

    /// Get all constructors for a type
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1 - Inductive Types
    fn get_type_constructors(&self, ty: &Type) -> Result<List<InductiveConstructor>, TypeError> {
        // Extract the type name from the type
        let type_name = match ty {
            // Primitive types with known constructors
            Type::Bool => Text::from(WKT::Bool.as_str()),
            Type::Unit => Text::from("Unit"),

            // Generic types like Maybe<T>, Result<T, E>, List<T>
            Type::Generic { name, .. } => name.clone(),

            // Named types
            Type::Named { path, .. } => {
                // Extract the last segment of the path as the type name
                if let Some(segment) = path.segments.last() {
                    match segment {
                        verum_ast::ty::PathSegment::Name(ident) => Text::from(ident.name.as_str()),
                        // Other path segments (Self, Super, Crate, Relative) don't have constructors
                        _ => return Ok(List::new()),
                    }
                } else {
                    return Ok(List::new());
                }
            }

            // Variant types are enum-like with known variants
            Type::Variant(variants) => {
                // Create constructors from variant definitions
                let constructors = variants
                    .iter()
                    .map(|(name, variant_ty)| {
                        if *variant_ty == Type::Unit {
                            InductiveConstructor::unit(name.clone(), ty.clone())
                        } else {
                            InductiveConstructor::with_args(
                                name.clone(),
                                List::from_iter([variant_ty.clone()]),
                                ty.clone(),
                            )
                        }
                    })
                    .collect();
                return Ok(constructors);
            }

            // Other types don't have constructors for exhaustiveness checking
            _ => return Ok(List::new()),
        };

        // Query the inductive constructors registry
        match self.inductive_constructors.get(&type_name) {
            Some(constructors) => Ok(constructors.clone()),
            None => Ok(List::new()),
        }
    }

    /// Check if a pattern covers a specific constructor
    fn pattern_covers_constructor(
        &self,
        pattern: &Pattern,
        constructor_name: &Text,
    ) -> Result<bool, TypeError> {
        match &pattern.kind {
            PatternKind::Variant { path, .. } => {
                if let Some(segment) = path.segments.last() {
                    match segment {
                        verum_ast::ty::PathSegment::Name(id) => {
                            Ok(id.name.as_str() == constructor_name.as_str())
                        }
                        _ => Ok(false),
                    }
                } else {
                    Ok(false)
                }
            }
            PatternKind::Wildcard => {
                // Wildcard covers all constructors
                Ok(true)
            }
            PatternKind::Ident { .. } => {
                // Identifier pattern covers all constructors
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Type check a match expression with dependent types.
    ///
    /// This extends regular pattern matching with:
    /// 1. Motive inference (how result type depends on scrutinee)
    /// 2. Constructor refinement (learning index constraints)
    /// 3. Branch type checking with refined types
    /// 4. Absurd pattern detection (impossible cases)
    /// 5. With-clause verification for proof obligations
    /// 6. View pattern support
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — Dependent Pattern Matching
    ///
    /// NOTE: This method now uses the unified exhaustiveness checker from
    /// `exhaustiveness::dependent` which provides the matrix-based algorithm
    /// with dependent type awareness.
    pub fn check_dependent_match(
        &mut self,
        scrutinee_ty: &Type,
        arms: &[verum_ast::pattern::MatchArm],
        result_ty: &Type,
        span: Span,
    ) -> Result<Type, TypeError> {
        if arms.is_empty() {
            return Err(TypeError::Other(Text::from(
                "Match expression must have at least one arm",
            )));
        }

        // Use the unified dependent match checker for exhaustiveness
        use crate::exhaustiveness::check_dependent_match_unified;

        let exhaustiveness_result =
            check_dependent_match_unified(scrutinee_ty, result_ty, arms, self.env, self.inductive_constructors)?;

        // Infer the motive (how result type depends on scrutinee)
        let motive = self.infer_motive(scrutinee_ty, result_ty)?;

        // Type check each arm with constructor refinement
        let mut refined_result_ty = result_ty.clone();

        for (arm_idx, arm) in arms.iter().enumerate() {
            // Check if this arm is absurd according to the unified checker
            if exhaustiveness_result.absurd_patterns.contains(&arm_idx) {
                // Absurd pattern - this branch is unreachable
                // No need to type check the body
                continue;
            }

            // Get constructor refinement for this pattern
            let refinement_opt = self.refine_on_pattern(&arm.pattern, scrutinee_ty, arm.span)?;

            // Verify with-clause proof obligations if present
            // Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — lines 386-391
            if let Some(ref with_exprs) = arm.with_clause {
                self.check_with_clause(with_exprs.as_slice(), &refinement_opt, scrutinee_ty)?;
            }

            // Refine the result type based on constructor constraints
            if let Some(refinement) = refinement_opt {
                refined_result_ty = refinement.refine_type(&motive.result_ty);
            }

            // Type check the arm body with the refined result type
            // (This would be done by the caller in the full implementation)
        }

        // Check exhaustiveness result from unified checker
        if !exhaustiveness_result.base.is_exhaustive {
            let missing = exhaustiveness_result
                .base
                .uncovered_witnesses
                .iter()
                .map(|w| format!("{}", w))
                .collect::<Vec<_>>()
                .join(", ");

            return Err(TypeError::Other(Text::from(format!(
                "Non-exhaustive pattern match: missing cases: {}",
                missing
            ))));
        }

        Ok(refined_result_ty)
    }

    /// Check with-clause proof obligations for a pattern match arm.
    ///
    /// The with-clause specifies constraints that must hold when the pattern matches.
    /// For dependent types, these constraints refine the type information available
    /// in the match arm body.
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — lines 386-391
    ///
    /// # Example
    /// ```verum
    /// fn is_zero(n: Nat) -> bool with (n = 0) | (n ≠ 0) =
    ///     match n {
    ///         Zero => true    // with-clause: n = 0
    ///         Succ(_) => false // with-clause: n ≠ 0
    ///     }
    /// ```
    fn check_with_clause(
        &mut self,
        with_exprs: &[verum_ast::expr::Expr],
        refinement_opt: &Option<ConstructorRefinement>,
        scrutinee_ty: &Type,
    ) -> Result<(), TypeError> {
        // For each expression in the with-clause, verify it holds given
        // the constructor refinement
        for expr in with_exprs {
            // Check that the expression is a valid boolean/proposition
            // In a full implementation, we would:
            // 1. Type check the expression to ensure it's a Bool or Prop
            // 2. Verify it's derivable from the constructor refinement
            // 3. Add it as an assumption for the arm body

            // For now, we just validate the structure
            match &expr.kind {
                verum_ast::expr::ExprKind::Binary { op, .. } => {
                    // Common with-clause patterns: x = y, x ≠ y, x < y, etc.
                    use verum_ast::expr::BinOp;
                    match op {
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                            // Valid constraint expression
                        }
                        _ => {
                            return Err(TypeError::Other(Text::from(
                                "With-clause must contain comparison or equality constraint",
                            )));
                        }
                    }
                }
                _ => {
                    // Allow other proposition forms (unary negation, calls to predicates, etc.)
                }
            }
        }

        Ok(())
    }

    /// Check a view pattern.
    ///
    /// View patterns apply a view function to the scrutinee before pattern matching,
    /// allowing pattern matching on computed properties rather than constructors.
    ///
    /// Dependent pattern matching: type-aware case analysis where match arms refine dependent types — .2 lines 401-427
    ///
    /// # Example
    /// ```verum
    /// view Parity : Nat -> Type {
    ///     Even : (n: Nat) -> Parity(2 * n),
    ///     Odd : (n: Nat) -> Parity(2 * n + 1)
    /// }
    ///
    /// fn is_even(n: Nat) -> bool =
    ///     match parity(n) {  // View pattern
    ///         Even(_) => true,
    ///         Odd(_) => false
    ///     }
    /// ```
    pub fn check_view_pattern(
        &mut self,
        view_function: &verum_ast::expr::Expr,
        pattern: &Pattern,
        scrutinee_ty: &Type,
        span: Span,
    ) -> Result<(Type, Option<ConstructorRefinement>), TypeError> {
        // Dependent pattern matching: type-aware case analysis where match arms refine dependent types — .2 - View Patterns
        //
        // View patterns allow matching through a transformation function:
        //   match parity(n) {
        //       Even(k) => ...,  // n = 2*k
        //       Odd(k) => ...    // n = 2*k + 1
        //   }
        //
        // The view function has type: ScrutineeType -> ViewType
        // We need to:
        // 1. Infer the type of the view function application
        // 2. Extract the ViewType from the function's return type
        // 3. Check the inner pattern against the ViewType
        // 4. Return refinements that flow back to the scrutinee

        use verum_ast::expr::ExprKind;

        // Step 1: Extract the view function's return type
        let view_type = match &view_function.kind {
            // Direct function call: parity(n)
            ExprKind::Call { func, args, .. } => {
                // Look up the function's type in environment
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                        if let Some(func_scheme) = self.env.lookup(ident.name.as_str()) {
                            // Get return type from function type
                            let func_ty = func_scheme.instantiate();
                            if let Type::Function { return_type, .. } = func_ty {
                                *return_type
                            } else {
                                // Not a function type - type error
                                return Err(TypeError::NotAFunction {
                                    ty: Text::from(format!("{:?}", func_ty)),
                                    span,
                                });
                            }
                        } else {
                            // Unknown function - use fresh type variable
                            Type::Var(crate::ty::TypeVar::fresh())
                        }
                    } else {
                        // Complex path - fall back to type variable
                        Type::Var(crate::ty::TypeVar::fresh())
                    }
                } else {
                    // Non-path function - infer type from expression
                    Type::Var(crate::ty::TypeVar::fresh())
                }
            }

            // Path expression treated as view function
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    if let Some(scheme) = self.env.lookup(ident.name.as_str()) {
                        let ty = scheme.instantiate();
                        if let Type::Function { return_type, .. } = ty {
                            *return_type
                        } else {
                            ty
                        }
                    } else {
                        Type::Var(crate::ty::TypeVar::fresh())
                    }
                } else {
                    Type::Var(crate::ty::TypeVar::fresh())
                }
            }

            // Other expressions - use type variable for now
            _ => Type::Var(crate::ty::TypeVar::fresh()),
        };

        // Step 2: Check the inner pattern against the view type
        // For variant patterns, we need to look up constructor info
        let refinement = match &pattern.kind {
            PatternKind::Variant { path, data: _ } => {
                // Look up the constructor for this variant
                // For now, we don't have constructor info available here,
                // so we return None and let the pattern checker handle it
                None
            }

            PatternKind::Ident { .. } | PatternKind::Wildcard => {
                // Non-constructor patterns don't produce refinements
                None
            }

            PatternKind::Literal { .. } => {
                // Literal patterns don't produce type refinements
                None
            }

            PatternKind::Tuple(_) | PatternKind::Record { .. } | PatternKind::Slice { .. } => {
                // Structural patterns - would need more complex handling
                None
            }

            _ => None,
        };

        // Step 3: Return the view type and any refinement
        // The view type is what the pattern will be checked against
        Ok((view_type, refinement))
    }

    /// Bind pattern variables with refined types from constructor matching.
    ///
    /// When we match a constructor, the pattern variables get refined types
    /// based on the constructor's argument types. For example:
    /// ```verum
    /// match vec {
    ///   cons(h, t) => ...  // t : Vec n T (where vec : Vec (n+1) T)
    /// }
    /// ```
    pub fn bind_pattern_refined(
        &mut self,
        pattern: &Pattern,
        ty: &Type,
        refinement: Option<&ConstructorRefinement>,
    ) -> Result<(), TypeError> {
        match &pattern.kind {
            PatternKind::Wildcard => Ok(()),

            PatternKind::Ident { name, .. } => {
                // Apply refinement to the type if present
                let refined_ty = if let Some(r) = refinement {
                    r.refine_type(ty)
                } else {
                    ty.clone()
                };

                self.env
                    .insert(name.name.as_str(), TypeScheme::mono(refined_ty));
                Ok(())
            }

            PatternKind::Variant { path: _, data } => {
                // For variant patterns with payload, bind the payload variables
                if let Some(variant_data) = data {
                    use verum_ast::pattern::VariantPatternData;
                    match variant_data {
                        VariantPatternData::Tuple(patterns) => {
                            // Get constructor argument types
                            if let Some(r) = refinement {
                                // Bind each pattern to its corresponding argument type
                                for (i, pat) in patterns.iter().enumerate() {
                                    if i < r.constructor.args.len() {
                                        let arg_ty = r.constructor.args[i].as_ref();
                                        let refined_arg_ty = r.refine_type(arg_ty);
                                        self.bind_pattern_refined(
                                            pat,
                                            &refined_arg_ty,
                                            refinement,
                                        )?;
                                    }
                                }
                            } else {
                                // No refinement, bind with base type
                                for pat in patterns {
                                    self.bind_pattern_refined(pat, ty, None)?;
                                }
                            }
                        }
                        VariantPatternData::Record { fields, .. } => {
                            // Record-style variant bindings
                            // Would extract field types from constructor
                            for field_pat in fields {
                                if let Some(ref pat) = field_pat.pattern {
                                    self.bind_pattern_refined(pat, ty, refinement)?;
                                } else {
                                    // Shorthand binding
                                    self.env.insert(
                                        field_pat.name.name.as_str(),
                                        TypeScheme::mono(ty.clone()),
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(())
            }

            PatternKind::Tuple(patterns) => {
                if let Type::Tuple(types) = ty {
                    for (pat, elem_ty) in patterns.iter().zip(types.iter()) {
                        self.bind_pattern_refined(pat, elem_ty, refinement)?;
                    }
                }
                Ok(())
            }

            _ => {
                // Other patterns don't interact with dependent types
                Ok(())
            }
        }
    }
}

/// Coverage analysis report for pattern matching.
///
/// Provides detailed information about pattern coverage including
/// missing cases and redundant patterns.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    /// Whether the patterns are exhaustive
    pub is_exhaustive: bool,
    /// Constructors not covered by any pattern
    pub uncovered_constructors: List<Text>,
    /// Patterns that are redundant (never match)
    pub redundant_patterns: List<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_motive() {
        let param_ty = Type::Int;
        let result_ty = Type::Bool;
        let motive = Motive::simple(Text::from("x"), param_ty.clone(), result_ty.clone());

        assert_eq!(motive.param, Text::from("x"));
        assert_eq!(motive.param_ty, param_ty);
        assert_eq!(motive.result_ty, result_ty);
    }

    #[test]
    fn test_empty_refinement() {
        let constructor = InductiveConstructor {
            name: Text::from("Cons"),
            type_params: List::new(),
            args: List::new(),
            return_type: Box::new(Type::Int),
        };

        let refinement = ConstructorRefinement::empty(constructor);
        assert_eq!(refinement.index_subst.len(), 0);
        assert_eq!(refinement.constraints.len(), 0);
        assert!(!refinement.is_absurd());
    }

    #[test]
    fn test_refinement_type_substitution() {
        let constructor = InductiveConstructor {
            name: Text::from("Cons"),
            type_params: List::new(),
            args: List::new(),
            return_type: Box::new(Type::Int),
        };

        let mut refinement = ConstructorRefinement::empty(constructor);

        // Add a substitution: n -> 5
        let type_var = TypeVar::fresh();
        refinement.index_subst.insert(Text::from("n"), Type::Int);

        // Apply refinement to a type containing 'n'
        let original = Type::Var(type_var);
        let refined = refinement.refine_type(&original);

        // Type should be unchanged since we're substituting by name, not by TypeVar
        assert_eq!(refined, original);
    }
}
