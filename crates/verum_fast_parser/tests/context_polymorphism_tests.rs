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
//! Parser tests for Context Polymorphism support.
//!
//! Tests for context polymorphism: abstracting over context sets in generic functions
//! Tests for context system and context parameter grammar rules
//!
//! Tests parsing of `using C` syntax in generic parameters for context polymorphism.

use verum_ast::span::FileId;
use verum_ast::ty::GenericParamKind;
use verum_ast::ItemKind;
use verum_common::List;
use verum_lexer::Lexer;
use verum_fast_parser::{ParseError, VerumParser};

/// Helper to parse a module from a string
fn parse_module(input: &str) -> Result<verum_ast::Module, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id)
}

/// Helper to check if parsing succeeds
fn parses_ok(input: &str) -> bool {
    parse_module(input).is_ok()
}

// ============================================================================
// Basic Context Parameter Parsing Tests
// ============================================================================

#[test]
fn test_parse_context_param_in_function() {
    // Basic context polymorphic function: fn foo<T, using C>(x: T) -> T using C
    let input = r#"
fn identity<T, using C>(value: T) -> T using C {
    value
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse function with context param: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            // Check generic params
            let generics = &func.generics;
            assert_eq!(generics.len(), 2, "Should have 2 generic params");

            // First param should be type T
            match &generics[0].kind {
                GenericParamKind::Type { name, .. } => {
                    assert_eq!(name.name.as_str(), "T");
                }
                _ => panic!("Expected Type param at index 0"),
            }

            // Second param should be context C
            match &generics[1].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param at index 1, got {:?}", generics[1].kind),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_context_param_only() {
    // Function with only context param (no type params)
    let input = r#"
fn logged_action<using C>() using C {
    ()
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse function with only context param: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;
            assert_eq!(generics.len(), 1, "Should have 1 generic param");

            match &generics[0].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_multiple_type_params_with_context() {
    // Multiple type params followed by context param
    let input = r#"
fn transform<A, B, R, using Ctx>(a: A, b: B) -> R using Ctx {
    a
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse with multiple type params and context: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;
            assert_eq!(generics.len(), 4, "Should have 4 generic params");

            // Check names
            let names: Vec<&str> = generics.iter().map(|p| {
                match &p.kind {
                    GenericParamKind::Type { name, .. } => name.name.as_str(),
                    GenericParamKind::Context { name } => name.name.as_str(),
                    _ => "",
                }
            }).collect();

            assert_eq!(names, vec!["A", "B", "R", "Ctx"]);

            // Last one should be context
            match &generics[3].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "Ctx");
                }
                _ => panic!("Expected Context param at last position"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_context_param_various_names() {
    // Test various valid context param names
    let names = vec!["C", "Ctx", "Context", "MyContext", "CtxA", "C1", "DatabaseCtx"];

    for ctx_name in names {
        let input = format!(r#"
fn foo<T, using {}>(x: T) -> T using {} {{
    x
}}
"#, ctx_name, ctx_name);

        let result = parse_module(&input);
        assert!(result.is_ok(), "Should parse context param name '{}': {:?}", ctx_name, result.err());

        let module = result.unwrap();
        match &module.items[0].kind {
            ItemKind::Function(func) => {
                match &func.generics[1].kind {
                    GenericParamKind::Context { name } => {
                        assert_eq!(name.name.as_str(), ctx_name);
                    }
                    _ => panic!("Expected Context param for name: {}", ctx_name),
                }
            }
            _ => panic!("Expected Function declaration"),
        }
    }
}

// ============================================================================
// Context Parameter with Callbacks Tests
// ============================================================================

#[test]
fn test_parse_context_polymorphic_higher_order() {
    // Higher-order function with context-polymorphic callback
    let input = r#"
fn map<T, U, using C>(value: T, f: fn(T) -> U using C) -> U using C {
    f(value)
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse higher-order context-polymorphic function: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            // Check that we have T, U, and context C
            assert_eq!(func.generics.len(), 3);

            // Last should be context
            match &func.generics[2].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_nested_context_polymorphism() {
    // Nested context-polymorphic calls
    let input = r#"
fn outer<T, using OuterCtx>(value: T, f: fn(T) -> T using OuterCtx) -> T using OuterCtx {
    f(value)
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse nested context polymorphism: {:?}", result.err());
}

// ============================================================================
// Context Parameter with Other Generic Param Types Tests
// ============================================================================

#[test]
fn test_parse_context_with_hkt() {
    // Context param alongside higher-kinded type param
    let input = r#"
fn traverse<F<_>, T, U, using C>(container: F<T>, f: fn(T) -> U using C) -> F<U> using C {
    container
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context with HKT: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;

            // Find context param
            let context_params: Vec<_> = generics.iter()
                .filter(|p| matches!(&p.kind, GenericParamKind::Context { .. }))
                .collect();

            assert_eq!(context_params.len(), 1, "Should have exactly 1 context param");
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_context_with_meta_param() {
    // Context param alongside meta param
    let input = r#"
fn sized_op<T, N: meta Int, using C>(arr: Array<T, N>) -> T using C {
    arr[0]
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context with meta param: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let generics = &func.generics;
            assert_eq!(generics.len(), 3);

            // Check for context param
            match &generics[2].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param at index 2"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_context_with_bounds() {
    // Type param with bounds alongside context param
    let input = r#"
fn clone_with_ctx<T: Clone, using C>(value: T) -> T using C {
    value.clone()
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context with bounded type: {:?}", result.err());
}

// ============================================================================
// Context Parameter in Various Positions Tests
// ============================================================================

#[test]
fn test_parse_context_param_position_first() {
    // Context param can appear first (though unusual)
    let input = r#"
fn unusual<using C, T>(x: T) -> T using C {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context param in first position: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            // First should be context
            match &func.generics[0].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param at first position"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

#[test]
fn test_parse_context_param_position_middle() {
    // Context param in middle
    let input = r#"
fn middle<T, using C, U>(x: T, y: U) -> T using C {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context param in middle: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            // Middle should be context
            match &func.generics[1].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "C");
                }
                _ => panic!("Expected Context param in middle"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

// ============================================================================
// Multiple Context Parameters Tests
// ============================================================================

#[test]
fn test_parse_multiple_context_params() {
    // Multiple context params (unusual but syntactically valid)
    let input = r#"
fn multi_ctx<T, using C1, using C2>(x: T) -> T using [C1, C2] {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse multiple context params: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let context_params: Vec<_> = func.generics.iter()
                .filter(|p| matches!(&p.kind, GenericParamKind::Context { .. }))
                .collect();

            assert_eq!(context_params.len(), 2, "Should have 2 context params");

            let names: Vec<&str> = context_params.iter()
                .filter_map(|p| match &p.kind {
                    GenericParamKind::Context { name } => Some(name.name.as_str()),
                    _ => None,
                })
                .collect();

            assert_eq!(names, vec!["C1", "C2"]);
        }
        _ => panic!("Expected Function declaration"),
    }
}

// ============================================================================
// Context Parameter Error Cases Tests
// ============================================================================

#[test]
fn test_parse_using_keyword_only_fails() {
    // Just `using` without identifier should fail
    let input = r#"
fn bad<T, using>(x: T) -> T {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_err(), "Should fail parsing 'using' without identifier");
}

#[test]
fn test_parse_using_with_number_fails() {
    // `using 123` should fail (not a valid identifier)
    let input = r#"
fn bad<T, using 123>(x: T) -> T {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_err(), "Should fail parsing 'using' with number");
}

// ============================================================================
// Context Parameter in Method Tests
// ============================================================================

#[test]
fn test_parse_context_param_in_method() {
    // Method with context param
    let input = r#"
type Container<T> is {
    items: List<T>,
};

implement<T> Container<T> {
    fn map_items<U, using C>(&self, f: fn(T) -> U using C) -> Container<U> using C {
        Container { items: List.new() }
    }
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse method with context param: {:?}", result.err());
}

// ============================================================================
// Context Parameter Pretty Print Roundtrip Tests
// ============================================================================

#[test]
fn test_context_param_preserves_name() {
    let input = r#"
fn identity<T, using MyContext>(value: T) -> T using MyContext {
    value
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            // Verify the context param name is exactly preserved
            match &func.generics[1].kind {
                GenericParamKind::Context { name } => {
                    assert_eq!(name.name.as_str(), "MyContext");
                }
                _ => panic!("Expected Context param"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

// ============================================================================
// Real-World Usage Pattern Tests
// ============================================================================

#[test]
fn test_parse_iterator_map_pattern() {
    // Real-world pattern: context-polymorphic iterator map
    let input = r#"
fn iter_map<T, U, using C>(
    iter: Iterator<T>,
    f: fn(T) -> U using C
) -> Iterator<U> using C {
    iter
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse iterator map pattern: {:?}", result.err());
}

#[test]
fn test_parse_result_and_then_pattern() {
    // Real-world pattern: context-polymorphic Result.and_then
    let input = r#"
fn and_then<T, U, E, using C>(
    result: Result<T, E>,
    f: fn(T) -> Result<U, E> using C
) -> Result<U, E> using C {
    result
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse Result.and_then pattern: {:?}", result.err());
}

#[test]
fn test_parse_maybe_map_pattern() {
    // Real-world pattern: context-polymorphic Maybe.map
    let input = r#"
fn maybe_map<T, U, using C>(
    opt: Maybe<T>,
    f: fn(T) -> U using C
) -> Maybe<U> using C {
    opt
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse Maybe.map pattern: {:?}", result.err());
}

// ============================================================================
// Context Parameter Span Tests
// ============================================================================

#[test]
fn test_context_param_has_correct_span() {
    let input = "fn foo<T, using Ctx>(x: T) -> T using Ctx { x }";

    let result = parse_module(input);
    assert!(result.is_ok());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let context_param = &func.generics[1];

            // Span should be non-zero and reasonable
            assert!(context_param.span.start < context_param.span.end,
                    "Context param span should have positive length");

            match &context_param.kind {
                GenericParamKind::Context { name } => {
                    // Name span should also be valid
                    assert!(name.span.start < name.span.end,
                            "Context param name span should have positive length");
                }
                _ => panic!("Expected Context param"),
            }
        }
        _ => panic!("Expected Function declaration"),
    }
}

// ============================================================================
// Context Parameter in Protocol Tests
// ============================================================================

#[test]
fn test_parse_context_param_in_protocol_method() {
    // Protocol method with context param
    let input = r#"
type Mappable is protocol {
    type Item;
    type Output;

    fn map<U, using C>(&self, f: fn(Self.Item) -> U using C) -> Self.Output using C;
};
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse protocol method with context param: {:?}", result.err());
}

// ============================================================================
// Context Parameter with Lifetime Tests
// ============================================================================

#[test]
fn test_parse_context_param_with_lifetime() {
    // Context param alongside lifetime param
    let input = r#"
fn borrowed<'a, T, using C>(value: &'a T) -> &'a T using C {
    value
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context with lifetime param: {:?}", result.err());
}

// ============================================================================
// Context Parameter Count Tests
// ============================================================================

#[test]
fn test_count_context_params() {
    let input = r#"
fn three_context<T, using A, using B, using C>(x: T) -> T using [A, B, C] {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse three context params: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let context_count = func.generics.iter()
                .filter(|p| matches!(&p.kind, GenericParamKind::Context { .. }))
                .count();

            assert_eq!(context_count, 3, "Should have exactly 3 context params");
        }
        _ => panic!("Expected Function declaration"),
    }
}

// ============================================================================
// Context Parameter Order Tests
// ============================================================================

#[test]
fn test_context_params_maintain_order() {
    let input = r#"
fn ordered<using First, T, using Second, U, using Third>(x: T, y: U) -> T using [First, Second, Third] {
    x
}
"#;

    let result = parse_module(input);
    assert!(result.is_ok(), "Should parse context params in various positions: {:?}", result.err());

    let module = result.unwrap();
    match &module.items[0].kind {
        ItemKind::Function(func) => {
            let params = &func.generics;

            // Check order: First, T, Second, U, Third
            assert!(matches!(&params[0].kind, GenericParamKind::Context { name } if name.name.as_str() == "First"));
            assert!(matches!(&params[1].kind, GenericParamKind::Type { name, .. } if name.name.as_str() == "T"));
            assert!(matches!(&params[2].kind, GenericParamKind::Context { name } if name.name.as_str() == "Second"));
            assert!(matches!(&params[3].kind, GenericParamKind::Type { name, .. } if name.name.as_str() == "U"));
            assert!(matches!(&params[4].kind, GenericParamKind::Context { name } if name.name.as_str() == "Third"));
        }
        _ => panic!("Expected Function declaration"),
    }
}
