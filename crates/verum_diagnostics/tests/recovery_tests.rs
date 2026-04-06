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
// Tests for recovery module
// Migrated from src/recovery.rs per CLAUDE.md standards

use verum_common::{List, Maybe, Text};
use verum_diagnostics::Applicability;
use verum_diagnostics::context_error::levenshtein_distance;
use verum_diagnostics::recovery::*;

#[test]
fn test_levenshtein_distance() {
    assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    assert_eq!(levenshtein_distance("hello", "hello"), 0);
    assert_eq!(levenshtein_distance("", "test"), 4);
}

#[test]
fn test_type_mismatch_suggestions() {
    let recovery = ErrorRecovery::new();
    let suggestions = recovery.suggest_fixes_for_type_mismatch("Int", "Text", "assignment");

    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().any(|s| s.description.contains("parse")));
}

#[test]
fn test_similar_name_suggestions() {
    let recovery = ErrorRecovery::new();
    let available: Vec<Text> = vec![
        Text::from("user_name"),
        Text::from("username"),
        Text::from("user_id"),
        Text::from("account"),
    ];

    let suggestions = recovery.suggest_similar_names("usrname", &available);

    assert!(!suggestions.is_empty());
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str() == "username" || s.as_str() == "user_name")
    );
}

#[test]
fn test_recovery_action_ranking() {
    let action1 = RecoveryAction {
        description: "Fix 1".into(),
        code_change: Maybe::Some("code".into()),
        confidence: 90,
        applicability: Applicability::Recommended,
    };

    let action2 = RecoveryAction {
        description: "Fix 2".into(),
        code_change: Maybe::None,
        confidence: 80,
        applicability: Applicability::MaybeIncorrect,
    };

    assert!(action1.ranking_score() > action2.ranking_score());
}

#[test]
fn test_partial_compilation() {
    let mut partial = PartialCompilation::new();

    partial.add_valid("item1".into());
    partial.add_valid("item2".into());
    partial.add_partial("item3".into());

    assert_eq!(partial.valid_items.len(), 2);
    assert_eq!(partial.partial_items.len(), 1);
    assert_eq!(partial.completion_percentage(), 66);
}

// === New comprehensive tests ===

#[test]
fn test_syntax_error_recovery_unexpected_token() {
    let recovery = ErrorRecovery::new();
    let context = SyntaxErrorContext {
        kind: SyntaxErrorKind::UnexpectedToken {
            expected: vec![";".into()].into(),
            found: "}".into(),
        },
        line: 10,
        column: 5,
        context: Some("let x = 5}".into()),
    };

    let suggestions = recovery.suggest_syntax_fixes(&context);
    assert!(!suggestions.is_empty());
    // Should suggest inserting semicolon
    assert!(suggestions.iter().any(|s| {
        s.code_change
            .as_ref()
            .map(|c| c.contains(";"))
            .unwrap_or(false)
    }));
}

#[test]
fn test_syntax_error_recovery_unexpected_eof() {
    let recovery = ErrorRecovery::new();
    let context = SyntaxErrorContext {
        kind: SyntaxErrorKind::UnexpectedEof {
            expected: vec!["}".into(), ";".into()].into(),
        },
        line: 20,
        column: 0,
        context: None,
    };

    let suggestions = recovery.suggest_syntax_fixes(&context);
    assert!(!suggestions.is_empty());
    // Should suggest closing brace and/or semicolon
    assert!(suggestions.iter().any(|s| {
        s.code_change
            .as_ref()
            .map(|c| c == "}" || c == ";")
            .unwrap_or(false)
    }));
}

#[test]
fn test_syntax_error_recovery_mismatched_delimiter() {
    let recovery = ErrorRecovery::new();
    let context = SyntaxErrorContext {
        kind: SyntaxErrorKind::MismatchedDelimiter {
            opening: "(".into(),
            closing: ")".into(),
        },
        line: 5,
        column: 10,
        context: Some("fn foo(x, y".into()),
    };

    let suggestions = recovery.suggest_syntax_fixes(&context);
    assert!(!suggestions.is_empty());
    // Should suggest adding closing paren
    assert!(suggestions.iter().any(|s| {
        s.description.contains("Add closing")
            && s.code_change.as_ref().map(|c| c == ")").unwrap_or(false)
    }));
}

#[test]
fn test_undefined_name_recovery_variable() {
    let recovery = ErrorRecovery::new();
    let available_names: Vec<Text> = vec!["username".into(), "user_id".into(), "name".into()];
    let available_types: Vec<Text> = vec![];

    let suggestions = recovery.suggest_fixes_for_undefined_name(
        "usrname",
        &available_names,
        &available_types,
        NameContext::Variable,
    );

    assert!(!suggestions.is_empty());
    // Should suggest similar name "username"
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("Did you mean"))
    );
    // Should also suggest declaring the variable
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("Declare variable"))
    );
}

#[test]
fn test_undefined_name_recovery_type() {
    let recovery = ErrorRecovery::new();
    let available_names: Vec<Text> = vec![];
    let available_types: Vec<Text> = vec![];

    // Test typo for built-in type "Int"
    let suggestions = recovery.suggest_fixes_for_undefined_name(
        "Intt",
        &available_names,
        &available_types,
        NameContext::Type,
    );

    assert!(!suggestions.is_empty());
    // Should suggest built-in type "Int"
    assert!(suggestions.iter().any(|s| {
        s.description.contains("built-in type")
            && s.code_change.as_ref().map(|c| c == "Int").unwrap_or(false)
    }));
}

#[test]
fn test_undefined_name_recovery_function() {
    let recovery = ErrorRecovery::new();
    let available_names: Vec<Text> = vec!["process_data".into(), "handle_request".into()];
    let available_types: Vec<Text> = vec![];

    let suggestions = recovery.suggest_fixes_for_undefined_name(
        "processData",
        &available_names,
        &available_types,
        NameContext::Function,
    );

    // Should suggest defining a new function
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("Define function"))
    );
}

#[test]
fn test_arity_mismatch_too_few_args() {
    let recovery = ErrorRecovery::new();
    let param_names: Vec<Text> = vec!["x".into(), "y".into(), "z".into()];

    let suggestions = recovery.suggest_fixes_for_arity_mismatch("my_function", 3, 1, &param_names);

    assert!(!suggestions.is_empty());
    // Should mention missing arguments
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("missing argument"))
    );
    // Should mention the expected count
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("expects 3"))
    );
}

#[test]
fn test_arity_mismatch_too_many_args() {
    let recovery = ErrorRecovery::new();
    let param_names: Vec<Text> = vec!["x".into()];

    let suggestions = recovery.suggest_fixes_for_arity_mismatch("my_function", 1, 5, &param_names);

    assert!(!suggestions.is_empty());
    // Should suggest removing extra arguments
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("Remove") && s.description.contains("extra"))
    );
}

#[test]
fn test_recovery_state_for_type_error() {
    let recovery = ErrorRecovery::new();
    let state = recovery.create_recovery_state(&ErrorKind::Type);

    assert!(state.can_continue);
    assert_eq!(state.severity, RecoverySeverity::Recoverable);
    assert_eq!(state.placeholder_type, "_");
}

#[test]
fn test_recovery_state_for_syntax_error() {
    let recovery = ErrorRecovery::new();
    let state = recovery.create_recovery_state(&ErrorKind::Syntax);

    // Syntax errors are fatal - can't continue
    assert!(!state.can_continue);
    assert_eq!(state.severity, RecoverySeverity::Fatal);
}

#[test]
fn test_recovery_state_for_semantic_error() {
    let recovery = ErrorRecovery::new();
    let state = recovery.create_recovery_state(&ErrorKind::Semantic);

    assert!(state.can_continue);
    assert_eq!(state.severity, RecoverySeverity::Warning);
}

#[test]
fn test_type_conversion_registration() {
    let mut recovery = ErrorRecovery::new();

    // Register a custom type conversion
    recovery.register_type_conversion(TypeConversion {
        from: "CustomType".into(),
        to: "Int".into(),
        template: "{value}.as_int()".into(),
        description: "Convert CustomType to Int".into(),
        confidence: 90,
        infallible: true,
    });

    // The registration should succeed (no panic)
    // In a full implementation, we'd test that the conversion is used
}

#[test]
fn test_type_mismatch_refinement() {
    let recovery = ErrorRecovery::new();

    // Test refinement type mismatch (types with constraints)
    let suggestions =
        recovery.suggest_fixes_for_type_mismatch("Int{x > 0}", "Int{x >= 0}", "assignment");

    assert!(!suggestions.is_empty());
    // Should suggest runtime check or @verify annotation
    assert!(
        suggestions.iter().any(|s| {
            s.description.contains("runtime check") || s.description.contains("@verify")
        })
    );
}

#[test]
fn test_type_mismatch_reference() {
    let recovery = ErrorRecovery::new();

    // Test reference mismatch (value vs reference)
    let suggestions = recovery.suggest_fixes_for_type_mismatch("&Int", "Int", "function_call");

    assert!(!suggestions.is_empty());
    // Should suggest adding reference
    assert!(
        suggestions
            .iter()
            .any(|s| s.description.contains("Add reference"))
    );
}

#[test]
fn test_type_mismatch_maybe() {
    let recovery = ErrorRecovery::new();

    // Test Maybe wrapping suggestion
    let suggestions = recovery.suggest_fixes_for_type_mismatch("Maybe<Int>", "Int", "return");

    assert!(!suggestions.is_empty());
    // Should suggest wrapping in Maybe
    assert!(suggestions.iter().any(|s| s.description.contains("Maybe")));
}

#[test]
fn test_recovery_with_experimental_disabled() {
    let recovery = ErrorRecovery::with_config(3, 30, false);
    // Should work normally without experimental features
    let suggestions = recovery.suggest_fixes_for_type_mismatch("Int", "Text", "assignment");
    assert!(!suggestions.is_empty());
}

#[test]
fn test_recovery_with_custom_edit_distance() {
    let recovery = ErrorRecovery::with_config(1, 30, false);
    let available: Vec<Text> = vec!["username".into(), "user".into()];

    // With max_edit_distance = 1, only very close matches should be found
    let suggestions = recovery.suggest_similar_names("usernme", &available);
    // "usernme" -> "username" is distance 2, so might not be included
    // with strict distance = 1

    // Just verify it doesn't panic and returns something reasonable
    assert!(suggestions.len() <= available.len());
}

// === Comprehensive Refinement Type Parsing Tests ===

#[test]
fn test_refinement_mismatch_brace_syntax() {
    let recovery = ErrorRecovery::new();

    // Same base type with different constraints
    let suggestions =
        recovery.suggest_fixes_for_type_mismatch("Int{x > 0}", "Int{x >= 0}", "assignment");

    // Should recognize as refinement mismatch and suggest appropriate fixes
    assert!(!suggestions.is_empty());
    assert!(
        suggestions.iter().any(|s| {
            s.description.contains("runtime check") || s.description.contains("@verify")
        })
    );
}

#[test]
fn test_refinement_mismatch_where_syntax() {
    let recovery = ErrorRecovery::new();

    // Test where-clause refinement syntax
    let suggestions = recovery.suggest_fixes_for_type_mismatch(
        "Float where x != 0.0",
        "Float where x > 0.0",
        "function_call",
    );

    assert!(!suggestions.is_empty());
}

#[test]
fn test_refinement_mismatch_refined_vs_unrefined() {
    let recovery = ErrorRecovery::new();

    // Refined type vs unrefined type with same base
    let suggestions = recovery.suggest_fixes_for_type_mismatch("Int{x > 0}", "Int", "return");

    // Should recognize this as refinement mismatch
    assert!(!suggestions.is_empty());
    // Should suggest wrapping or verifying
    assert!(suggestions.iter().any(|s| {
        s.description.contains("runtime check")
            || s.description.contains("@verify")
            || s.description.contains("constraint")
    }));
}

#[test]
fn test_refinement_mismatch_unrefined_vs_refined() {
    let recovery = ErrorRecovery::new();

    // Unrefined type vs refined type
    let suggestions = recovery.suggest_fixes_for_type_mismatch("Int", "Int{x != 0}", "return");

    assert!(!suggestions.is_empty());
}

#[test]
fn test_refinement_mismatch_complex_constraints() {
    let recovery = ErrorRecovery::new();

    // Multiple predicates in refinement
    let suggestions = recovery.suggest_fixes_for_type_mismatch(
        "Int{x > 0, x < 100}",
        "Int{x >= 0}",
        "assignment",
    );

    assert!(!suggestions.is_empty());
}

#[test]
fn test_non_refinement_mismatch_different_base_types() {
    let recovery = ErrorRecovery::new();

    // Different base types should NOT be treated as refinement mismatch
    // but should still provide conversion suggestions
    let suggestions =
        recovery.suggest_fixes_for_type_mismatch("Int{x > 0}", "Float{x > 0}", "assignment");

    // Should provide suggestions but not refinement-specific ones only
    assert!(!suggestions.is_empty());
}

#[test]
fn test_refinement_with_nested_generics() {
    let recovery = ErrorRecovery::new();

    // Generic type containing refinement
    let suggestions =
        recovery.suggest_fixes_for_type_mismatch("List<Int{x > 0}>", "List<Int>", "assignment");

    assert!(!suggestions.is_empty());
}

#[test]
fn test_refinement_mismatch_mixed_syntax() {
    let recovery = ErrorRecovery::new();

    // Mixing brace and where syntax (same base type)
    let suggestions =
        recovery.suggest_fixes_for_type_mismatch("Int{x > 0}", "Int where x >= 0", "assignment");

    assert!(!suggestions.is_empty());
}
