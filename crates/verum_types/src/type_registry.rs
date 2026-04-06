//! Type Registry for AST → Type mapping
//!
//! This module provides a registry that maps AST nodes to their inferred types,
//! enabling type information propagation from the type checker to code generation.
//!
//! # Architecture
//!
//! The TypeRegistry solves the critical problem of passing type information from
//! verum_types to verum_codegen without modifying AST structures:
//!
//! ```text
//! Parser → AST (verum_ast::Type) → TypeChecker → TypeRegistry → Codegen
//!                                        ↓
//!                                  verum_types::Type
//! ```
//!
//! # Usage
//!
//! During type checking:
//! ```ignore
//! // Register expression types
//! registry.register_expr(expr.span, inferred_type);
//!
//! // Register variable types
//! registry.register_var(param.span, param.name.clone(), param_type);
//! ```
//!
//! During code generation:
//! ```ignore
//! // Lookup expression type
//! if let Some(ty) = registry.lookup_expr(expr.span) {
//!     // Use inferred type
//! }
//!
//! // Lookup variable type
//! if let Some(ty) = registry.lookup_var(param.span, &param.name) {
//!     // Use inferred type
//! }
//! ```

use std::collections::HashMap;
use verum_ast::span::Span;
use verum_common::{Maybe, Text};

use crate::ty::Type;

/// Maximum entries in expr_types before clearing
/// MEMORY FIX: For 10K LOC files this prevents 2.4-10 MB accumulation
const MAX_EXPR_TYPES: usize = 100_000;

/// Maximum entries in var_types before clearing
const MAX_VAR_TYPES: usize = 50_000;

/// Maximum entries in func_return_types before clearing
const MAX_FUNC_RETURN_TYPES: usize = 10_000;

/// Registry mapping AST nodes to inferred types
///
/// This structure enables type information to flow from the type checker
/// to code generation without modifying the AST.
///
/// # Performance
///
/// - Lookups: O(1) average case
/// - Memory: ~16 bytes per registered type (Span + pointer)
/// - Build time: < 1ms for typical modules
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Expression types indexed by span
    ///
    /// Maps expression spans to their inferred types. This handles all
    /// expression-level type information including literals, variables,
    /// function calls, and compound expressions.
    expr_types: HashMap<Span, Type>,

    /// Variable types indexed by (span, name)
    ///
    /// Maps variable declarations (parameters, locals) to their types.
    /// Uses both span and name for precise identification, handling cases
    /// where variables are shadowed or reused.
    var_types: HashMap<(Span, Text), Type>,

    /// Function return types indexed by span
    ///
    /// Maps function declaration spans to their return types. This enables
    /// codegen to determine the correct LLVM return type without re-analyzing
    /// the AST type annotation.
    func_return_types: HashMap<Span, Type>,
}

impl TypeRegistry {
    /// Create a new empty type registry
    pub fn new() -> Self {
        Self {
            expr_types: HashMap::new(),
            var_types: HashMap::new(),
            func_return_types: HashMap::new(),
        }
    }

    /// Register the type of an expression
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the expression in the source code
    /// * `ty` - The inferred type of the expression
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During type checking of: x + 1
    /// registry.register_expr(plus_expr.span, Type::Int);
    /// ```
    pub fn register_expr(&mut self, span: Span, ty: Type) {
        // MEMORY FIX: Clear if exceeding limit to prevent 2-10GB accumulation
        if self.expr_types.len() >= MAX_EXPR_TYPES {
            self.expr_types.clear();
        }
        self.expr_types.insert(span, ty);
    }

    /// Register the type of a variable (parameter or local)
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the variable declaration
    /// * `name` - The name of the variable
    /// * `ty` - The inferred type of the variable
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During type checking of: fn foo(x: Int, y: Float) -> Bool
    /// registry.register_var(x_param.span, "x".into(), Type::Int);
    /// registry.register_var(y_param.span, "y".into(), Type::Float);
    /// ```
    pub fn register_var(&mut self, span: Span, name: Text, ty: Type) {
        // MEMORY FIX: Clear if exceeding limit
        if self.var_types.len() >= MAX_VAR_TYPES {
            self.var_types.clear();
        }
        self.var_types.insert((span, name), ty);
    }

    /// Register the return type of a function
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the function declaration
    /// * `ty` - The inferred return type
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During type checking of: fn foo() -> Int { 42 }
    /// registry.register_func_return(func.span, Type::Int);
    /// ```
    pub fn register_func_return(&mut self, span: Span, ty: Type) {
        // MEMORY FIX: Clear if exceeding limit
        if self.func_return_types.len() >= MAX_FUNC_RETURN_TYPES {
            self.func_return_types.clear();
        }
        self.func_return_types.insert(span, ty);
    }

    /// Lookup the type of an expression
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the expression to lookup
    ///
    /// # Returns
    ///
    /// The inferred type if registered, or `Maybe::None` if not found
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During codegen of: x + 1
    /// if let Some(ty) = registry.lookup_expr(plus_expr.span) {
    ///     let llvm_ty = translate_type(ty);
    ///     // Generate code with correct type
    /// }
    /// ```
    pub fn lookup_expr(&self, span: Span) -> Maybe<&Type> {
        self.expr_types.get(&span)
    }

    /// Lookup the type of a variable
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the variable declaration
    /// * `name` - The name of the variable
    ///
    /// # Returns
    ///
    /// The inferred type if registered, or `Maybe::None` if not found
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During codegen of function parameter x
    /// if let Some(ty) = registry.lookup_var(x_param.span, "x") {
    ///     let llvm_ty = translate_type(ty);
    ///     // Generate parameter with correct type
    /// }
    /// ```
    pub fn lookup_var(&self, span: Span, name: &Text) -> Maybe<&Type> {
        self.var_types.get(&(span, name.clone()))
    }

    /// Lookup the return type of a function
    ///
    /// # Arguments
    ///
    /// * `span` - The span of the function declaration
    ///
    /// # Returns
    ///
    /// The inferred return type if registered, or `Maybe::None` if not found
    ///
    /// # Example
    ///
    /// ```ignore
    /// // During codegen of function
    /// if let Some(ret_ty) = registry.lookup_func_return(func.span) {
    ///     let llvm_ret_ty = translate_type(ret_ty);
    ///     // Create function with correct return type
    /// }
    /// ```
    pub fn lookup_func_return(&self, span: Span) -> Maybe<&Type> {
        self.func_return_types.get(&span)
    }

    /// Get statistics about the registry
    ///
    /// Returns (expr_count, var_count, func_count)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (exprs, vars, funcs) = registry.stats();
    /// println!("Registry: {} exprs, {} vars, {} funcs", exprs, vars, funcs);
    /// ```
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.expr_types.len(),
            self.var_types.len(),
            self.func_return_types.len(),
        )
    }

    /// Clear all registered types
    ///
    /// This is useful for testing or when processing multiple modules
    /// that should not share type information.
    pub fn clear(&mut self) {
        self.expr_types.clear();
        self.var_types.clear();
        self.func_return_types.clear();
    }

    /// Apply a unifier's substitution to all types in the registry
    ///
    /// This resolves type variables to their concrete types after type checking
    /// is complete. This is important for codegen which needs fully resolved types.
    ///
    /// # Arguments
    ///
    /// * `unifier` - The unifier containing the accumulated substitutions
    pub fn apply_substitution(&mut self, unifier: &crate::unify::Unifier) {
        // Apply substitution to all expression types
        for ty in self.expr_types.values_mut() {
            *ty = unifier.apply(ty);
        }

        // Apply substitution to all variable types
        for ty in self.var_types.values_mut() {
            *ty = unifier.apply(ty);
        }

        // Apply substitution to all function return types
        for ty in self.func_return_types.values_mut() {
            *ty = unifier.apply(ty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::Span;
    use verum_common::FileId;

    #[test]
    fn test_basic_registration_and_lookup() {
        let mut registry = TypeRegistry::new();

        let span1 = Span::new(0, 10, FileId::dummy());
        let span2 = Span::new(10, 20, FileId::dummy());

        // Register expression types
        registry.register_expr(span1, Type::Int);
        registry.register_expr(span2, Type::Bool);

        // Lookup
        assert!(matches!(
            registry.lookup_expr(span1),
            Maybe::Some(&Type::Int)
        ));
        assert!(matches!(
            registry.lookup_expr(span2),
            Maybe::Some(&Type::Bool)
        ));

        // Not found
        let span3 = Span::new(20, 30, FileId::dummy());
        assert!(matches!(registry.lookup_expr(span3), Maybe::None));
    }

    #[test]
    fn test_variable_registration() {
        let mut registry = TypeRegistry::new();

        let span = Span::new(0, 10, FileId::dummy());
        let name = Text::from("x");

        registry.register_var(span, name.clone(), Type::Float);

        assert!(matches!(
            registry.lookup_var(span, &name),
            Maybe::Some(&Type::Float)
        ));

        // Different name at same span
        let other_name = Text::from("y");
        assert!(matches!(
            registry.lookup_var(span, &other_name),
            Maybe::None
        ));
    }

    #[test]
    fn test_function_return_types() {
        let mut registry = TypeRegistry::new();

        let span = Span::new(0, 100, FileId::dummy());
        registry.register_func_return(span, Type::Unit);

        assert!(matches!(
            registry.lookup_func_return(span),
            Maybe::Some(&Type::Unit)
        ));
    }

    #[test]
    fn test_stats() {
        let mut registry = TypeRegistry::new();

        let span1 = Span::new(0, 10, FileId::dummy());
        let span2 = Span::new(10, 20, FileId::dummy());

        registry.register_expr(span1, Type::Int);
        registry.register_var(span2, Text::from("x"), Type::Bool);
        registry.register_func_return(span1, Type::Unit);

        let (exprs, vars, funcs) = registry.stats();
        assert_eq!(exprs, 1);
        assert_eq!(vars, 1);
        assert_eq!(funcs, 1);
    }

    #[test]
    fn test_clear() {
        let mut registry = TypeRegistry::new();

        let span = Span::new(0, 10, FileId::dummy());
        registry.register_expr(span, Type::Int);

        assert!(matches!(registry.lookup_expr(span), Maybe::Some(_)));

        registry.clear();

        assert!(matches!(registry.lookup_expr(span), Maybe::None));
        assert_eq!(registry.stats(), (0, 0, 0));
    }
}
