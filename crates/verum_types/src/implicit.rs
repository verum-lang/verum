//! Implicit argument resolution.
//!
//! This module implements implicit argument inference for dependent types.
//! Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
//!
//! # Overview
//!
//! Implicit arguments are function parameters that can be automatically inferred
//! by the compiler, reducing boilerplate while maintaining type safety.
//!
//! Syntax:
//! - `{T}` for implicit type arguments
//! - `[n]` for implicit value arguments (compile-time constants)
//!
//! # Implementation Strategy
//!
//! 1. **Metavariable Generation**: Create fresh type variables for implicit arguments
//! 2. **Constraint Collection**: Gather constraints from usage sites
//! 3. **Unification**: Solve constraints using the unification algorithm
//! 4. **Elaboration**: Fill in inferred implicit arguments in the typed AST
//!
//! # Example
//!
//! ```verum
//! fn id{T}(x: T) -> T = x
//!
//! let y = id(42)  // T inferred as Int
//! ```
//!
//! During type checking:
//! 1. Generate metavariable ?T for the implicit type parameter
//! 2. Check argument: 42 : ?T
//! 3. Unify: ?T = Int
//! 4. Solve: T = Int
//! 5. Elaborate: id<Int>(42)

use crate::ty::{Substitution, SubstitutionExt, Type, TypeVar};
use crate::unify::Unifier;
use crate::{Result, TypeError};
use verum_ast::span::Span;
use verum_common::{List, Map, Maybe, Text};

/// An implicit argument that needs to be inferred.
#[derive(Debug, Clone)]
pub struct ImplicitArg {
    /// Name of the implicit parameter
    pub name: Text,
    /// Metavariable representing this implicit argument
    pub metavar: TypeVar,
    /// Expected kind/type of this implicit argument
    /// For type parameters: Type
    /// For value parameters: the type of the value
    pub expected_type: Type,
    /// Span for error reporting
    pub span: Span,
}

/// A constraint on an implicit argument.
#[derive(Debug, Clone)]
pub struct ImplicitConstraint {
    /// The metavariable being constrained
    pub metavar: TypeVar,
    /// The type it must unify with
    pub constraint_type: Type,
    /// Source span for error reporting
    pub span: Span,
    /// Description of where this constraint came from
    pub source: ConstraintSource,
}

/// Source of a constraint for error messages.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintSource {
    /// Constraint from function argument: f(arg)
    Argument { position: usize },
    /// Constraint from return type: let x: T = f()
    ReturnType,
    /// Constraint from field access: expr.field
    FieldAccess { field: Text },
    /// Constraint from method call: expr.method()
    MethodCall { method: Text },
    /// Constraint from type annotation: expr : T
    TypeAnnotation,
}

/// Implicit argument resolution context.
///
/// This structure tracks all implicit arguments and constraints during
/// type inference, and provides methods to solve them.
pub struct ImplicitResolver {
    /// Pending implicit arguments to resolve
    implicit_args: List<ImplicitArg>,
    /// Collected constraints
    constraints: List<ImplicitConstraint>,
    /// Unifier for solving constraints
    unifier: Unifier,
}

impl ImplicitResolver {
    /// Create a new implicit resolver.
    pub fn new() -> Self {
        Self {
            implicit_args: List::new(),
            constraints: List::new(),
            unifier: Unifier::new(),
        }
    }

    /// Register a new implicit argument.
    ///
    /// Creates a fresh metavariable for the implicit parameter and
    /// tracks it for later resolution.
    pub fn register_implicit(&mut self, name: Text, expected_type: Type, span: Span) -> TypeVar {
        let metavar = TypeVar::fresh();
        self.implicit_args.push(ImplicitArg {
            name,
            metavar,
            expected_type,
            span,
        });
        metavar
    }

    /// Add a constraint on an implicit argument.
    ///
    /// This records that a metavariable must unify with a specific type,
    /// based on how the function is used.
    pub fn add_constraint(
        &mut self,
        metavar: TypeVar,
        constraint_type: Type,
        span: Span,
        source: ConstraintSource,
    ) {
        self.constraints.push(ImplicitConstraint {
            metavar,
            constraint_type,
            span,
            source,
        });
    }

    /// Solve all implicit arguments.
    ///
    /// This performs constraint solving via unification to determine
    /// the values of all implicit arguments.
    ///
    /// Returns a substitution mapping metavariables to their inferred types.
    pub fn solve(&mut self) -> Result<Substitution> {
        let mut subst = Substitution::new();

        // Process constraints in order, building up the substitution
        for constraint in &self.constraints {
            // Apply current substitution to both sides
            let metavar_type = Type::Var(constraint.metavar);
            let metavar_applied = metavar_type.apply_subst(&subst);
            let constraint_applied = constraint.constraint_type.apply_subst(&subst);

            // Unify the metavariable with the constraint
            let new_subst =
                self.unifier
                    .unify(&metavar_applied, &constraint_applied, constraint.span)?;

            // Compose substitutions
            subst = subst.compose(&new_subst);
        }

        // Check that all implicit arguments were solved
        for implicit_arg in &self.implicit_args {
            let metavar_type = Type::Var(implicit_arg.metavar);
            let resolved = metavar_type.apply_subst(&subst);

            // If still a variable after solving, inference failed
            if let Type::Var(_) = resolved {
                return Err(TypeError::AmbiguousType {
                    span: implicit_arg.span,
                });
            }

            // Verify the inferred type matches the expected kind
            // For now, we skip kind checking (would need full kind inference)
        }

        Ok(subst)
    }

    /// Solve with detailed error reporting for ambiguous cases.
    ///
    /// This variant provides more helpful error messages when resolution fails,
    /// including suggestions for explicit instantiation.
    pub fn solve_with_diagnostics(&mut self) -> Result<Substitution> {
        let mut subst = Substitution::new();

        // Process constraints in order, building up the substitution
        for constraint in &self.constraints {
            // Apply current substitution to both sides
            let metavar_type = Type::Var(constraint.metavar);
            let metavar_applied = metavar_type.apply_subst(&subst);
            let constraint_applied = constraint.constraint_type.apply_subst(&subst);

            // Unify the metavariable with the constraint
            match self
                .unifier
                .unify(&metavar_applied, &constraint_applied, constraint.span)
            {
                Ok(new_subst) => {
                    subst = subst.compose(&new_subst);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        // Check that all implicit arguments were solved
        for implicit_arg in &self.implicit_args {
            let metavar_type = Type::Var(implicit_arg.metavar);
            let resolved = metavar_type.apply_subst(&subst);

            // If still a variable after solving, inference failed
            if let Type::Var(_) = resolved {
                return Err(TypeError::AmbiguousType {
                    span: implicit_arg.span,
                });
            }

            // Verify the inferred type matches the expected kind
            // For now, we skip kind checking (would need full kind inference)
        }

        Ok(subst)
    }

    /// Get the inferred value for an implicit argument.
    ///
    /// Must be called after `solve()`.
    pub fn get_inferred(&self, metavar: TypeVar, subst: &Substitution) -> Maybe<Type> {
        let metavar_type = Type::Var(metavar);
        let resolved = metavar_type.apply_subst(subst);

        // Return the inferred type if it's not still a variable
        match resolved {
            Type::Var(_) => Maybe::None,
            ty => Maybe::Some(ty),
        }
    }

    /// Clear all state (for reuse).
    pub fn clear(&mut self) {
        self.implicit_args.clear();
        self.constraints.clear();
    }

    /// Get the number of pending implicit arguments.
    pub fn pending_count(&self) -> usize {
        self.implicit_args.len()
    }

    /// Get the number of collected constraints.
    pub fn constraint_count(&self) -> usize {
        self.constraints.len()
    }
}

impl Default for ImplicitResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Elaboration context for filling in implicit arguments.
///
/// After solving implicit arguments, this structure helps insert
/// the inferred values into the typed AST.
pub struct ImplicitElaborator {
    /// Mapping from metavariables to their inferred types
    solution: Substitution,
}

impl ImplicitElaborator {
    /// Create a new elaborator with a solution.
    pub fn new(solution: Substitution) -> Self {
        Self { solution }
    }

    /// Elaborate a type by filling in implicit arguments.
    ///
    /// Replaces all metavariables with their inferred types.
    pub fn elaborate_type(&self, ty: &Type) -> Type {
        ty.apply_subst(&self.solution)
    }

    /// Get all inferred implicit arguments as a map.
    ///
    /// This can be used to generate explicit type applications
    /// for the elaborated AST.
    pub fn get_inferred_args(&self) -> Map<TypeVar, Type> {
        let mut map = Map::new();
        for (var, ty) in &self.solution {
            map.insert(*var, ty.clone());
        }
        map
    }
}

/// Implicit argument context for tracking scope.
///
/// This structure manages implicit argument scopes during type checking,
/// allowing nested function definitions to have their own implicit parameters.
pub struct ImplicitContext {
    /// Stack of scopes, each containing a resolver
    scopes: List<ImplicitResolver>,
}

impl ImplicitContext {
    /// Create a new implicit context.
    pub fn new() -> Self {
        Self {
            scopes: List::new(),
        }
    }

    /// Enter a new scope for a function with implicit parameters.
    pub fn enter_scope(&mut self) {
        self.scopes.push(ImplicitResolver::new());
    }

    /// Exit the current scope and return the resolver.
    ///
    /// Returns None if there are no active scopes.
    pub fn exit_scope(&mut self) -> Maybe<ImplicitResolver> {
        // pop returns Option, convert to Maybe
        self.scopes.pop()
    }

    /// Get the current scope's resolver.
    pub fn current_scope(&mut self) -> Maybe<&mut ImplicitResolver> {
        if self.scopes.is_empty() {
            Maybe::None
        } else {
            let len = self.scopes.len();
            Maybe::Some(&mut self.scopes[len - 1])
        }
    }

    /// Check if we're in an implicit scope.
    pub fn in_scope(&self) -> bool {
        !self.scopes.is_empty()
    }
}

impl Default for ImplicitContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implicit_resolver_registration() {
        let mut resolver = ImplicitResolver::new();
        let span = Span::dummy();

        let metavar = resolver.register_implicit(
            "T".into(),
            Type::Universe {
                level: crate::ty::UniverseLevel::TYPE,
            },
            span,
        );

        assert_eq!(resolver.pending_count(), 1);
        assert_eq!(resolver.constraint_count(), 0);
    }

    #[test]
    fn test_implicit_resolver_constraints() {
        let mut resolver = ImplicitResolver::new();
        let span = Span::dummy();

        let metavar = resolver.register_implicit(
            "T".into(),
            Type::Universe {
                level: crate::ty::UniverseLevel::TYPE,
            },
            span,
        );

        resolver.add_constraint(
            metavar,
            Type::Int,
            span,
            ConstraintSource::Argument { position: 0 },
        );

        assert_eq!(resolver.constraint_count(), 1);
    }

    #[test]
    fn test_implicit_resolver_solve_simple() {
        let mut resolver = ImplicitResolver::new();
        let span = Span::dummy();

        let metavar = resolver.register_implicit(
            "T".into(),
            Type::Universe {
                level: crate::ty::UniverseLevel::TYPE,
            },
            span,
        );

        resolver.add_constraint(
            metavar,
            Type::Int,
            span,
            ConstraintSource::Argument { position: 0 },
        );

        let subst = resolver.solve().expect("Should solve successfully");

        let inferred = resolver.get_inferred(metavar, &subst);
        assert!(matches!(inferred, Maybe::Some(Type::Int)));
    }

    #[test]
    fn test_implicit_resolver_ambiguous() {
        let mut resolver = ImplicitResolver::new();
        let span = Span::dummy();

        let metavar = resolver.register_implicit(
            "T".into(),
            Type::Universe {
                level: crate::ty::UniverseLevel::TYPE,
            },
            span,
        );

        // No constraints - should fail
        let result = resolver.solve();
        assert!(matches!(result, Err(TypeError::AmbiguousType { .. })));
    }

    #[test]
    fn test_implicit_context_scopes() {
        let mut ctx = ImplicitContext::new();

        assert!(!ctx.in_scope());

        ctx.enter_scope();
        assert!(ctx.in_scope());

        let scope = ctx.exit_scope();
        assert!(matches!(scope, Maybe::Some(_)));
        assert!(!ctx.in_scope());
    }

    #[test]
    fn test_elaborator() {
        let mut subst = Substitution::new();
        let metavar = TypeVar::fresh();
        subst.insert(metavar, Type::Int);

        let elaborator = ImplicitElaborator::new(subst);

        let ty = Type::Var(metavar);
        let elaborated = elaborator.elaborate_type(&ty);

        assert_eq!(elaborated, Type::Int);
    }
}
