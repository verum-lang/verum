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
//!  ↓
//!  verum_types::Type
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
//!  // Use inferred type
//! }
//!
//! // Lookup variable type
//! if let Some(ty) = registry.lookup_var(param.span, &param.name) {
//!  // Use inferred type
//! }
//! ```

use verum_ast::span::Span;
use verum_common::{Map, Maybe, Shared, Text};

use crate::ty::Type;

#[derive(Debug, Clone)]
struct CallTarget {
    ty: Type,
    explicit_inputs: bool,
    declared_receiver: Option<Type>,
    returned_value: Option<Shared<crate::context::ReturnedValue>>,
    structural_receiver: Option<Type>,
}

/// Registry mapping AST nodes to inferred types
///
/// This structure enables type information to flow from the type checker
/// to code generation without modifying the AST.
///
/// These are compilation facts, not an evictable cache. Keep them until the
/// compilation releases the registry or explicitly clears it. Resource limits
/// must reject a compilation rather than silently erase facts needed by codegen.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Expression types indexed by span
    ///
    /// Maps expression spans to their inferred types. This handles all
    /// expression-level type information including literals, variables,
    /// function calls, and compound expressions.
    expr_types: Map<Span, Type>,
    /// Callable selected for an executed call, distinct from its result type.
    call_targets: Map<Span, CallTarget>,
    /// A source site consumes its receiver only if every resolution at that
    /// span agrees. Expanded expressions can share a source location.
    consuming_receivers: Map<Span, bool>,

    /// Variable types indexed by (span, name)
    ///
    /// Maps variable declarations (parameters, locals) to their types.
    /// Uses both span and name for precise identification, handling cases
    /// where variables are shadowed or reused.
    var_types: Map<(Span, Text), Type>,

    /// Function return types indexed by span
    ///
    /// Maps function declaration spans to their return types. This enables
    /// codegen to determine the correct LLVM return type without re-analyzing
    /// the AST type annotation.
    func_return_types: Map<Span, Type>,
}

impl TypeRegistry {
    /// Create a new empty type registry
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_receiver_kind(
        &mut self,
        span: Span,
        kind: Option<crate::core_metadata::ReceiverKind>,
    ) {
        if !span.is_empty() {
            let consumes = kind == Some(crate::core_metadata::ReceiverKind::SelfValue);
            self.consuming_receivers
                .entry(span)
                .and_modify(|previous| *previous &= consumes)
                .or_insert(consumes);
        }
    }

    pub(crate) fn consumes_receiver(&self, span: Span) -> bool {
        self.consuming_receivers.get(&span) == Some(&true)
    }

    pub fn register_call_target(&mut self, span: Span, ty: Type) {
        self.record_call_target(span, ty, false, None, None, None);
    }

    pub(crate) fn register_declared_static_target(
        &mut self,
        span: Span,
        ty: Type,
        scheme: &crate::TypeScheme,
    ) {
        // Unknown metadata, host calls, context providers and receiver values
        // cannot prove that the explicitly passed values are the only inputs.
        let explicit_inputs = scheme.callable_source.as_ref().is_some_and(|(_, source)| {
            crate::core_metadata::ReceiverKind::from_declaration(source)
                == crate::core_metadata::ReceiverKind::None
                && source.contexts.is_empty()
                && !source.is_unsafe
                && source.extern_abi.is_none()
                && !source.is_variadic
                && source.body.is_some()
        });
        self.record_call_target(span, ty, explicit_inputs, None, scheme.returned_value.clone(), None);
    }

    /// A selected source receiver is separate from the explicit argument list.
    /// Unknown dispatch and context inputs do not supply this authority.
    pub(crate) fn register_declared_method_target(
        &mut self,
        span: Span,
        ty: Type,
        receiver: Type,
        scheme: &crate::TypeScheme,
    ) {
        use verum_ast::FunctionParamKind;
        let declared_receiver = scheme.callable_source.as_ref().and_then(|(_, source)| {
            if !source.contexts.is_empty() || source.is_unsafe || source.extern_abi.is_some()
                || source.is_variadic || source.body.is_none() { return None; }
            if scheme.receiver_kind != Some(crate::core_metadata::ReceiverKind::from_declaration(source)) {
                return None;
            }
            let mutable = match &source.params.first()?.kind {
                FunctionParamKind::SelfRef => false,
                FunctionParamKind::SelfRefMut => true,
                _ => return None,
            };
            Some(Type::Reference { inner: Box::new(receiver), mutable })
        });
        self.record_call_target(span, ty, false, declared_receiver, scheme.returned_value.clone(), None);
    }

    /// The structural operation resolver, rather than a callable property,
    /// certifies that this selected operation reads a declared receiver.
    pub(crate) fn register_structural_call_target(&mut self, span: Span, ty: Type, receiver: Type) {
        self.record_call_target(span, ty, false, None, None, Some(receiver));
    }

    pub(crate) fn structural_call_target(&self, span: Span) -> Option<(&Type, &Type)> {
        let target = self.call_targets.get(&span)?;
        Some((&target.ty, target.structural_receiver.as_ref()?))
    }

    fn record_call_target(
        &mut self,
        span: Span,
        ty: Type,
        explicit_inputs: bool,
        declared_receiver: Option<Type>,
        returned_value: Option<Shared<crate::context::ReturnedValue>>,
        structural_receiver: Option<Type>,
    ) {
        let previous = self.call_targets.get(&span);
        let agreed = !span.is_empty() && explicit_inputs
            && previous.is_none_or(|previous| previous.explicit_inputs && previous.ty == ty);
        let same_rule = !span.is_empty() && (explicit_inputs || declared_receiver.is_some() || structural_receiver.is_some())
            && previous.is_none_or(|previous| previous.ty == ty
                && previous.explicit_inputs == explicit_inputs
                && previous.declared_receiver == declared_receiver
                && previous.returned_value == returned_value
                && previous.structural_receiver == structural_receiver);
        self.call_targets.insert(span, CallTarget {
            ty,
            explicit_inputs: agreed,
            declared_receiver: same_rule.then_some(declared_receiver).flatten(),
            returned_value: same_rule.then_some(returned_value).flatten(),
            structural_receiver: same_rule.then_some(structural_receiver).flatten(),
        });
    }

    pub fn lookup_call_target(&self, span: Span) -> Maybe<&Type> {
        self.call_targets.get(&span).map(|target| &target.ty)
    }

    pub(crate) fn explicit_input_target(&self, span: Span) -> Maybe<&Type> {
        let target = self.call_targets.get(&span)?;
        target.explicit_inputs.then_some(&target.ty)
    }

    pub(crate) fn declared_receiver(&self, span: Span) -> Option<&Type> {
        self.call_targets.get(&span)?.declared_receiver.as_ref()
    }

    pub(crate) fn returned_value(&self, span: Span) -> Option<&Shared<crate::context::ReturnedValue>> {
        self.call_targets.get(&span)?.returned_value.as_ref()
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
    ///  let llvm_ty = translate_type(ty);
    ///  // Generate code with correct type
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
    ///  let llvm_ty = translate_type(ty);
    ///  // Generate parameter with correct type
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
    ///  let llvm_ret_ty = translate_type(ret_ty);
    ///  // Create function with correct return type
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
        self.call_targets.clear();
        self.consuming_receivers.clear();
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
        for target in self.call_targets.values_mut() { target.ty = unifier.apply(&target.ty); }
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
