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
// Unit tests for suggestion.rs
//
// Migrated from src/suggestion.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::suggestion::*;

#[test]
fn test_code_snippet() {
    let snippet = CodeSnippet::new("x: Int{> 0}");
    assert_eq!(snippet.language, "verum");
    assert_eq!(snippet.code, "x: Int{> 0}");
    assert!(snippet.span.is_none());
}

#[test]
fn test_suggestion_builder() {
    let suggestion = SuggestionBuilder::new("Add refinement constraint")
        .description("Add a constraint to ensure safety")
        .code("x: Int{> 0}")
        .recommended()
        .build();

    assert_eq!(suggestion.title(), "Add refinement constraint");
    assert!(suggestion.description().is_some());
    assert!(suggestion.snippet().is_some());
    assert_eq!(suggestion.applicability(), Applicability::Recommended);
}

#[test]
fn test_applicability_safety() {
    assert!(Applicability::Recommended.is_safe_to_apply());
    assert!(Applicability::Alternative.is_safe_to_apply());
    assert!(!Applicability::MaybeIncorrect.is_safe_to_apply());
    assert!(!Applicability::HasPlaceholders.is_safe_to_apply());
}

#[test]
fn test_template_refinement() {
    let suggestion = templates::add_refinement_constraint("x", "> 0");
    assert_eq!(suggestion.applicability(), Applicability::Recommended);
    assert!(suggestion.snippet().is_some());
}

#[test]
fn test_template_runtime_check() {
    let suggestion = templates::runtime_check("x > 0", "return Err(...)");
    assert_eq!(suggestion.applicability(), Applicability::Alternative);
}

// === TypeSuggestionTemplates Tests ===

#[test]
fn test_type_add_type_annotation() {
    let suggestion = TypeSuggestionTemplates::add_type_annotation("x", "Int");
    assert!(suggestion.title().contains("type annotation"));
    assert_eq!(suggestion.applicability(), Applicability::Recommended);
    assert!(suggestion.code().contains("let x: Int"));
}

#[test]
fn test_type_use_type_conversion() {
    let suggestion =
        TypeSuggestionTemplates::use_type_conversion("Int", "Text", "value.to_string()");
    assert!(suggestion.title().contains("Convert"));
    assert!(suggestion.code().contains("to_string"));
}

#[test]
fn test_type_use_refinement_type() {
    let suggestion = TypeSuggestionTemplates::use_refinement_type("Int", "x > 0");
    assert!(suggestion.title().contains("refinement"));
    assert!(suggestion.code().contains("where"));
    assert_eq!(suggestion.applicability(), Applicability::Alternative);
}

#[test]
fn test_type_wrap_in_maybe() {
    let suggestion = TypeSuggestionTemplates::wrap_in_maybe("value");
    assert!(suggestion.title().contains("Maybe"));
    assert!(suggestion.code().contains("Maybe::Some"));
}

#[test]
fn test_type_unwrap_with_default() {
    let suggestion = TypeSuggestionTemplates::unwrap_with_default("opt", "0");
    assert!(suggestion.title().contains("default"));
    assert!(suggestion.code().contains("unwrap_or(0)"));
}

#[test]
fn test_type_use_result_type() {
    let suggestion = TypeSuggestionTemplates::use_result_type("parse_data", "ParseError");
    assert!(suggestion.title().contains("Result"));
    assert!(suggestion.code().contains("Result<T, ParseError>"));
}

#[test]
fn test_type_add_generic_parameter() {
    let suggestion = TypeSuggestionTemplates::add_generic_parameter("process", "T", Some("Clone"));
    assert!(suggestion.title().contains("generic"));
    assert!(suggestion.code().contains("<T: Clone>"));
}

#[test]
fn test_type_use_associated_type() {
    let suggestion = TypeSuggestionTemplates::use_associated_type("Iterator", "Item");
    assert!(suggestion.title().contains("associated type"));
    assert!(suggestion.code().contains("<Self as Iterator>::Item"));
}

// === ErrorHandlingSuggestionTemplates Tests ===

#[test]
fn test_error_use_question_mark() {
    let suggestion = ErrorHandlingSuggestionTemplates::use_question_mark_operator("result");
    assert!(suggestion.title().contains("?"));
    assert!(suggestion.code().contains("result?"));
}

#[test]
fn test_error_match_result() {
    let suggestion = ErrorHandlingSuggestionTemplates::match_result("result");
    assert!(suggestion.title().contains("match"));
    assert!(suggestion.code().contains("Ok(value)"));
    assert!(suggestion.code().contains("Err(e)"));
}

#[test]
fn test_error_use_if_let() {
    let suggestion = ErrorHandlingSuggestionTemplates::use_if_let("maybe_value", "v");
    assert!(suggestion.title().contains("if let"));
    assert!(suggestion.code().contains("if let Maybe::Some(v)"));
}

#[test]
fn test_error_convert_error_type() {
    let suggestion = ErrorHandlingSuggestionTemplates::convert_error_type("IoError", "AppError");
    assert!(suggestion.title().contains("Convert"));
    assert!(suggestion.code().contains("map_err"));
    assert!(suggestion.code().contains("AppError::from"));
}

#[test]
fn test_error_add_error_context() {
    let suggestion = ErrorHandlingSuggestionTemplates::add_error_context("failed to read file");
    assert!(suggestion.title().contains("context"));
    assert!(suggestion.code().contains(".context("));
}

#[test]
fn test_error_use_unwrap_or_else() {
    let suggestion =
        ErrorHandlingSuggestionTemplates::use_unwrap_or_else("result", "default_value()");
    assert!(suggestion.title().contains("unwrap_or_else"));
    assert!(suggestion.code().contains("unwrap_or_else"));
}

#[test]
fn test_error_handle_must_handle_result() {
    let suggestion =
        ErrorHandlingSuggestionTemplates::handle_must_handle_result("db_result", "DbError");
    assert!(suggestion.title().contains("@must_handle"));
    assert!(suggestion.code().contains("match db_result"));
}

// === SyntaxSuggestionTemplates Tests ===

#[test]
fn test_syntax_add_missing_delimiter() {
    let suggestion = SyntaxSuggestionTemplates::add_missing_delimiter(";", "at end of statement");
    assert!(suggestion.title().contains(";"));
    assert!(suggestion.code() == ";");
}

#[test]
fn test_syntax_remove_extra_token() {
    let suggestion = SyntaxSuggestionTemplates::remove_extra_token(";;");
    assert!(suggestion.title().contains("Remove"));
    assert!(suggestion.code().is_empty());
}

#[test]
fn test_syntax_replace_token() {
    let suggestion = SyntaxSuggestionTemplates::replace_token("=", "==");
    assert!(suggestion.title().contains("Replace"));
    assert!(suggestion.code() == "==");
}

#[test]
fn test_syntax_add_function_element() {
    let suggestion = SyntaxSuggestionTemplates::add_function_element("return type", "my_func");
    assert!(suggestion.title().contains("return type"));
    assert_eq!(suggestion.applicability(), Applicability::HasPlaceholders);
}

#[test]
fn test_syntax_fix_indentation() {
    let suggestion = SyntaxSuggestionTemplates::fix_indentation(4);
    assert!(suggestion.title().contains("indentation"));
    assert_eq!(suggestion.code().len(), 4);
}

#[test]
fn test_syntax_add_module_declaration() {
    let suggestion = SyntaxSuggestionTemplates::add_module_declaration("utils");
    assert!(suggestion.title().contains("module"));
    assert!(suggestion.code().contains("mod utils;"));
}

#[test]
fn test_syntax_import_symbol() {
    let suggestion = SyntaxSuggestionTemplates::import_symbol("HashMap", "std::collections");
    assert!(suggestion.title().contains("Import"));
    assert!(
        suggestion
            .code()
            .contains("using std::collections::HashMap")
    );
}

// === PerformanceSuggestionTemplates Tests ===

#[test]
fn test_perf_use_iterator() {
    let suggestion = PerformanceSuggestionTemplates::use_iterator("items");
    assert!(suggestion.title().contains("iterator"));
    assert!(suggestion.code().contains(".iter()"));
}

#[test]
fn test_perf_use_with_capacity() {
    let suggestion = PerformanceSuggestionTemplates::use_with_capacity("Vec", "expected_size");
    assert!(suggestion.title().contains("capacity"));
    assert!(suggestion.code().contains("with_capacity"));
}

#[test]
fn test_perf_use_borrow_instead_of_clone() {
    let suggestion = PerformanceSuggestionTemplates::use_borrow_instead_of_clone("data");
    assert!(suggestion.title().contains("borrow"));
    assert!(suggestion.code().contains("&data"));
}

#[test]
fn test_perf_move_instead_of_clone() {
    let suggestion = PerformanceSuggestionTemplates::move_instead_of_clone("value");
    assert!(suggestion.title().contains("Move"));
    assert_eq!(suggestion.code(), "value");
}

#[test]
fn test_perf_use_lazy_evaluation() {
    let suggestion = PerformanceSuggestionTemplates::use_lazy_evaluation("expensive_compute()");
    assert!(suggestion.title().contains("lazy"));
    assert!(suggestion.code().contains("lazy {"));
}

// === Applicability Tests ===

#[test]
fn test_applicability_safety_levels() {
    // Test that applicability levels are correctly categorized
    assert!(Applicability::Recommended.is_safe_to_apply());
    assert!(Applicability::Alternative.is_safe_to_apply());
    assert!(!Applicability::MaybeIncorrect.is_safe_to_apply());
    assert!(!Applicability::HasPlaceholders.is_safe_to_apply());
}

// === SuggestionBuilder Edge Cases ===

#[test]
fn test_suggestion_builder_minimal() {
    let suggestion = SuggestionBuilder::new("Minimal suggestion").build();
    assert_eq!(suggestion.title(), "Minimal suggestion");
    // Default applicability is Alternative
    assert_eq!(suggestion.applicability(), Applicability::Alternative);
}

#[test]
fn test_suggestion_builder_all_options() {
    let suggestion = SuggestionBuilder::new("Full suggestion")
        .description("Detailed description")
        .code("let x = 42;")
        .applicability(Applicability::Alternative)
        .build();

    assert_eq!(suggestion.title(), "Full suggestion");
    assert_eq!(suggestion.description(), Some("Detailed description"));
    assert_eq!(suggestion.applicability(), Applicability::Alternative);

    let snippet = suggestion.snippet().unwrap();
    assert_eq!(snippet.code, "let x = 42;");
}

#[test]
fn test_suggestion_multiple_snippets() {
    let snippet1 = CodeSnippet::new("first");
    let snippet2 = CodeSnippet::with_language("second", "python");

    let suggestion = SuggestionBuilder::new("Multi-snippet")
        .add_snippet(snippet1)
        .add_snippet(snippet2)
        .build();

    // After adding snippets, can iterate through all snippets
    let all_snippets: Vec<_> = suggestion.all_snippets().collect();
    assert_eq!(all_snippets.len(), 2);
}

// === Templates Module Tests ===

#[test]
fn test_templates_use_safe_method() {
    let suggestion = templates::use_safe_method("get", "Returns Option instead of panicking");
    assert!(suggestion.title().contains("get"));
    assert!(suggestion.code().contains(".get()"));
}

#[test]
fn test_templates_compile_time_proof() {
    let suggestion = templates::compile_time_proof("x > 0");
    assert!(suggestion.title().contains("proof"));
    assert!(suggestion.code().contains("@verify x > 0"));
}

#[test]
fn test_templates_strengthen_precondition() {
    let suggestion = templates::strengthen_precondition("idx", "< len");
    assert!(suggestion.title().contains("precondition"));
}

#[test]
fn test_templates_weaken_postcondition() {
    let suggestion = templates::weaken_postcondition("Int");
    assert!(suggestion.title().contains("postcondition"));
}
