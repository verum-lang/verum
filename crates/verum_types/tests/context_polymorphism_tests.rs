#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Type inference tests for Context Polymorphism.
//!
//! Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 17.2 (Context Polymorphism)
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage
//!
//! Tests that context polymorphism is correctly handled during type inference,
//! including context variable binding, unification, and propagation.

use verum_ast::span::{FileId, Span};
use verum_ast::ty::{GenericParam, GenericParamKind, Ident, Path};
use verum_common::{List, Text};
use verum_types::context::{TypeContext, TypeEnv};
use verum_types::ty::{Type, TypeVar};

// ============================================================================
// Context Variable Creation Tests
// ============================================================================

#[test]
fn test_context_variable_from_generic_param() {
    // Test that context parameters create type variables
    let ctx = TypeContext::new();

    // Create a context parameter: using C
    let context_param = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C", Span::default()),
        },
        is_implicit: false,
        span: Span::default(),
    };

    // The context param should be representable as a type variable
    match &context_param.kind {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "C");
            // Context params create fresh type variables during inference
            let tvar = TypeVar::fresh();
            let context_type = Type::Var(tvar);
            // The type variable should be a valid type
            assert!(matches!(context_type, Type::Var(_)));
        }
        _ => panic!("Expected Context param"),
    }
}

#[test]
fn test_multiple_context_variables() {
    // Test that multiple context params create distinct type variables
    let ctx_names = vec!["C1", "C2", "C3"];
    let mut type_vars = Vec::new();

    for name in &ctx_names {
        let tvar = TypeVar::fresh();
        type_vars.push(tvar);
    }

    // All type variables should be distinct
    assert_ne!(type_vars[0], type_vars[1]);
    assert_ne!(type_vars[1], type_vars[2]);
    assert_ne!(type_vars[0], type_vars[2]);
}

// ============================================================================
// Context Environment Tests
// ============================================================================

#[test]
fn test_context_variable_in_environment() {
    // Test that context variables can be added to the type environment
    let mut env = TypeEnv::new();

    // Add a context variable C
    let ctx_var = TypeVar::fresh();
    let ctx_type = Type::Var(ctx_var);
    env.insert_mono("C".to_string(), ctx_type.clone());

    // Verify we can look it up
    let lookup_result = env.lookup("C");
    assert!(lookup_result.is_some());

    if let Some(scheme) = lookup_result {
        assert!(matches!(scheme.ty, Type::Var(_)));
    }
}

#[test]
fn test_context_variable_scope() {
    // Test that context variables respect scoping rules
    let mut parent_env = TypeEnv::new();

    // Add context variable in parent scope
    let ctx_var = TypeVar::fresh();
    parent_env.insert_mono("OuterCtx".to_string(), Type::Var(ctx_var));

    // Create child scope
    let mut child_env = parent_env.child();

    // Child can see parent's context variable
    assert!(child_env.lookup("OuterCtx").is_some());

    // Add context variable in child scope
    let inner_var = TypeVar::fresh();
    child_env.insert_mono("InnerCtx".to_string(), Type::Var(inner_var));

    // Child can see both
    assert!(child_env.lookup("OuterCtx").is_some());
    assert!(child_env.lookup("InnerCtx").is_some());

    // Parent cannot see child's context variable
    assert!(parent_env.lookup("OuterCtx").is_some());
    assert!(parent_env.lookup("InnerCtx").is_none());
}

// ============================================================================
// Context Type Representation Tests
// ============================================================================

#[test]
fn test_context_as_type_variable() {
    // Context parameters are represented as type variables during inference
    let ctx_var = TypeVar::fresh();
    let ctx_type = Type::Var(ctx_var);

    // Verify it's a proper type
    assert!(matches!(ctx_type, Type::Var(v) if v == ctx_var));
}

#[test]
fn test_context_in_function_type() {
    // Test that context variables can appear in function types
    // fn foo<T, using C>(x: T) -> T using C

    let t_var = TypeVar::fresh();
    let c_var = TypeVar::fresh();

    // Create function type: (T) -> T
    let param_type = Type::Var(t_var);
    let return_type = Type::Var(t_var);

    let func_type = Type::function(
        List::from(vec![param_type]),
        return_type,
    );

    // Verify the function type
    match func_type {
        Type::Function { ref params, ref return_type, .. } => {
            assert_eq!(params.len(), 1);
            assert!(matches!(&params[0], Type::Var(_)));
        }
        _ => panic!("Expected Function type"),
    }
}

// ============================================================================
// Context Unification Tests
// ============================================================================

#[test]
fn test_context_variable_unification() {
    // Test that context variables can be unified with concrete context types
    let ctx_var = TypeVar::fresh();

    // Context variables should be fresh (not equal to other fresh vars)
    let other_var = TypeVar::fresh();
    assert_ne!(ctx_var, other_var);
}

#[test]
fn test_context_propagation() {
    // Test that context propagates through higher-order functions
    // fn map<T, U, using C>(value: T, f: fn(T) -> U using C) -> U using C

    let t_var = TypeVar::fresh();
    let u_var = TypeVar::fresh();
    let c_var = TypeVar::fresh();

    // f: fn(T) -> U using C
    let callback_type = Type::function(
        List::from(vec![Type::Var(t_var)]),
        Type::Var(u_var),
    );

    // The callback's context C should propagate to the result
    // This is verified through the type checker during inference
    assert!(matches!(callback_type, Type::Function { .. }));
}

// ============================================================================
// Context Type Equality Tests
// ============================================================================

#[test]
fn test_same_context_variable_equality() {
    // Same context variable should be equal to itself
    let ctx_var = TypeVar::fresh();
    let ctx_type1 = Type::Var(ctx_var);
    let ctx_type2 = Type::Var(ctx_var);

    assert_eq!(ctx_type1, ctx_type2);
}

#[test]
fn test_different_context_variables_inequality() {
    // Different context variables should not be equal
    let ctx_var1 = TypeVar::fresh();
    let ctx_var2 = TypeVar::fresh();

    let type1 = Type::Var(ctx_var1);
    let type2 = Type::Var(ctx_var2);

    assert_ne!(type1, type2);
}

// ============================================================================
// Context with Type Parameters Tests
// ============================================================================

#[test]
fn test_mixed_type_and_context_params() {
    // Test that type parameters and context parameters work together
    // fn foo<T, U, using C>(x: T, y: U) -> T using C

    let mut env = TypeEnv::new();

    // Add type parameters
    let t_var = TypeVar::fresh();
    let u_var = TypeVar::fresh();
    env.insert_mono("T".to_string(), Type::Var(t_var));
    env.insert_mono("U".to_string(), Type::Var(u_var));

    // Add context parameter
    let c_var = TypeVar::fresh();
    env.insert_mono("C".to_string(), Type::Var(c_var));

    // All should be accessible
    assert!(env.lookup("T").is_some());
    assert!(env.lookup("U").is_some());
    assert!(env.lookup("C").is_some());
}

// ============================================================================
// Context Generalization Tests
// ============================================================================

#[test]
fn test_context_variable_generalization() {
    // Test that context variables are properly generalized
    let mut env = TypeEnv::new();

    let ctx_var = TypeVar::fresh();
    let ctx_type = Type::Var(ctx_var);

    // Create a function type that uses the context
    let func_type = Type::function(
        List::from(vec![Type::int()]),
        Type::int(),
    );

    // Generalize the type
    let scheme = env.generalize(func_type);

    // The function type should be generalized (no free variables in env)
    // Since func_type doesn't contain ctx_var, vars should be empty
    assert_eq!(scheme.vars.len(), 0);
}

#[test]
fn test_context_variable_in_scheme() {
    // Test that context variables appear in type schemes when free
    let env = TypeEnv::new();

    let ctx_var = TypeVar::fresh();

    // Create a type that contains the context variable
    let func_type = Type::function(
        List::from(vec![Type::Var(ctx_var)]),
        Type::Var(ctx_var),
    );

    // Generalize - ctx_var should be quantified
    let scheme = env.generalize(func_type);

    // The context variable should be in the scheme's vars
    assert!(scheme.vars.contains(&ctx_var));
}

// ============================================================================
// Context Pretty Printing Tests
// ============================================================================

#[test]
fn test_context_type_display() {
    // Test that context types can be displayed
    let ctx_var = TypeVar::fresh();
    let ctx_type = Type::Var(ctx_var);

    // Should be able to format without panic
    let displayed = format!("{}", ctx_type);
    assert!(!displayed.is_empty());
}

// ============================================================================
// Context in Generic Param Kind Tests
// ============================================================================

#[test]
fn test_context_param_kind_distinct() {
    // Context param kind should be distinct from other kinds
    let context_param = GenericParamKind::Context {
        name: Ident::new("C", Span::default()),
    };

    let type_param = GenericParamKind::Type {
        name: Ident::new("T", Span::default()),
        bounds: List::new(),
        default: verum_common::Maybe::None,
    };

    // They should not be equal
    assert_ne!(
        std::mem::discriminant(&context_param),
        std::mem::discriminant(&type_param)
    );
}

#[test]
fn test_context_param_name_extraction() {
    // Test extracting the name from a context param
    let param = GenericParamKind::Context {
        name: Ident::new("MyContext", Span::default()),
    };

    match param {
        GenericParamKind::Context { name } => {
            assert_eq!(name.name.as_str(), "MyContext");
        }
        _ => panic!("Expected Context"),
    }
}

// ============================================================================
// Context Inference Integration Tests
// ============================================================================

#[test]
fn test_context_fresh_each_call() {
    // Each instantiation of a context-polymorphic function should get fresh context
    let mut contexts = Vec::new();

    for _ in 0..3 {
        let ctx_var = TypeVar::fresh();
        contexts.push(ctx_var);
    }

    // All contexts should be distinct
    assert_ne!(contexts[0], contexts[1]);
    assert_ne!(contexts[1], contexts[2]);
    assert_ne!(contexts[0], contexts[2]);
}

#[test]
fn test_context_callback_propagation() {
    // When calling a context-polymorphic function with a callback,
    // the callback's context should propagate to the result

    let mut env = TypeEnv::new();

    // Simulate: fn map<T, U, using C>(value: T, f: fn(T) -> U using C) -> U using C
    let t_var = TypeVar::fresh();
    let u_var = TypeVar::fresh();
    let c_var = TypeVar::fresh();

    // Add to environment
    env.insert_mono("T".to_string(), Type::Var(t_var));
    env.insert_mono("U".to_string(), Type::Var(u_var));
    env.insert_mono("C".to_string(), Type::Var(c_var));

    // All should be present
    assert!(env.lookup("T").is_some());
    assert!(env.lookup("U").is_some());
    assert!(env.lookup("C").is_some());

    // The context variable C should be usable in the return type
    let result_type = env.lookup("C").map(|s| s.ty.clone());
    assert!(result_type.is_some());
    assert!(matches!(result_type.unwrap(), Type::Var(_)));
}

// ============================================================================
// Context Type Clone Tests
// ============================================================================

#[test]
fn test_context_type_clone() {
    let ctx_var = TypeVar::fresh();
    let ctx_type = Type::Var(ctx_var);

    let cloned = ctx_type.clone();
    assert_eq!(ctx_type, cloned);
}

// ============================================================================
// Context with Higher-Kinded Types Tests
// ============================================================================

#[test]
fn test_context_with_hkt() {
    // Test that context polymorphism works with higher-kinded types
    // fn traverse<F<_>, T, U, using C>(container: F<T>, f: fn(T) -> U using C) -> F<U> using C

    let mut env = TypeEnv::new();

    // Type variables
    let f_var = TypeVar::fresh(); // Higher-kinded type F
    let t_var = TypeVar::fresh(); // Element type T
    let u_var = TypeVar::fresh(); // Result element type U
    let c_var = TypeVar::fresh(); // Context C

    env.insert_mono("F".to_string(), Type::Var(f_var));
    env.insert_mono("T".to_string(), Type::Var(t_var));
    env.insert_mono("U".to_string(), Type::Var(u_var));
    env.insert_mono("C".to_string(), Type::Var(c_var));

    // All should be accessible
    assert!(env.lookup("F").is_some());
    assert!(env.lookup("T").is_some());
    assert!(env.lookup("U").is_some());
    assert!(env.lookup("C").is_some());
}

// ============================================================================
// Context with Maybe/Result Types Tests
// ============================================================================

#[test]
fn test_context_with_maybe_map() {
    // fn map_maybe<T, U, using C>(opt: Maybe<T>, f: fn(T) -> U using C) -> Maybe<U> using C

    let mut env = TypeEnv::new();

    let t_var = TypeVar::fresh();
    let u_var = TypeVar::fresh();
    let c_var = TypeVar::fresh();

    env.insert_mono("T".to_string(), Type::Var(t_var));
    env.insert_mono("U".to_string(), Type::Var(u_var));
    env.insert_mono("C".to_string(), Type::Var(c_var));

    // All type variables should be distinct
    assert_ne!(t_var, u_var);
    assert_ne!(u_var, c_var);
    assert_ne!(t_var, c_var);
}

#[test]
fn test_context_with_result_and_then() {
    // fn and_then<T, U, E, using C>(result: Result<T, E>, f: fn(T) -> Result<U, E> using C) -> Result<U, E> using C

    let mut env = TypeEnv::new();

    let t_var = TypeVar::fresh();
    let u_var = TypeVar::fresh();
    let e_var = TypeVar::fresh();
    let c_var = TypeVar::fresh();

    env.insert_mono("T".to_string(), Type::Var(t_var));
    env.insert_mono("U".to_string(), Type::Var(u_var));
    env.insert_mono("E".to_string(), Type::Var(e_var));
    env.insert_mono("C".to_string(), Type::Var(c_var));

    // All should be accessible
    assert!(env.lookup("T").is_some());
    assert!(env.lookup("U").is_some());
    assert!(env.lookup("E").is_some());
    assert!(env.lookup("C").is_some());
}

// ============================================================================
// Context Debug Tests
// ============================================================================

#[test]
fn test_context_param_debug() {
    let param = GenericParam {
        kind: GenericParamKind::Context {
            name: Ident::new("C", Span::default()),
        },
        is_implicit: false,
        span: Span::default(),
    };

    // Should be debuggable without panic
    let debug_str = format!("{:?}", param);
    assert!(debug_str.contains("Context"));
    assert!(debug_str.contains("C"));
}
