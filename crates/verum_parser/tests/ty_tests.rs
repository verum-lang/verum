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
// Comprehensive test suite for type parsing in Verum.
//
// This test suite covers all type syntax in the Verum language:
// - Primitive types (Int, Float, Bool, String, Char)
// - Generic types (Vec<T>, HashMap<K, V>)
// - Reference types (CBGR: &T, &mut T; Ownership: %T, %mut T)
// - Function types (simple, multi-param, with effects, higher-order)
// - **Refinement types** (the core innovation of Verum!)
// - Protocol types (impl Display + Debug)
// - Effect types ([IO], [Database, Logging])
// - Tuple types ((Int, String, Bool))
// - Array and slice types ([T; N], [T])
// - Complex nested types

use verum_ast::{FileId, Span, Type, TypeKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Helper function to parse a type from a string.
fn parse_type(source: &str) -> Result<Type, String> {
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

/// Helper to check if parsing succeeds.
fn assert_parses(source: &str) {
    parse_type(source).unwrap_or_else(|_| panic!("Failed to parse: {}", source));
}

/// Helper to check if parsing fails.
fn assert_fails(source: &str) {
    assert!(
        parse_type(source).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

// ============================================================================
// SECTION 1: PRIMITIVE TYPES (~10 tests)
// ============================================================================

#[test]
fn test_primitive_int() {
    let ty = parse_type("Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Int));
}

#[test]
fn test_primitive_float() {
    let ty = parse_type("Float").unwrap();
    assert!(matches!(ty.kind, TypeKind::Float));
}

#[test]
fn test_primitive_bool() {
    let ty = parse_type("Bool").unwrap();
    assert!(matches!(ty.kind, TypeKind::Bool));
}

#[test]
fn test_primitive_string() {
    let ty = parse_type("Text").unwrap();
    assert!(matches!(ty.kind, TypeKind::Text));
}

#[test]
fn test_primitive_char() {
    let ty = parse_type("Char").unwrap();
    assert!(matches!(ty.kind, TypeKind::Char));
}

#[test]
fn test_unit_type() {
    let ty = parse_type("()").unwrap();
    assert!(matches!(ty.kind, TypeKind::Unit));
}

#[test]
fn test_inferred_type() {
    let ty = parse_type("_").unwrap();
    assert!(matches!(ty.kind, TypeKind::Inferred));
}

#[test]
fn test_named_type() {
    assert_parses("Vec");
    assert_parses("Option");
    assert_parses("Result");
}

#[test]
fn test_path_type() {
    assert_parses("std.Vec");
    assert_parses("collections.HashMap");
    assert_parses("io.Result");
}

#[test]
fn test_qualified_path() {
    assert_parses("std.collections.HashMap");
}

// ============================================================================
// SECTION 2: GENERIC TYPES (~10 tests)
// ============================================================================

#[test]
fn test_generic_single_param() {
    assert_parses("Vec<Int>");
    assert_parses("Option<String>");
    assert_parses("Box<Float>");
}

#[test]
fn test_generic_two_params() {
    assert_parses("HashMap<String, Int>");
    assert_parses("Result<Int, String>");
}

#[test]
fn test_generic_three_params() {
    assert_parses("Triple<Int, String, Bool>");
}

#[test]
fn test_nested_generics() {
    assert_parses("Vec<Vec<Int>>");
    assert_parses("Option<Result<Int, String>>");
    assert_parses("HashMap<String, Vec<Int>>");
}

#[test]
fn test_deeply_nested_generics() {
    assert_parses("Vec<HashMap<String, Option<Result<Int, String>>>>");
}

#[test]
fn test_generic_with_trailing_comma() {
    assert_parses("Vec<Int,>");
    assert_parses("HashMap<String, Int,>");
}

#[test]
fn test_const_generic() {
    assert_parses("Array<Int, 10>");
}

#[test]
fn test_const_generic_expression() {
    assert_parses("Array<Int, 5>");
}

#[test]
fn test_mixed_type_const_generic() {
    assert_parses("Matrix<Float, 3, 4>");
}

#[test]
fn test_generic_with_path() {
    assert_parses("std.Vec<Int>");
    assert_parses("collections.HashMap<String, Int>");
}

// ============================================================================
// SECTION 3: REFERENCE TYPES (~10 tests)
// ============================================================================

#[test]
fn test_cbgr_immutable_reference() {
    let ty = parse_type("&Int").unwrap();
    assert!(matches!(
        ty.kind,
        TypeKind::Reference { mutable: false, .. }
    ));
}

#[test]
fn test_cbgr_mutable_reference() {
    let ty = parse_type("&mut Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Reference { mutable: true, .. }));
}

#[test]
fn test_ownership_immutable_reference() {
    let ty = parse_type("%Int").unwrap();
    assert!(matches!(
        ty.kind,
        TypeKind::Ownership { mutable: false, .. }
    ));
}

#[test]
fn test_ownership_mutable_reference() {
    let ty = parse_type("%mut Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Ownership { mutable: true, .. }));
}

#[test]
fn test_reference_to_generic() {
    assert_parses("&Vec<Int>");
    assert_parses("&mut HashMap<String, Int>");
}

#[test]
fn test_reference_to_reference() {
    assert_parses("&&Int");
    assert_parses("&mut &Int");
}

#[test]
fn test_raw_pointer_const() {
    let ty = parse_type("*const Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Pointer { mutable: false, .. }));
}

#[test]
fn test_raw_pointer_mut() {
    let ty = parse_type("*mut Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Pointer { mutable: true, .. }));
}

#[test]
fn test_ownership_to_generic() {
    assert_parses("%Vec<Int>");
    assert_parses("%mut Option<String>");
}

#[test]
fn test_mixed_references() {
    assert_parses("&%Int");
    assert_parses("%&Int");
}

// ============================================================================
// SECTION 4: FUNCTION TYPES (~15 tests)
// ============================================================================

#[test]
fn test_function_no_params() {
    let ty = parse_type("fn() -> Int").unwrap();
    assert!(matches!(ty.kind, TypeKind::Function { .. }));
}

#[test]
fn test_function_single_param() {
    assert_parses("fn(Int) -> String");
}

#[test]
fn test_function_two_params() {
    assert_parses("fn(Int, String) -> Bool");
}

#[test]
fn test_function_three_params() {
    assert_parses("fn(Int, String, Bool) -> Float");
}

#[test]
fn test_function_no_return() {
    assert_parses("fn(Int)");
    assert_parses("fn(Int, String)");
}

#[test]
fn test_function_unit_return() {
    assert_parses("fn(Int) -> ()");
}

#[test]
fn test_function_generic_params() {
    assert_parses("fn(Vec<Int>) -> Option<String>");
}

#[test]
fn test_function_reference_params() {
    assert_parses("fn(&Int, &mut String) -> Bool");
}

#[test]
fn test_higher_order_function() {
    assert_parses("fn(fn(Int) -> String) -> Bool");
}

#[test]
fn test_function_returning_function() {
    assert_parses("fn(Int) -> fn(String) -> Bool");
}

#[test]
fn test_complex_higher_order() {
    assert_parses("fn(fn(Int, String) -> Bool, Vec<Int>) -> Option<String>");
}

#[test]
fn test_function_with_trailing_comma() {
    assert_parses("fn(Int, String,) -> Bool");
}

#[test]
fn test_function_complex_nested() {
    assert_parses("fn(HashMap<String, Vec<Int>>, &mut Option<Result<Int, String>>) -> Bool");
}

#[test]
fn test_function_with_effects() {
    // Note: Contexts are part of the function type, declared after return type
    assert_parses("fn(Int) -> String using [IO]");
}

#[test]
fn test_function_with_multiple_effects() {
    assert_parses("fn(Int, String) -> Result<Int, String> using [IO, Database, Logging]");
}

// ============================================================================
// SECTION 5: REFINEMENT TYPES (~20 tests) - CRITICAL P0!
// ============================================================================

#[test]
fn test_refinement_simple_greater() {
    let ty = parse_type("Int{> 0}").unwrap();
    assert!(matches!(ty.kind, TypeKind::Refined { .. }));
}

#[test]
fn test_refinement_simple_less() {
    assert_parses("Int{< 100}");
}

#[test]
fn test_refinement_greater_equal() {
    assert_parses("Int{>= 0}");
}

#[test]
fn test_refinement_less_equal() {
    assert_parses("Int{<= 100}");
}

#[test]
fn test_refinement_equality() {
    assert_parses("Int{== 42}");
}

#[test]
fn test_refinement_not_equal() {
    assert_parses("Int{!= 0}");
}

#[test]
fn test_refinement_range() {
    assert_parses("Int{>= 0 && it <= 100}");
}

#[test]
fn test_refinement_complex_predicate() {
    assert_parses("Int{> 0 && it < 100 && it % 2 == 0}");
}

#[test]
fn test_refinement_string() {
    assert_parses("String{len(it) > 0}");
}

#[test]
fn test_refinement_string_method() {
    assert_parses("String{it.contains('@')}");
}

#[test]
fn test_refinement_string_complex() {
    assert_parses("String{it.contains('@') && it.len() > 3}");
}

#[test]
fn test_refinement_on_generic() {
    assert_parses("Vec<Int>{len(it) > 0}");
}

#[test]
fn test_refinement_nested_generic() {
    assert_parses("Vec<String>{len(it) > 0 && it[0].len() > 0}");
}

#[test]
fn test_refinement_float_range() {
    assert_parses("Float{0.0 <= it && it <= 100.0}");
}

#[test]
fn test_refinement_with_function_call() {
    assert_parses("Vec<Int>{is_sorted(it)}");
}

#[test]
fn test_refinement_logical_or() {
    assert_parses("Int{it < 0 || it > 10}");
}

#[test]
fn test_refinement_arithmetic() {
    assert_parses("Int{it * 2 > 100}");
}

#[test]
fn test_refinement_on_reference() {
    assert_parses("&Int{> 0}");
}

#[test]
fn test_refinement_on_function_param() {
    assert_parses("fn(Int{> 0}) -> String");
}

#[test]
fn test_refinement_on_function_return() {
    assert_parses("fn(Int) -> Int{>= 0}");
}

// ============================================================================
// SECTION 5.4.1: NESTED REFINEMENT TYPES IN GENERIC ARGUMENTS
// ============================================================================
// Tests for refinement types nested inside generic arguments like Option<Int{> 0}>
// The parser must track brace depth to avoid treating > inside {> 0} as generic close

#[test]
fn test_nested_refinement_option_int_gt() {
    assert_parses("Option<Int{> 0}>");
}

#[test]
fn test_nested_refinement_option_int_gte() {
    assert_parses("Option<Int{>= 1}>");
}

#[test]
fn test_nested_refinement_list_float() {
    assert_parses("List<Float{>= 0.0}>");
}

#[test]
fn test_nested_refinement_map() {
    assert_parses("Map<Text, Int{!= 0}>");
}

#[test]
fn test_nested_refinement_vec_string() {
    assert_parses("Vec<String{len(it) > 0}>");
}

#[test]
fn test_nested_refinement_multiple_args() {
    assert_parses("Result<Int{> 0}, String{len(it) > 0}>");
}

#[test]
fn test_nested_refinement_double_nested() {
    assert_parses("Vec<Option<Int{> 0}>>");
}

#[test]
fn test_nested_refinement_complex_predicate() {
    assert_parses("Option<Int{> 0 && < 100}>");
}

#[test]
fn test_nested_refinement_triple_nested() {
    assert_parses("Vec<List<Option<Int{>= 0}>>>");
}

#[test]
fn test_nested_refinement_with_implicit_it() {
    assert_parses("Maybe<Int{> 0, < 100}>");
}

// ============================================================================
// SECTION 5.5: LAMBDA REFINEMENT TYPES (Rule 2 + Rule 5)
// ============================================================================
// Five Binding Rules: inline {pred}, declarative `where pred`, sigma `n: T where f(n)`
// Rule 2: Lambda Refinements - Explicit Parameters: type Positive is Int where |x| x > 0
// Rule 5: Bare `where` - Backward Compatibility: type Positive is Int where it > 0

#[test]
fn test_lambda_refinement_simple() {
    // Rule 2: Lambda with explicit parameter
    assert_parses("Int where |x| x > 0");
}

#[test]
fn test_lambda_refinement_with_binding() {
    let ty = parse_type("Int where |x| x > 0").unwrap();
    // Should parse as Refined type with RefinementPredicate
    if let TypeKind::Refined { predicate, .. } = &ty.kind {
        // The predicate should have a binding
        assert!(
            predicate.binding.is_some(),
            "Lambda refinement should have explicit binding"
        );
    } else {
        panic!("Expected Refined type for lambda refinement");
    }
}

#[test]
fn test_lambda_refinement_text() {
    // Rule 2: Lambda with text type
    assert_parses("Text where |s| s.len() > 5");
}

#[test]
fn test_lambda_refinement_complex_predicate() {
    assert_parses("Int where |x| x > 0 && x < 100");
}

#[test]
fn test_lambda_refinement_with_function_call() {
    assert_parses("Text where |s| is_email(s)");
}

#[test]
fn test_lambda_refinement_multiple_conditions() {
    assert_parses("Text where |p| p.len() > 5 && p.contains('@')");
}

#[test]
fn test_implicit_it_refinement() {
    // Rule 5: Bare where with implicit 'it' binding (backward compatibility)
    assert_parses("Int where it > 0");
}

#[test]
fn test_implicit_it_refinement_complex() {
    assert_parses("Int where it >= 0 && it <= 100");
}

#[test]
fn test_implicit_it_refinement_text() {
    assert_parses("Text where it.contains('@')");
}

#[test]
fn test_implicit_it_refinement_with_binding() {
    let ty = parse_type("Int where it > 0").unwrap();
    // Should parse as Refined type without explicit binding
    if let TypeKind::Refined { predicate, .. } = &ty.kind {
        // The predicate should NOT have a binding (uses implicit 'it')
        assert!(
            predicate.binding.is_none(),
            "Bare where refinement should use implicit 'it'"
        );
    } else {
        panic!("Expected Refined type for implicit it refinement");
    }
}

#[test]
fn test_lambda_refinement_on_generic() {
    assert_parses("Vec<Int> where |v| v.len() > 0");
}

#[test]
fn test_lambda_refinement_on_reference() {
    assert_parses("&Int where |x| x > 0");
}

#[test]
fn test_lambda_refinement_negative_number() {
    assert_parses("Int where |n| n >= -10 && n <= 10");
}

#[test]
fn test_lambda_refinement_float_range() {
    assert_parses("Float where |f| f >= 0.0 && f <= 1.0");
}

#[test]
fn test_lambda_refinement_logical_operators() {
    assert_parses("Int where |n| n > 0 || n == -1");
}

#[test]
fn test_complex_lambda_refinement_password() {
    // Real-world example: password validation
    assert_parses("Text where |p| p.len() >= 8 && p.contains('@') && p.contains('!')");
}

#[test]
fn test_complex_lambda_refinement_age() {
    // Real-world example: age validation
    assert_parses("Int where |age| age >= 0 && age <= 150");
}

#[test]
fn test_lambda_refinement_array() {
    assert_parses("[Int; 10] where |arr| arr.len() == 10");
}

#[test]
fn test_mixed_lambda_and_inline_refinements() {
    // Inline refinement on base type
    assert_parses("Int{> 0} where |x| x < 100");
}

#[test]
fn test_lambda_refinement_with_different_param_names() {
    // Different parameter names should all parse fine
    assert_parses("Int where |n| n > 0");
    assert_parses("Int where |x| x > 0");
    assert_parses("Int where |value| value > 0");
    assert_parses("Text where |s| s.len() > 0");
    assert_parses("Text where |str| str.contains('@')");
}

#[test]
fn test_lambda_refinement_on_function_param() {
    assert_parses("fn(Int where |x| x > 0) -> Bool");
}

#[test]
fn test_lambda_refinement_on_function_return() {
    assert_parses("fn(Int) -> Int where |result| result > 0");
}

#[test]
fn test_lambda_refinement_on_function_return_various() {
    // Simple positive constraint
    assert_parses("fn(Int) -> Int where |n| n > 0");

    // Range constraint with logical operators
    assert_parses("fn(Int, Int) -> Int where |result| result >= 0 && result <= 100");

    // Text refinement
    assert_parses("fn(Text) -> Text where |s| s.len() > 0");

    // Boolean predicate
    assert_parses("fn(Int) -> Bool where |b| b == true");

    // Multiple parameters with refined return
    assert_parses("fn(Int, Int, Int) -> Int where |sum| sum > 0");
}

// ============================================================================
// SECTION 6: PROTOCOL TYPES (~5 tests)
// ============================================================================

#[test]
fn test_protocol_single() {
    assert_parses("impl Display");
}

#[test]
fn test_protocol_multiple() {
    assert_parses("impl Display + Debug");
}

#[test]
fn test_protocol_three_traits() {
    assert_parses("impl Display + Debug + Clone");
}

#[test]
fn test_protocol_with_path() {
    assert_parses("impl std.Display");
}

#[test]
fn test_protocol_multiple_with_paths() {
    assert_parses("impl std.Display + std.Debug");
}

// ============================================================================
// SECTION 7: TUPLE TYPES (~5 tests)
// ============================================================================

#[test]
fn test_tuple_two_elements() {
    let ty = parse_type("(Int, String)").unwrap();
    assert!(matches!(ty.kind, TypeKind::Tuple(_)));
}

#[test]
fn test_tuple_three_elements() {
    assert_parses("(Int, String, Bool)");
}

#[test]
fn test_tuple_many_elements() {
    assert_parses("(Int, String, Bool, Float, Char)");
}

#[test]
fn test_tuple_nested() {
    assert_parses("((Int, String), Bool)");
}

#[test]
fn test_tuple_with_generics() {
    assert_parses("(Vec<Int>, HashMap<String, Int>)");
}

// ============================================================================
// SECTION 8: ARRAY AND SLICE TYPES (~5 tests)
// ============================================================================

#[test]
fn test_array_with_size() {
    let ty = parse_type("[Int; 10]").unwrap();
    assert!(matches!(ty.kind, TypeKind::Array { .. }));
}

#[test]
fn test_array_variable_size() {
    assert_parses("[Int; n]");
}

#[test]
fn test_slice_type() {
    let ty = parse_type("[Int]").unwrap();
    assert!(matches!(ty.kind, TypeKind::Slice(_)));
}

#[test]
fn test_array_of_generic() {
    assert_parses("[Vec<Int>; 5]");
}

#[test]
fn test_nested_arrays() {
    assert_parses("[[Int; 5]; 3]");
}

// ============================================================================
// SECTION 9: COMPLEX NESTED TYPES (~10 tests)
// ============================================================================

#[test]
fn test_complex_nested_1() {
    assert_parses("Vec<HashMap<String, Result<Int, String>>>");
}

#[test]
fn test_complex_nested_2() {
    assert_parses("Option<&Vec<&mut String>>");
}

#[test]
fn test_complex_nested_3() {
    assert_parses("fn(Vec<HashMap<String, Int>>) -> Option<Result<String, Int>>");
}

#[test]
fn test_complex_nested_4() {
    assert_parses("HashMap<String, fn(Int) -> Vec<String>>");
}

#[test]
fn test_complex_nested_5() {
    assert_parses("Vec<(Int, String, HashMap<String, Vec<Int>>)>");
}

#[test]
fn test_complex_with_refinements_1() {
    assert_parses("Vec<Int{> 0}>");
}

#[test]
fn test_complex_with_refinements_2() {
    assert_parses("HashMap<String{len(it) > 0}, Int{>= 0}>");
}

#[test]
fn test_complex_with_refinements_3() {
    assert_parses("fn(Int{> 0}, String{len(it) > 0}) -> Int{>= 0}");
}

#[test]
fn test_complex_with_references_and_refinements() {
    assert_parses("&Vec<Int{> 0}>");
}

#[test]
fn test_ultra_complex() {
    assert_parses(
        "fn(&Vec<HashMap<String{len(it) > 0}, Result<Int{>= 0}, String>>>) -> Option<Vec<Int{> 0}>> using [IO, Database]",
    );
}

// ============================================================================
// SECTION 10: ERROR CASES (~10 tests)
// ============================================================================

#[test]
fn test_error_unclosed_generic() {
    assert_fails("Vec<Int");
}

#[test]
fn test_error_unclosed_refinement() {
    assert_fails("Int{> 0");
}

#[test]
fn test_error_empty_generic() {
    assert_fails("Vec<>");
}

#[test]
fn test_error_empty_generic_map() {
    assert_fails("Map<>");
}

#[test]
fn test_error_empty_generic_list() {
    assert_fails("List<>");
}

#[test]
fn test_error_empty_generic_option() {
    assert_fails("Option<>");
}

#[test]
fn test_error_empty_tuple() {
    // Empty tuple should parse as unit type ()
    let ty = parse_type("()").unwrap();
    assert!(matches!(ty.kind, TypeKind::Unit));
}

#[test]
fn test_error_single_element_tuple() {
    // Single element tuple is not valid - should parse as parenthesized type
    assert_parses("(Int)");
}

#[test]
fn test_error_unclosed_paren() {
    assert_fails("(Int, String");
}

#[test]
fn test_error_unclosed_bracket() {
    assert_fails("[Int; 10");
}

#[test]
fn test_error_invalid_refinement() {
    // Refinement needs at least an expression
    assert_fails("Int{}");
}

#[test]
fn test_error_double_mut() {
    assert_fails("&mut mut Int");
}

#[test]
fn test_error_invalid_generic_separator() {
    assert_fails("Vec<Int; String>");
}

// ============================================================================
// SECTION 11: EDGE CASES AND SPECIAL SYNTAX (~5 tests)
// ============================================================================

#[test]
fn test_self_type() {
    assert_parses("Self");
}

#[test]
fn test_self_value_in_path() {
    assert_parses("self.Type");
}

#[test]
fn test_super_in_path() {
    assert_parses("super.Type");
}

#[test]
fn test_cog_in_path() {
    assert_parses("cog.Type");
}

#[test]
fn test_very_long_path() {
    assert_parses("cog.module.submodule.types.MyType");
}

// ============================================================================
// SECTION 12: REAL-WORLD EXAMPLES (~10 tests)
// ============================================================================

#[test]
fn test_real_world_positive_int() {
    assert_parses("Int{> 0}");
}

#[test]
fn test_real_world_percentage() {
    assert_parses("Float{0.0 <= it && it <= 100.0}");
}

#[test]
fn test_real_world_non_empty_string() {
    assert_parses("String{len(it) > 0}");
}

#[test]
fn test_real_world_email() {
    assert_parses("String{it.contains('@') && it.len() > 3}");
}

#[test]
fn test_real_world_port() {
    assert_parses("Int{1 <= it && it <= 65535}");
}

#[test]
fn test_real_world_age() {
    assert_parses("Int{0 <= it && it <= 150}");
}

#[test]
fn test_real_world_non_empty_vec() {
    assert_parses("Vec<Int>{len(it) > 0}");
}

#[test]
fn test_real_world_sorted_vec() {
    assert_parses("Vec<Int>{is_sorted(it)}");
}

#[test]
fn test_real_world_bounded_string() {
    assert_parses("String{it.len() >= 3 && it.len() <= 20}");
}

#[test]
fn test_real_world_safe_index() {
    assert_parses("fn(Vec<Int>, Int{>= 0}) -> Option<Int>");
}

// ============================================================================
// SUMMARY
// ============================================================================

// Total test count: ~140 tests
// - Primitive types: 10 tests
// - Generic types: 10 tests
// - Reference types: 10 tests
// - Function types: 15 tests
// - Refinement types: 20 tests (CRITICAL P0!)
// - Protocol types: 5 tests
// - Tuple types: 5 tests
// - Array/Slice types: 5 tests
// - Complex nested types: 10 tests
// - Error cases: 10 tests
// - Edge cases: 5 tests
// - Real-world examples: 10 tests
//
// This comprehensive test suite ensures that the type parser correctly
// handles all Verum type syntax, with special emphasis on refinement types
// (Verum's unique value proposition).

// Test for named predicate syntax in inline refinements
#[test]
fn test_inline_refinement_with_named_predicates() {
    // Grammar: refinement_predicate = identifier , ':' , expression | expression ;
    assert_parses("Int { value: self }");
    assert_parses("Int { min: self >= 0 }");
    assert_parses("Int { min: self >= 0, max: self <= 100 }");
    assert_parses("Int { value: self, min: self >= 0, max: self <= 100 }");
}
