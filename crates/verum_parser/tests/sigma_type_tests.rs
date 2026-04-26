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
// Test suite for sigma-type refinement parsing.
//
// Sigma-types are a key feature of Verum's refinement type system.
// Rule 3 of Five Binding Rules: sigma-type `n: T where pred` is canonical dependent type form
//
// Syntax: name: Type where predicate
// Example: x: Int where x > 0
//
// This test suite verifies that the parser correctly handles sigma-type syntax
// in all relevant contexts.

use verum_ast::{TypeKind, span::FileId};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper function to parse a type from a string.
fn parse_type(source: &str) -> Result<verum_ast::Type, String> {
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    parser.parse_type_str(source, file_id).map_err(|errors| {
        errors
            .into_iter()
            .map(|e| format!("{:?}", e))
            .collect::<Vec<_>>()
            .join(", ")
    })
}

/// Helper function to parse a type declaration from a string.
fn parse_decl(source: &str) -> Result<verum_common::List<verum_ast::Item>, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .map(|module| module.items)
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| format!("{:?}", e))
                .collect::<Vec<_>>()
                .join(", ")
        })
}

/// Helper to check if parsing succeeds.
fn assert_parses_type(source: &str) {
    parse_type(source).unwrap_or_else(|_| panic!("Failed to parse type: {}", source));
}

/// Helper to check if parsing succeeds.
fn assert_parses_decl(source: &str) {
    parse_decl(source).unwrap_or_else(|_| panic!("Failed to parse decl: {}", source));
}

// ============================================================================
// SECTION 1: BASIC SIGMA-TYPE PARSING (~10 tests)
// ============================================================================

#[test]
fn test_sigma_type_simple_positive_int() {
    // Per VVA §5 the sigma surface form parses to `TypeKind::Refined`
    // with `predicate.binding = Some(name)`.
    let ty = parse_type("x: Int where x > 0").unwrap();
    match ty.kind {
        TypeKind::Refined { ref base, ref predicate } => {
            let binder = match &predicate.binding {
                verum_common::Maybe::Some(id) => id,
                verum_common::Maybe::None => {
                    panic!("Expected sigma refinement with explicit binder")
                }
            };
            assert_eq!(binder.name.as_str(), "x");
            assert!(matches!(base.kind, TypeKind::Int));
        }
        _ => panic!("Expected Refined (sigma form), got {:?}", ty.kind),
    }
}

#[test]
fn test_sigma_type_with_named_predicate() {
    let ty = parse_type("value: Int where value >= 0").unwrap();
    match ty.kind {
        TypeKind::Refined { ref predicate, .. } => {
            let binder = match &predicate.binding {
                verum_common::Maybe::Some(id) => id,
                verum_common::Maybe::None => {
                    panic!("Expected sigma refinement with explicit binder")
                }
            };
            assert_eq!(binder.name.as_str(), "value");
        }
        _ => panic!("Expected Refined (sigma form), got {:?}", ty.kind),
    }
}

#[test]
fn test_sigma_type_with_float() {
    assert_parses_type("percent: Float where percent >= 0.0 && percent <= 100.0");
}

#[test]
fn test_sigma_type_with_text() {
    assert_parses_type("email: Text where len(email) > 3");
}

#[test]
fn test_sigma_type_with_complex_predicate() {
    assert_parses_type("port: Int where port >= 1 && port <= 65535");
}

#[test]
fn test_sigma_type_with_function_call() {
    assert_parses_type("s: Text where is_email(s)");
}

#[test]
fn test_sigma_type_with_method_call() {
    assert_parses_type("email: Text where email.contains('@')");
}

#[test]
fn test_sigma_type_with_logical_or() {
    assert_parses_type("x: Int where x < 0 || x > 10");
}

#[test]
fn test_sigma_type_with_arithmetic() {
    assert_parses_type("x: Int where x * 2 > 100");
}

#[test]
fn test_sigma_type_multiple_conditions() {
    assert_parses_type("user: User where user.age > 18 && user.verified");
}

// ============================================================================
// SECTION 2: SIGMA-TYPES IN TYPE DECLARATIONS (~10 tests)
// ============================================================================

#[test]
fn test_sigma_type_in_type_alias() {
    assert_parses_decl("type Positive is x: Int where x > 0;");
}

#[test]
fn test_sigma_type_in_type_alias_float() {
    assert_parses_decl("type Percentage is p: Float where p >= 0.0 && p <= 100.0;");
}

#[test]
fn test_sigma_type_in_type_alias_text() {
    assert_parses_decl("type Email is email: Text where email.contains('@');");
}

#[test]
fn test_sigma_type_in_type_alias_complex() {
    assert_parses_decl(
        "type ValidUser is user: User where user.age > 18 && user.email.contains('@');",
    );
}

#[test]
fn test_sigma_type_with_port_range() {
    assert_parses_decl("type Port is port: Int where port >= 1 && port <= 65535;");
}

#[test]
fn test_sigma_type_with_age_constraint() {
    assert_parses_decl("type Age is age: Int where age >= 0 && age <= 150;");
}

#[test]
fn test_sigma_type_with_non_empty_string() {
    assert_parses_decl("type NonEmptyText is s: Text where len(s) > 0;");
}

#[test]
fn test_sigma_type_with_bounded_string() {
    assert_parses_decl("type Username is name: Text where len(name) >= 3 && len(name) <= 20;");
}

#[test]
fn test_sigma_type_with_custom_predicate() {
    assert_parses_decl("type ValidEmail is email: Text where is_valid_email(email);");
}

#[test]
fn test_sigma_type_with_complex_user_validation() {
    assert_parses_decl(
        r#"
        type ValidUser is user: User where
            user.age >= 18 &&
            user.email.len() > 5 &&
            user.email.contains('@');
    "#,
    );
}

// ============================================================================
// SECTION 3: SIGMA-TYPES WITH GENERIC TYPES (~5 tests)
// ============================================================================

#[test]
fn test_sigma_type_with_generic_base() {
    assert_parses_type("list: Vec<Int> where len(list) > 0");
}

#[test]
fn test_sigma_type_with_nested_generic() {
    assert_parses_type("map: HashMap<String, Int> where len(map) > 0");
}

#[test]
fn test_sigma_type_with_option() {
    assert_parses_type("opt: Option<Int> where is_some(opt)");
}

#[test]
fn test_sigma_type_with_result() {
    assert_parses_type("res: Result<Int, String> where is_ok(res)");
}

#[test]
fn test_sigma_type_with_complex_generic() {
    assert_parses_type("data: Vec<User> where len(data) > 0 && all_valid(data)");
}

// ============================================================================
// SECTION 4: EDGE CASES AND SPECIAL SYNTAX (~5 tests)
// ============================================================================

#[test]
fn test_sigma_type_with_single_letter_name() {
    assert_parses_type("x: Int where x > 0");
}

#[test]
fn test_sigma_type_with_long_name() {
    assert_parses_type("validatedUserData: User where validatedUserData.age > 18");
}

#[test]
fn test_sigma_type_with_underscore_name() {
    assert_parses_type("user_data: User where user_data.verified");
}

#[test]
fn test_sigma_type_with_nested_field_access() {
    assert_parses_type("user: User where user.profile.age > 18");
}

#[test]
fn test_sigma_type_with_array_indexing() {
    assert_parses_type("arr: Array<Int> where arr[0] > 0");
}

// ============================================================================
// SECTION 5: COMPARISON WITH OTHER REFINEMENT STYLES (~10 tests)
// ============================================================================

#[test]
fn test_inline_refinement_still_works() {
    // Rule 1: Inline refinement {expr}
    assert_parses_type("Int{> 0}");
}

#[test]
fn test_lambda_refinement_still_works() {
    // Rule 2: Lambda-style: where |x| expr
    assert_parses_type("Int where |x| x > 0");
}

#[test]
fn test_bare_where_still_works() {
    // Rule 5: Bare where (deprecated)
    assert_parses_type("Int where it > 0");
}

#[test]
fn test_sigma_vs_inline_in_decl() {
    // Both should work
    assert_parses_decl("type Positive1 is Int{> 0};");
    assert_parses_decl("type Positive2 is x: Int where x > 0;");
}

#[test]
fn test_sigma_vs_lambda_in_decl() {
    // Both should work
    assert_parses_decl("type Positive1 is Int where |x| x > 0;");
    assert_parses_decl("type Positive2 is x: Int where x > 0;");
}

#[test]
fn test_sigma_type_preferred_for_complex_predicates() {
    // Sigma-type is clearer for complex predicates with field access
    assert_parses_decl("type ValidUser is user: User where user.age > 18 && user.verified;");
}

#[test]
fn test_inline_refinement_preferred_for_simple_predicates() {
    // Inline is clearer for simple predicates
    assert_parses_decl("type Positive is Int{> 0};");
}

#[test]
fn test_lambda_refinement_for_reusable_logic() {
    // Lambda-style is good for complex expressions
    assert_parses_decl("type Email is Text where |s| len(s) > 5 && s.contains('@');");
}

#[test]
fn test_sigma_type_shows_explicit_binding() {
    // Sigma-type makes the binding explicit — `predicate.binding = Some(x)`.
    let ty = parse_type("x: Int where x > 0").unwrap();
    match ty.kind {
        TypeKind::Refined { ref predicate, .. } => {
            assert!(matches!(predicate.binding, verum_common::Maybe::Some(_)));
        }
        _ => panic!("Expected Refined (sigma form)"),
    }
}

#[test]
fn test_all_three_styles_parse_to_refined() {
    // Per VVA §5 the three refinement forms collapse onto `TypeKind::Refined`.
    let inline = parse_type("Int{> 0}").unwrap();
    let lambda = parse_type("Int where |x| x > 0").unwrap();
    let sigma = parse_type("x: Int where x > 0").unwrap();

    assert!(matches!(inline.kind, TypeKind::Refined { .. }));
    assert!(matches!(lambda.kind, TypeKind::Refined { .. }));
    assert!(matches!(sigma.kind, TypeKind::Refined { .. }));

    if let TypeKind::Refined { ref predicate, .. } = sigma.kind {
        assert!(matches!(predicate.binding, verum_common::Maybe::Some(_)));
    }
}

// ============================================================================
// SECTION 6: REAL-WORLD EXAMPLES (~10 tests)
// ============================================================================

#[test]
fn test_real_world_positive_integer() {
    assert_parses_decl("type Positive is n: Int where n > 0;");
}

#[test]
fn test_real_world_percentage() {
    assert_parses_decl("type Percentage is p: Float where p >= 0.0 && p <= 100.0;");
}

#[test]
fn test_real_world_non_empty_string() {
    assert_parses_decl("type NonEmptyString is s: Text where len(s) > 0;");
}

#[test]
fn test_real_world_email_validation() {
    assert_parses_decl("type Email is email: Text where email.contains('@') && email.len() > 3;");
}

#[test]
fn test_real_world_port_number() {
    assert_parses_decl("type Port is port: Int where port >= 1 && port <= 65535;");
}

#[test]
fn test_real_world_age() {
    assert_parses_decl("type Age is age: Int where age >= 0 && age <= 150;");
}

#[test]
fn test_real_world_non_empty_vec() {
    assert_parses_decl("type NonEmptyVec is vec: Vec<Int> where len(vec) > 0;");
}

#[test]
fn test_real_world_sorted_vec() {
    assert_parses_decl("type SortedVec is vec: Vec<Int> where is_sorted(vec);");
}

#[test]
fn test_real_world_bounded_string() {
    assert_parses_decl("type Username is name: Text where len(name) >= 3 && len(name) <= 20;");
}

#[test]
fn test_real_world_valid_user() {
    assert_parses_decl(
        r#"
        type ValidUser is user: User where
            user.age >= 18 &&
            user.email.contains('@') &&
            user.email.len() > 5 &&
            user.verified;
    "#,
    );
}

// ============================================================================
// SUMMARY
// ============================================================================

// Total test count: ~50 tests
// - Basic sigma-type parsing: 10 tests
// - Sigma-types in type declarations: 10 tests
// - Sigma-types with generic types: 5 tests
// - Edge cases and special syntax: 5 tests
// - Comparison with other refinement styles: 10 tests
// - Real-world examples: 10 tests
//
// This comprehensive test suite ensures that the parser correctly handles
// sigma-type syntax (Rule 3 of the Five Binding Rules) in all contexts,
// and that it coexists properly with the other refinement type styles.
