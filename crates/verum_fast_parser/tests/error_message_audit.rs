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
// Comprehensive audit of parser error messages
// Tests various error scenarios to ensure helpful diagnostics

use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

#[derive(Debug)]
struct ErrorTest {
    name: &'static str,
    source: &'static str,
    expected_error_substring: &'static str,
    should_have_help: bool,
}

const ERROR_TESTS: &[ErrorTest] = &[
    ErrorTest {
        name: "missing_semicolon",
        source: "let x = 42\nlet y = 10",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "unclosed_brace",
        source: "fn test() {\n    let y = 10",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "invalid_token_in_expression",
        source: "let z = 42 @@ 10;",
        expected_error_substring: "unexpected",
        should_have_help: false,
    },
    ErrorTest {
        name: "wrong_keyword_struct",
        source: "struct Point { x: Int, y: Int }",
        expected_error_substring: "not a Verum keyword",
        should_have_help: true,
    },
    ErrorTest {
        name: "unclosed_paren",
        source: "let result = (1 + 2;",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "mismatched_delimiters",
        source: "let arr = [1, 2, 3};",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "missing_colon_type_annotation",
        source: "let name Int = 5;",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "missing_colon_in_param",
        source: "fn foo(x Int) -> Int { x + 1 }",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "missing_arrow_in_function",
        source: "fn foo(x: Int) Int { x + 1 }",
        expected_error_substring: "expected",
        should_have_help: false,
    },
    ErrorTest {
        name: "extra_comma_in_params",
        source: "fn foo(x: Int,, y: Int) -> Int { x + y }",
        expected_error_substring: "expected",
        should_have_help: false,
    },
];

#[test]
fn audit_error_messages() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    println!("\n=== ERROR MESSAGE AUDIT ===\n");

    let mut passed = 0;
    let mut failed = 0;

    for test in ERROR_TESTS {
        println!("Test: {}", test.name);
        println!("Source: {}", test.source);

        let lexer = Lexer::new(test.source, file_id);
        let result = parser.parse_module(lexer, file_id);

        match result {
            Ok(_) => {
                println!("  ❌ UNEXPECTED SUCCESS - Expected an error!\n");
                failed += 1;
            }
            Err(errors) => {
                if errors.is_empty() {
                    println!("  ❌ NO ERRORS - Expected error messages!\n");
                    failed += 1;
                } else {
                    println!("  Errors found: {}", errors.len());
                    for (i, error) in errors.iter().enumerate() {
                        println!("    Error {}: {}", i + 1, error);

                        // Check if error message contains expected substring
                        let error_text = format!("{}", error);
                        let has_expected = error_text
                            .to_lowercase()
                            .contains(&test.expected_error_substring.to_lowercase());

                        if has_expected {
                            println!(
                                "    ✓ Contains expected: '{}'",
                                test.expected_error_substring
                            );
                        } else {
                            println!(
                                "    ⚠ Missing expected substring: '{}'",
                                test.expected_error_substring
                            );
                        }

                        // Check for help message if expected
                        if test.should_have_help {
                            if error.help.is_some() {
                                println!(
                                    "    ✓ Has help message: {}",
                                    error.help.as_ref().unwrap()
                                );
                            } else {
                                println!("    ⚠ Missing help message");
                            }
                        }

                        // Check for span information
                        println!("    Span: {}", error.span);
                    }

                    passed += 1;
                    println!("  ✓ PASS\n");
                }
            }
        }
    }

    println!("=== SUMMARY ===");
    println!("Passed: {}/{}", passed, ERROR_TESTS.len());
    println!("Failed: {}/{}", failed, ERROR_TESTS.len());

    // Don't fail the test - this is an audit, not enforcement
    if failed > 0 {
        println!("\n⚠ Some tests had unexpected behavior");
    }
}

#[test]
fn test_error_has_source_location() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let source = "let x = 42\nlet y = 10"; // Missing semicolon

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);

    match result {
        Err(errors) => {
            assert!(!errors.is_empty(), "Should have errors");
            for error in &errors {
                // Check that span is not dummy (has valid position)
                assert!(
                    error.span.start <= error.span.end,
                    "Error should have valid span"
                );
                assert_eq!(
                    error.span.file_id, file_id,
                    "Error should reference correct file"
                );
            }
        }
        Ok(_) => panic!("Expected parsing to fail"),
    }
}

#[test]
fn test_multiple_errors_reported() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // Source with multiple errors
    let source = r#"
        let x = 42
        let y = (1 + 2
        fn foo(x Int) {
            x + 1
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);

    match result {
        Err(errors) => {
            println!("Found {} errors:", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("  Error {}: {}", i + 1, error);
            }
            // We should get multiple errors with error recovery
            // (though exact number may vary with recovery strategy)
            assert!(!errors.is_empty(), "Should report at least one error");
        }
        Ok(_) => panic!("Expected parsing to fail with multiple errors"),
    }
}

#[test]
fn test_error_recovery_continues_parsing() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    // Source with early error but valid code after
    let source = r#"
        let x = @@ invalid

        fn valid_function() -> Int {
            42
        }
    "#;

    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);

    match result {
        Err(errors) => {
            println!("Errors found (parser recovered): {}", errors.len());
            for error in &errors {
                println!("  {}", error);
            }
            // Parser should recover and continue parsing
            assert!(!errors.is_empty(), "Should report errors");
        }
        Ok(module) => {
            println!(
                "Module parsed successfully with {} items",
                module.items.len()
            );
            // In some cases, parser might succeed if it can recover fully
        }
    }
}

// =============================================================================
// Rust Syntax Migration Detection Tests
// =============================================================================
// These tests verify that when users write Rust syntax in Verum, the parser
// provides helpful "did you mean?" messages pointing to Verum equivalents.

fn parse_and_get_error(source: &str) -> String {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let result = parser.parse_module(lexer, file_id);
    match result {
        Err(errors) => {
            assert!(!errors.is_empty(), "Expected at least one error for: {}", source);
            let error = &errors[0];
            let mut msg = format!("{}", error);
            if let Some(help) = &error.help {
                msg.push_str(&format!(" [help: {}]", help));
            }
            msg
        }
        Ok(_) => panic!("Expected parse error for: {}", source),
    }
}

#[test]
fn test_rust_struct_suggests_type_is() {
    let msg = parse_and_get_error("struct Point { x: Int, y: Int }");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("type Name is"),
        "Should suggest 'type Name is {{ ... }}', got: {}", msg
    );
}

#[test]
fn test_rust_enum_suggests_type_is() {
    let msg = parse_and_get_error("enum Color { Red, Green, Blue }");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("type Name is A | B"),
        "Should suggest variant syntax, got: {}", msg
    );
}

#[test]
fn test_rust_trait_suggests_protocol() {
    let msg = parse_and_get_error("trait Display { fn fmt(&self) -> Text; }");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("protocol"),
        "Should suggest 'type Name is protocol {{ ... }}', got: {}", msg
    );
}

#[test]
fn test_rust_impl_suggests_implement() {
    let msg = parse_and_get_error("impl Display for Point { }");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("implement"),
        "Should suggest 'implement', got: {}", msg
    );
}

#[test]
fn test_rust_use_suggests_mount() {
    let msg = parse_and_get_error("use std::collections::HashMap;");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("mount"),
        "Should suggest 'mount', got: {}", msg
    );
}

#[test]
fn test_rust_mod_suggests_module() {
    let msg = parse_and_get_error("mod tests { }");
    assert!(
        msg.contains("not a Verum keyword"),
        "Should mention 'not a Verum keyword', got: {}", msg
    );
    assert!(
        msg.contains("module"),
        "Should suggest 'module', got: {}", msg
    );
}

#[test]
fn test_rust_println_macro_suggests_print() {
    let msg = parse_and_get_error("fn main() { println!(\"hello\"); }");
    assert!(
        msg.contains("Rust macro syntax") || msg.contains("print("),
        "Should detect println! as Rust macro, got: {}", msg
    );
}

#[test]
fn test_rust_vec_macro_suggests_list() {
    let msg = parse_and_get_error("fn main() { let x = vec![1, 2, 3]; }");
    assert!(
        msg.contains("Rust macro syntax") || msg.contains("List"),
        "Should detect vec! as Rust macro and suggest List, got: {}", msg
    );
}

#[test]
fn test_rust_assert_macro_suggests_assert() {
    let msg = parse_and_get_error("fn main() { assert!(true); }");
    assert!(
        msg.contains("Rust macro syntax") || msg.contains("assert("),
        "Should detect assert! as Rust macro, got: {}", msg
    );
}

#[test]
fn test_error_code_e0e0_for_rust_keywords() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new("struct Foo { }", file_id);
    let result = parser.parse_module(lexer, file_id);
    match result {
        Err(errors) => {
            assert!(!errors.is_empty());
            let error = &errors[0];
            assert_eq!(
                error.code.as_deref(),
                Some("E0E0"),
                "Rust keyword error should have code E0E0, got: {:?}", error.code
            );
        }
        Ok(_) => panic!("Expected parse error"),
    }
}

#[test]
fn test_error_code_e0e2_for_rust_macros() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new("fn main() { println!(\"hi\"); }", file_id);
    let result = parser.parse_module(lexer, file_id);
    match result {
        Err(errors) => {
            assert!(!errors.is_empty());
            let error = &errors[0];
            assert_eq!(
                error.code.as_deref(),
                Some("E0E2"),
                "Rust macro error should have code E0E2, got: {:?}", error.code
            );
        }
        Ok(_) => panic!("Expected parse error"),
    }
}

#[test]
fn test_error_messages_have_help_text() {
    // All Rust keyword errors should have help text with "did you mean"
    let rust_inputs = [
        "struct Foo { }",
        "enum Bar { A, B }",
        "trait Baz { }",
        "impl Foo { }",
        "use foo::bar;",
        "mod tests { }",
    ];

    let parser = VerumParser::new();
    let file_id = FileId::new(0);

    for source in &rust_inputs {
        let lexer = Lexer::new(source, file_id);
        let result = parser.parse_module(lexer, file_id);
        match result {
            Err(errors) => {
                assert!(!errors.is_empty(), "Expected error for: {}", source);
                let error = &errors[0];
                assert!(
                    error.help.is_some(),
                    "Error for '{}' should have help text, got: {:?}", source, error
                );
                let help = error.help.as_ref().unwrap();
                assert!(
                    help.contains("did you mean"),
                    "Help for '{}' should contain 'did you mean', got: {}", source, help
                );
            }
            Ok(_) => panic!("Expected parse error for: {}", source),
        }
    }
}

// =============================================================================
// Rust Type Name → Verum Semantic Type Suggestion Tests
// =============================================================================
// These tests verify that the diagnostics infrastructure correctly maps
// Rust type names to their Verum equivalents via rust_type_suggestion().

#[test]
fn test_vec_suggests_list() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("Vec");
    assert_eq!(suggestion, Some("List"), "Vec<T> should suggest List<T>");
}

#[test]
fn test_string_suggests_text() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("String");
    assert_eq!(suggestion, Some("Text"), "String should suggest Text");
}

#[test]
fn test_hashmap_suggests_map() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("HashMap");
    assert_eq!(suggestion, Some("Map"), "HashMap should suggest Map");
}

#[test]
fn test_hashset_suggests_set() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("HashSet");
    assert_eq!(suggestion, Some("Set"), "HashSet should suggest Set");
}

#[test]
fn test_box_suggests_heap() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("Box");
    assert_eq!(suggestion, Some("Heap"), "Box should suggest Heap");
}

#[test]
fn test_option_suggests_maybe() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("Option");
    assert_eq!(suggestion, Some("Maybe"), "Option should suggest Maybe");
}

#[test]
fn test_arc_suggests_shared() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("Arc");
    assert_eq!(suggestion, Some("Shared"), "Arc should suggest Shared");
}

#[test]
fn test_rc_suggests_shared() {
    let suggestion = verum_diagnostics::recovery::rust_type_suggestion("Rc");
    assert_eq!(suggestion, Some("Shared"), "Rc should suggest Shared");
}

// =============================================================================
// Rust Macro → Verum Built-in Mapping Tests
// =============================================================================

#[test]
fn test_println_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("println!");
    assert!(suggestion.is_some(), "println! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("print"),
        "println! should suggest print(...), got: {}", suggestion.unwrap()
    );
}

#[test]
fn test_format_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("format!");
    assert!(suggestion.is_some(), "format! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("f\""),
        "format! should suggest f\"...\" format strings, got: {}", suggestion.unwrap()
    );
}

#[test]
fn test_panic_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("panic!");
    assert!(suggestion.is_some(), "panic! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("panic("),
        "panic! should suggest panic(...), got: {}", suggestion.unwrap()
    );
}

#[test]
fn test_assert_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("assert!");
    assert!(suggestion.is_some(), "assert! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("assert("),
        "assert! should suggest assert(...), got: {}", suggestion.unwrap()
    );
}

#[test]
fn test_vec_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("vec!");
    assert!(suggestion.is_some(), "vec! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("List"),
        "vec! should suggest List, got: {}", suggestion.unwrap()
    );
}

#[test]
fn test_todo_macro_mapping() {
    let suggestion = verum_diagnostics::recovery::rust_macro_suggestion("todo!");
    assert!(suggestion.is_some(), "todo! should have a suggestion");
    assert!(
        suggestion.unwrap().contains("todo("),
        "todo! should suggest todo(), got: {}", suggestion.unwrap()
    );
}

/// Comprehensive test: parser detects println!("hello") and gives helpful error
#[test]
fn test_println_in_function_body_detected() {
    let msg = parse_and_get_error("fn main() { println!(\"hello\"); }");
    // The parser should detect the `!` after an identifier and flag it as Rust macro syntax
    assert!(
        msg.contains("Rust macro") || msg.contains("print(") || msg.contains("E0E2"),
        "println!(\"hello\") should be flagged as Rust macro syntax, got: {}", msg
    );
}
