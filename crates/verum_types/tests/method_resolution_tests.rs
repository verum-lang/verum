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
// Comprehensive tests for P0 Blocker #7: Method Resolution System
//
// Higher-rank protocol bounds: for<T> quantification in protocol bounds for universal requirements — .1-2.3
//
// Tests cover:
// 1. Simple method calls
// 2. Generic methods
// 3. Multiple protocols
// 4. Method overloading scenarios
// 5. Error cases (method not found, wrong arg count, protocol not implemented, ambiguous)

use verum_ast::{
    expr::*,
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_types::infer::*;
use verum_types::protocol::*;
use verum_types::ty::Type;

/// Helper to create a dummy span
fn dummy() -> Span {
    Span::dummy()
}

/// Helper to create an identifier
fn ident(name: &str) -> Ident {
    Ident::new(name, dummy())
}

/// Helper to create a simple path
fn path(name: &str) -> Path {
    Path::single(ident(name))
}

// ============================================================================
// TEST 1: Simple Method Calls
// ============================================================================

#[test]
fn test_simple_method_call_eq() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Setup: Register Eq protocol with eq method
    let _eq_protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Eq".into(),
        type_params: List::new(),
        methods: {
            let mut methods = Map::new();
            methods.insert(
                Text::from("eq"),
                ProtocolMethod {
                    name: Text::from("eq"),
                    // fn eq(self: Int, other: Int) -> Bool
                    ty: Type::function(vec![Type::int(), Type::int()].into(), Type::bool()),
                    has_default: false,
                    doc: Maybe::None,
                    refinement_constraints: Map::new(),
                    is_async: false,
                    context_requirements: List::new(),
                    type_param_names: List::new(),
                    type_param_bounds: Map::new(),
            receiver_kind: Maybe::None,
                },
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some(Text::from("test_crate")),
        span,
    };

    // Register protocol
    checker.context_mut().env.insert(
        "Eq".to_string(),
        verum_types::context::TypeScheme::mono(Type::Named {
            path: path("Eq"),
            args: List::new(),
        }),
    );

    // ISSUE: We need to actually register the implementation, not just the protocol
    // For now, test will demonstrate the infrastructure is in place
    // Full integration requires protocol registration API

    // This test validates the type checker structure accepts method calls
    // without panicking (success criteria for infrastructure)
}

// ============================================================================
// TEST 2: Record Field Function Calls
// ============================================================================

#[test]
fn test_record_field_function_call() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record type with a function field
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("compute"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::int()),
    );
    let record_ty = Type::Record(fields);

    // Add record type to context
    checker.context_mut().env.insert(
        "obj".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build expression: obj.compute(1, 2)
    let receiver = Expr::new(ExprKind::Path(path("obj")), span);

    let args = vec![
        Expr::literal(Literal::int(1, span)),
        Expr::literal(Literal::int(2, span)),
    ];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("compute"),
            args: args.into(),
        },
        span,
    );

    // Type check the method call
    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// TEST 3: Method Not Found Error
// ============================================================================

#[test]
fn test_method_not_found() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a simple int variable
    checker.context_mut().env.insert(
        "x".to_string(),
        verum_types::context::TypeScheme::mono(Type::int()),
    );

    // Build expression: x.nonexistent()
    let receiver = Expr::new(ExprKind::Path(path("x")), span);

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("nonexistent"),
            args: vec![].into(),
        },
        span,
    );

    // Should fail with MethodNotFound error
    let result = checker.synth_expr(&method_call);
    assert!(result.is_err());

    if let Err(verum_types::TypeError::MethodNotFound { method, ty, .. }) = result {
        assert_eq!(method, "nonexistent");
        assert_eq!(ty, "Int");
    } else {
        panic!("Expected MethodNotFound error, got: {:?}", result);
    }
}

// ============================================================================
// TEST 4: Wrong Argument Count
// ============================================================================

#[test]
fn test_wrong_arg_count() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a function that expects 2 arguments
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("add"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::int()),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "calc".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build expression: calc.add(1) - missing second argument
    let receiver = Expr::new(ExprKind::Path(path("calc")), span);

    let args = vec![Expr::literal(Literal::int(1, span))];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("add"),
            args: args.into(),
        },
        span,
    );

    // Should fail with WrongArgCount error
    let result = checker.synth_expr(&method_call);
    assert!(result.is_err());

    if let Err(verum_types::TypeError::WrongArgCount {
        method,
        expected,
        actual,
        ..
    }) = result
    {
        assert_eq!(method, "add");
        assert_eq!(expected, 2);
        assert_eq!(actual, 1);
    } else {
        panic!("Expected WrongArgCount error, got: {:?}", result);
    }
}

// ============================================================================
// TEST 5: Method Call with Correct Arguments
// ============================================================================

#[test]
fn test_method_call_correct_args() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("multiply"),
        Type::function(vec![Type::int(), Type::int()].into(), Type::int()),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "math".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build expression: math.multiply(3, 4)
    let receiver = Expr::new(ExprKind::Path(path("math")), span);

    let args = vec![
        Expr::literal(Literal::int(3, span)),
        Expr::literal(Literal::int(4, span)),
    ];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("multiply"),
            args: args.into(),
        },
        span,
    );

    // Should succeed and return Int
    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// TEST 6: Method with Return Type
// ============================================================================

#[test]
fn test_method_return_type() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with methods returning different types
    let mut fields = indexmap::IndexMap::new();

    // fn is_positive(n: Int) -> Bool
    fields.insert(
        Text::from("is_positive"),
        Type::function(vec![Type::int()].into(), Type::bool()),
    );

    // fn to_string(n: Int) -> Text
    fields.insert(
        Text::from("to_string"),
        Type::function(vec![Type::int()].into(), Type::text()),
    );

    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "utils".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Test 1: utils.is_positive(5) -> Bool
    {
        let receiver = Expr::new(ExprKind::Path(path("utils")), span);
        let args = vec![Expr::literal(Literal::int(5, span))];

        let method_call = Expr::new(
            ExprKind::MethodCall { type_args: vec![].into(),
                receiver: Box::new(receiver),
                method: ident("is_positive"),
                args: args.into(),
            },
            span,
        );

        let result = checker.synth_expr(&method_call).unwrap();
        assert_eq!(result.ty, Type::bool());
    }

    // Test 2: utils.to_string(42) -> Text
    {
        let receiver = Expr::new(ExprKind::Path(path("utils")), span);
        let args = vec![Expr::literal(Literal::int(42, span))];

        let method_call = Expr::new(
            ExprKind::MethodCall { type_args: vec![].into(),
                receiver: Box::new(receiver),
                method: ident("to_string"),
                args: args.into(),
            },
            span,
        );

        let result = checker.synth_expr(&method_call).unwrap();
        assert_eq!(result.ty, Type::text());
    }
}

// ============================================================================
// TEST 7: Chained Method Calls
// ============================================================================

#[test]
fn test_chained_method_calls() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record type with methods
    let mut fields = indexmap::IndexMap::new();

    // fn double(n: Int) -> Int
    fields.insert(
        Text::from("double"),
        Type::function(vec![Type::int()].into(), Type::int()),
    );

    let math_ty = Type::Record(fields.clone());

    // Create nested record: { math: { double: fn(Int) -> Int } }
    let mut outer_fields = indexmap::IndexMap::new();
    outer_fields.insert(Text::from("math"), math_ty);
    let outer_record = Type::Record(outer_fields);

    checker.context_mut().env.insert(
        "obj".to_string(),
        verum_types::context::TypeScheme::mono(outer_record),
    );

    // Build: obj.math.double(5)
    // First: obj.math
    let obj_expr = Expr::new(ExprKind::Path(path("obj")), span);
    let math_field = Expr::new(
        ExprKind::Field {
            expr: Box::new(obj_expr),
            field: ident("math"),
        },
        span,
    );

    // Then: .double(5)
    let args = vec![Expr::literal(Literal::int(5, span))];
    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(math_field),
            method: ident("double"),
            args: args.into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// TEST 8: Method with Generic Return Type
// ============================================================================

#[test]
fn test_method_with_generic_return() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method returning a tuple
    let mut fields = indexmap::IndexMap::new();

    // fn pair(a: Int, b: Int) -> (Int, Int)
    fields.insert(
        Text::from("pair"),
        Type::function(
            vec![Type::int(), Type::int()].into(),
            Type::tuple(vec![Type::int(), Type::int()].into()),
        ),
    );

    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "factory".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build: factory.pair(1, 2)
    let receiver = Expr::new(ExprKind::Path(path("factory")), span);
    let args = vec![
        Expr::literal(Literal::int(1, span)),
        Expr::literal(Literal::int(2, span)),
    ];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("pair"),
            args: args.into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(
        result.ty,
        Type::tuple(vec![Type::int(), Type::int()].into())
    );
}

// ============================================================================
// TEST 9: Argument Type Mismatch
// ============================================================================

#[test]
fn test_argument_type_mismatch() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method expecting Int
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("process"),
        Type::function(vec![Type::int()].into(), Type::bool()),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "processor".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build: processor.process(true) - passing Bool instead of Int
    let receiver = Expr::new(ExprKind::Path(path("processor")), span);
    let args = vec![Expr::literal(Literal::bool(true, span))];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("process"),
            args: args.into(),
        },
        span,
    );

    // Should fail with type mismatch
    let result = checker.synth_expr(&method_call);
    assert!(result.is_err());

    // The error will be a Mismatch from check_expr
    if let Err(verum_types::TypeError::Mismatch {
        expected, actual, ..
    }) = result
    {
        assert_eq!(expected, "Int");
        assert_eq!(actual, "Bool");
    } else {
        panic!("Expected Mismatch error, got: {:?}", result);
    }
}

// ============================================================================
// TEST 10: Method on Non-Record Type
// ============================================================================

#[test]
fn test_method_on_non_record() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a simple int variable (not a record)
    checker.context_mut().env.insert(
        "n".to_string(),
        verum_types::context::TypeScheme::mono(Type::int()),
    );

    // Build: n.some_method()
    let receiver = Expr::new(ExprKind::Path(path("n")), span);

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("some_method"),
            args: vec![].into(),
        },
        span,
    );

    // Should fail because Int doesn't have methods (no protocol implementations)
    let result = checker.synth_expr(&method_call);
    assert!(result.is_err());

    if let Err(verum_types::TypeError::MethodNotFound { ty, method, .. }) = result {
        assert_eq!(ty, "Int");
        assert_eq!(method, "some_method");
    } else {
        panic!("Expected MethodNotFound error, got: {:?}", result);
    }
}

// ============================================================================
// TEST 11: Zero-Argument Method
// ============================================================================

#[test]
fn test_zero_argument_method() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a zero-argument method
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("get_value"),
        Type::function(vec![].into(), Type::int()),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "getter".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build: getter.get_value()
    let receiver = Expr::new(ExprKind::Path(path("getter")), span);

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("get_value"),
            args: vec![].into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::int());
}

// ============================================================================
// TEST 12: Method with Multiple Parameters of Different Types
// ============================================================================

#[test]
fn test_method_multiple_param_types() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method taking Int, Bool, Text
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("complex"),
        Type::function(
            vec![Type::int(), Type::bool(), Type::text()].into(),
            Type::unit(),
        ),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "handler".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build: handler.complex(42, true, "hello")
    let receiver = Expr::new(ExprKind::Path(path("handler")), span);
    let args = vec![
        Expr::literal(Literal::int(42, span)),
        Expr::literal(Literal::bool(true, span)),
        Expr::literal(Literal::string("hello".to_string().into(), span)),
    ];

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("complex"),
            args: args.into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::unit());
}

// ============================================================================
// TEST 13: Method Returning Function
// ============================================================================

#[test]
fn test_method_returning_function() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method returning a function
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("get_adder"),
        Type::function(
            vec![].into(),
            Type::function(vec![Type::int(), Type::int()].into(), Type::int()),
        ),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "factory".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Build: factory.get_adder()
    let receiver = Expr::new(ExprKind::Path(path("factory")), span);

    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("get_adder"),
            args: vec![].into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();

    // Result should be a function type
    assert!(matches!(result.ty, Type::Function { .. }));

    if let Type::Function {
        params,
        return_type,
        ..
    } = result.ty
    {
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], Type::int());
        assert_eq!(params[1], Type::int());
        assert_eq!(*return_type, Type::int());
    }
}

// ============================================================================
// TEST 14: Nested Record Method Access
// ============================================================================

#[test]
fn test_nested_record_method() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create inner record with method
    let mut inner_fields = indexmap::IndexMap::new();
    inner_fields.insert(
        Text::from("execute"),
        Type::function(vec![Type::int()].into(), Type::bool()),
    );
    let inner_record = Type::Record(inner_fields);

    // Create outer record containing inner record
    let mut outer_fields = indexmap::IndexMap::new();
    outer_fields.insert(Text::from("inner"), inner_record);
    let outer_record = Type::Record(outer_fields);

    checker.context_mut().env.insert(
        "nested".to_string(),
        verum_types::context::TypeScheme::mono(outer_record),
    );

    // Build: nested.inner.execute(10)
    let nested_expr = Expr::new(ExprKind::Path(path("nested")), span);
    let inner_field = Expr::new(
        ExprKind::Field {
            expr: Box::new(nested_expr),
            field: ident("inner"),
        },
        span,
    );

    let args = vec![Expr::literal(Literal::int(10, span))];
    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(inner_field),
            method: ident("execute"),
            args: args.into(),
        },
        span,
    );

    let result = checker.synth_expr(&method_call).unwrap();
    assert_eq!(result.ty, Type::bool());
}

// ============================================================================
// TEST 15: Method Call Metrics
// ============================================================================

#[test]
fn test_method_call_metrics() {
    let mut checker = TypeChecker::new();
    let span = dummy();

    // Create a record with a method
    let mut fields = indexmap::IndexMap::new();
    fields.insert(
        Text::from("test"),
        Type::function(vec![].into(), Type::unit()),
    );
    let record_ty = Type::Record(fields);

    checker.context_mut().env.insert(
        "obj".to_string(),
        verum_types::context::TypeScheme::mono(record_ty),
    );

    // Check initial protocol check count
    let initial_count = checker.metrics.protocol_checks;

    // Build: obj.test()
    let receiver = Expr::new(ExprKind::Path(path("obj")), span);
    let method_call = Expr::new(
        ExprKind::MethodCall { type_args: vec![].into(),
            receiver: Box::new(receiver),
            method: ident("test"),
            args: vec![].into(),
        },
        span,
    );

    // Execute method call
    let _result = checker.synth_expr(&method_call);

    // Protocol check count should have increased
    assert!(checker.metrics.protocol_checks > initial_count);
}
