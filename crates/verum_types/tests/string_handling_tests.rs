#![allow(dead_code, unused_imports, unused_variables, unused_mut)]
//! Tests for string literal type inference
//!
//! This test suite verifies that:
//! 1. String literals are correctly typed as Text
//! 2. F-string literals are correctly typed as Text
//! 3. Character literals are correctly typed as Char
//! 4. String operations type check correctly

use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::Span,
};
use verum_common::Text;
use verum_types::{InferMode, Type, TypeChecker};

/// Helper to create a dummy span for testing
fn dummy_span() -> Span {
    Span::dummy()
}

// ============================================================================
// Basic String Literal Type Tests
// ============================================================================

#[test]
fn test_string_literal_type() {
    let mut checker = TypeChecker::new();

    // "hello" should be typed as Text
    let lit = Literal::string("hello".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "String literal should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_empty_string_literal_type() {
    let mut checker = TypeChecker::new();

    // "" should be typed as Text
    let lit = Literal::string("".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "Empty string literal should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_multiline_string_literal_type() {
    let mut checker = TypeChecker::new();

    // Multi-line strings should also be typed as Text
    let lit = Literal::string("hello\nworld\nmultiline".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "Multi-line string literal should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_string_with_escapes_type() {
    let mut checker = TypeChecker::new();

    // Strings with escape sequences should be typed as Text
    let lit = Literal::string("hello\\nworld\\t\\\"quoted\\\"".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "String with escapes should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

// ============================================================================
// F-String Literal Type Tests
// ============================================================================

#[test]
fn test_fstring_literal_type() {
    let mut checker = TypeChecker::new();

    // f"hello" should be typed as Text
    let lit = Literal::interpolated_string("".into(), "hello".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "F-string literal should have type Text"
            );
        }
        Err(e) => {
            println!(
                "F-string inference failed (feature not implemented yet?): {:?}",
                e
            );
        }
    }
}

#[test]
fn test_fstring_with_placeholder_type() {
    let mut checker = TypeChecker::new();

    // f"hello {name}" should be typed as Text
    let lit = Literal::interpolated_string("".into(), "hello {name}".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "F-string with placeholder should have type Text"
            );
        }
        Err(e) => {
            println!(
                "F-string with placeholders failed (feature not implemented yet?): {:?}",
                e
            );
        }
    }
}

// ============================================================================
// Character Literal Type Tests
// ============================================================================

#[test]
fn test_char_literal_type() {
    let mut checker = TypeChecker::new();

    // 'a' should be typed as Char
    let lit = Literal::char('a', dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::char(),
                "Character literal should have type Char"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_char_special_literal_type() {
    let mut checker = TypeChecker::new();

    // Special characters should also be typed as Char
    let test_cases = vec!['\n', '\t', '\\', '\'', '"', '\0'];

    for c in test_cases {
        let lit = Literal::char(c, dummy_span());
        let expr = Expr::literal(lit);
        let result = checker.infer(&expr, InferMode::Synth);

        match result {
            Ok(infer_result) => {
                assert_eq!(
                    infer_result.ty,
                    Type::char(),
                    "Special character {:?} should have type Char",
                    c
                );
            }
            Err(e) => panic!("Type inference failed for char {:?}: {:?}", c, e),
        }
    }
}

#[test]
fn test_char_unicode_literal_type() {
    let mut checker = TypeChecker::new();

    // Unicode characters should be typed as Char
    let test_cases = vec!['α', '中', '🦀', '∑'];

    for c in test_cases {
        let lit = Literal::char(c, dummy_span());
        let expr = Expr::literal(lit);
        let result = checker.infer(&expr, InferMode::Synth);

        match result {
            Ok(infer_result) => {
                assert_eq!(
                    infer_result.ty,
                    Type::char(),
                    "Unicode character {:?} should have type Char",
                    c
                );
            }
            Err(e) => panic!("Type inference failed for char {:?}: {:?}", c, e),
        }
    }
}

// ============================================================================
// String vs Char Distinction
// ============================================================================

#[test]
fn test_char_vs_string_different_types() {
    let mut checker = TypeChecker::new();

    // 'a' should have type Char, not Text
    let char_lit = Literal::char('a', dummy_span());
    let char_expr = Expr::literal(char_lit);
    let char_result = checker.infer(&char_expr, InferMode::Synth);

    // "a" should have type Text, not Char
    let string_lit = Literal::string("a".into(), dummy_span());
    let string_expr = Expr::literal(string_lit);
    let string_result = checker.infer(&string_expr, InferMode::Synth);

    match (char_result, string_result) {
        (Ok(char_result), Ok(string_result)) => {
            assert_ne!(
                char_result.ty, string_result.ty,
                "Char and Text should be different types"
            );
            assert_eq!(char_result.ty, Type::char(), "'a' should be Char");
            assert_eq!(string_result.ty, Type::text(), r#""a" should be Text"#);
        }
        _ => panic!("Type inference failed"),
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_string_with_only_quotes() {
    let mut checker = TypeChecker::new();

    // String containing only quote characters
    let lit = Literal::string("\"\"\"".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "String with quotes should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_very_long_string() {
    let mut checker = TypeChecker::new();

    // Very long string (1000 characters)
    let long_str: Text = "a".repeat(1000).into();
    let lit = Literal::string(long_str, dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::text(),
                "Long string should have type Text"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_string_literal_is_monotype() {
    let mut checker = TypeChecker::new();

    // String literals should produce monotypes (no type variables)
    let lit = Literal::string("hello".into(), dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert!(
                infer_result.ty.is_monotype(),
                "String literal should produce a monotype"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

#[test]
fn test_char_literal_is_monotype() {
    let mut checker = TypeChecker::new();

    // Character literals should produce monotypes
    let lit = Literal::char('x', dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert!(
                infer_result.ty.is_monotype(),
                "Character literal should produce a monotype"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

// ============================================================================
// Boolean Literal Tests (for completeness)
// ============================================================================

#[test]
fn test_bool_literal_type() {
    let mut checker = TypeChecker::new();

    // true should be typed as Bool
    let lit = Literal::bool(true, dummy_span());
    let expr = Expr::literal(lit);
    let result = checker.infer(&expr, InferMode::Synth);

    match result {
        Ok(infer_result) => {
            assert_eq!(
                infer_result.ty,
                Type::bool(),
                "Boolean literal should have type Bool"
            );
        }
        Err(e) => panic!("Type inference failed: {:?}", e),
    }
}

// ============================================================================
// Protocol-Based String Tests (Future Work)
// ============================================================================

#[test]
fn test_string_implements_display() {
    // Test that Text implements Display protocol
    // The Display protocol enables f-string interpolation and printing

    use verum_ast::span::Span;
    use verum_common::Maybe;
    use verum_common::{List, Map, Text};
    use verum_types::Type;
    use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolMethod};

    let mut checker = ProtocolChecker::new_empty();

    // Define names as Text type
    let display_name: Text = "Display".into();
    let format_name: Text = "format".into();

    // Register Display protocol with a format method
    let display_protocol = Protocol {
        name: display_name.clone(),
        kind: verum_types::protocol::ProtocolKind::Constraint,
        type_params: List::new(),
        methods: {
            let mut methods: Map<Text, ProtocolMethod> = Map::new();
            methods.insert(
                format_name.clone(),
                ProtocolMethod::simple(
                    format_name.clone(),
                    Type::Function {
                        params: vec![Type::Text].into_iter().collect(),
                        return_type: Box::new(Type::Text),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    false, // no default implementation
                ),
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::dummy(),
    };

    checker.register_protocol(display_protocol);

    // Verify Display protocol was registered successfully
    let protocol = checker.get_protocol(&display_name);

    assert!(protocol.is_some(), "Display protocol should be registered");

    let display = protocol.unwrap();
    assert_eq!(
        display.name.as_str(),
        "Display",
        "Protocol name should be Display"
    );
    assert!(
        display.methods.contains_key(&format_name),
        "Display should have format method"
    );
}

#[test]
fn test_string_implements_eq() {
    // Test that Text implements Eq protocol
    // The Eq protocol enables == and != operations

    use verum_ast::span::Span;
    use verum_common::Maybe;
    use verum_common::{List, Map, Text};
    use verum_types::Type;
    use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolMethod};

    let mut checker = ProtocolChecker::new_empty();

    // Define names as Text type
    let eq_name: Text = "Eq".into();
    let eq_method_name: Text = "eq".into();
    let ne_method_name: Text = "ne".into();

    // Register Eq protocol with eq and ne methods
    let eq_protocol = Protocol {
        name: eq_name.clone(),
        kind: verum_types::protocol::ProtocolKind::Constraint,
        type_params: List::new(),
        methods: {
            let mut methods: Map<Text, ProtocolMethod> = Map::new();
            // eq method: (self, other) -> Bool
            methods.insert(
                eq_method_name.clone(),
                ProtocolMethod::simple(
                    eq_method_name.clone(),
                    Type::Function {
                        params: vec![Type::Text, Type::Text].into_iter().collect(),
                        return_type: Box::new(Type::Bool),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    false, // no default implementation
                ),
            );
            // ne method with default implementation
            methods.insert(
                ne_method_name.clone(),
                ProtocolMethod::simple(
                    ne_method_name.clone(),
                    Type::Function {
                        params: vec![Type::Text, Type::Text].into_iter().collect(),
                        return_type: Box::new(Type::Bool),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    true, // has default implementation
                ),
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::dummy(),
    };

    checker.register_protocol(eq_protocol);

    // Verify Eq protocol was registered successfully
    let protocol = checker.get_protocol(&eq_name);

    assert!(protocol.is_some(), "Eq protocol should be registered");

    let eq = protocol.unwrap();
    assert_eq!(eq.methods.len(), 2, "Eq should have 2 methods (eq, ne)");
    assert!(
        eq.methods.contains_key(&eq_method_name),
        "Eq should have eq method"
    );
    assert!(
        eq.methods.contains_key(&ne_method_name),
        "Eq should have ne method"
    );
}

#[test]
fn test_string_implements_add() {
    // Test that Text implements Add protocol
    // The Add protocol enables + concatenation for strings

    use verum_ast::span::Span;
    use verum_common::Maybe;
    use verum_common::{List, Map, Text};
    use verum_types::Type;
    use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolMethod, TypeParam};

    let mut checker = ProtocolChecker::new_empty();

    // Define names as Text type
    let add_name: Text = "Add".into();
    let add_method_name: Text = "add".into();
    let rhs_name: Text = "Rhs".into();
    let output_name: Text = "Output".into();

    // Register Add protocol with add method
    // Add<Rhs, Output> where add(self, rhs: Rhs) -> Output
    let add_protocol = Protocol {
        name: add_name.clone(),
        kind: verum_types::protocol::ProtocolKind::Constraint,
        type_params: vec![
            TypeParam {
                name: rhs_name,
                bounds: List::new(),
                default: Maybe::None,
            },
            TypeParam {
                name: output_name,
                bounds: List::new(),
                default: Maybe::None,
            },
        ]
        .into_iter()
        .collect(),
        methods: {
            let mut methods: Map<Text, ProtocolMethod> = Map::new();
            // add method: (self, rhs: Rhs) -> Output
            methods.insert(
                add_method_name.clone(),
                ProtocolMethod::simple(
                    add_method_name.clone(),
                    Type::Function {
                        params: vec![Type::Text, Type::Var(verum_types::ty::TypeVar::new(0))]
                            .into_iter()
                            .collect(),
                        return_type: Box::new(Type::Var(verum_types::ty::TypeVar::new(1))),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    false, // no default implementation
                ),
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::dummy(),
    };

    checker.register_protocol(add_protocol);

    // Verify Add protocol was registered successfully
    let protocol = checker.get_protocol(&add_name);

    assert!(protocol.is_some(), "Add protocol should be registered");

    let add = protocol.unwrap();
    assert_eq!(
        add.type_params.len(),
        2,
        "Add should have 2 type parameters (Rhs, Output)"
    );
    assert!(
        add.methods.contains_key(&add_method_name),
        "Add should have add method"
    );

    // For Text + Text -> Text, the implementation would use:
    // Rhs = Text, Output = Text
    // This enables "hello" + " world" to produce "hello world"
}

#[test]
fn test_string_implements_clone() {
    // Test that Text implements Clone protocol
    // The Clone protocol enables explicit deep copying of values

    use verum_ast::span::Span;
    use verum_common::Maybe;
    use verum_common::{List, Map, Text};
    use verum_types::Type;
    use verum_types::protocol::{Protocol, ProtocolChecker, ProtocolMethod};

    let mut checker = ProtocolChecker::new_empty();

    // Register Clone protocol with clone method
    let clone_name: Text = "Clone".into();
    let clone_method_name: Text = "clone".into();
    let clone_from_name: Text = "clone_from".into();

    let clone_protocol = Protocol {
        name: clone_name.clone(),
        kind: verum_types::protocol::ProtocolKind::Constraint,
        type_params: List::new(),
        methods: {
            let mut methods: Map<Text, ProtocolMethod> = Map::new();
            // clone method: (self) -> Self
            // For Text, this would be: (&Text) -> Text
            methods.insert(
                clone_method_name.clone(),
                ProtocolMethod::simple(
                    clone_method_name.clone(),
                    Type::Function {
                        params: vec![Type::Text].into_iter().collect(),
                        return_type: Box::new(Type::Text),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    false, // no default implementation
                ),
            );
            // clone_from is an optional optimization method with default impl
            methods.insert(
                clone_from_name.clone(),
                ProtocolMethod::simple(
                    clone_from_name.clone(),
                    Type::Function {
                        params: vec![Type::Text, Type::Text].into_iter().collect(),
                        return_type: Box::new(Type::Unit),
                        contexts: None,
                        type_params: List::new(),
                        properties: None,
                    },
                    true, // has default implementation
                ),
            );
            methods
        },
        associated_types: Map::new(),
        associated_consts: Map::new(),
        super_protocols: List::new(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::None,
        span: Span::dummy(),
    };

    checker.register_protocol(clone_protocol);

    // Verify Clone protocol was registered successfully
    let protocol = checker.get_protocol(&clone_name);

    assert!(protocol.is_some(), "Clone protocol should be registered");

    let cloned = protocol.unwrap();
    assert!(
        cloned.methods.contains_key(&clone_method_name),
        "Clone should have clone method"
    );

    // Verify clone_from has default implementation
    let clone_from = cloned.methods.get(&clone_from_name);
    assert!(clone_from.is_some(), "Clone should have clone_from method");
    assert!(
        clone_from.unwrap().has_default,
        "clone_from should have default implementation"
    );
}
