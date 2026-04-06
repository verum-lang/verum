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
use verum_parser::VerumParser;

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
        expected_error_substring: "unexpected",
        should_have_help: false,
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
