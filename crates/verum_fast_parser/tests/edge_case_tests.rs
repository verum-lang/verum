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
//! Edge case tests for parser robustness.
//!
//! Tests that the parser never panics on any input and handles all edge cases
//! gracefully by returning errors instead of crashing.

use verum_ast::span::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn file_id() -> FileId {
    FileId::new(0)
}

// ============================================================================
// Empty and minimal inputs
// ============================================================================

#[test]
fn empty_file_parses_to_empty_module() {
    let parser = VerumParser::new();
    let result = parser.parse_module_str("", file_id());
    assert!(result.is_ok(), "Empty file should parse to an empty module");
    let module = result.unwrap();
    assert!(module.items.is_empty(), "Empty file should have no items");
}

#[test]
fn file_with_only_whitespace() {
    let parser = VerumParser::new();
    let result = parser.parse_module_str("   \n\n\t\t  \n  ", file_id());
    assert!(result.is_ok(), "Whitespace-only file should parse successfully");
}

#[test]
fn file_with_only_line_comments() {
    let parser = VerumParser::new();
    let result = parser.parse_module_str(
        "// This is a comment\n// Another comment\n// Final comment\n",
        file_id(),
    );
    assert!(result.is_ok(), "Comment-only file should parse successfully");
}

#[test]
fn file_with_only_block_comments() {
    let parser = VerumParser::new();
    let result = parser.parse_module_str(
        "/* block comment */\n/* another\n   multiline\n   comment */",
        file_id(),
    );
    assert!(result.is_ok(), "Block-comment-only file should parse successfully");
}

#[test]
fn file_with_nested_block_comments() {
    let parser = VerumParser::new();
    let result = parser.parse_module_str(
        "/* outer /* inner */ still outer */",
        file_id(),
    );
    // Whether nested comments are supported or not, this should not panic
    let _ = result;
}

// ============================================================================
// Very long identifiers
// ============================================================================

#[test]
fn very_long_identifier_1000_chars() {
    let long_name: String = "a".repeat(1000);
    let input = format!("let {} = 42;", long_name);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Should either parse or return an error, but never panic
    let _ = result;
}

#[test]
fn very_long_identifier_5000_chars() {
    let long_name: String = "x".repeat(5000);
    let input = format!("fn {}() {{}}", long_name);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

// ============================================================================
// Deeply nested expressions - recursion depth limits
// ============================================================================

#[test]
fn deeply_nested_parentheses_100_levels() {
    let mut input = String::new();
    for _ in 0..100 {
        input.push('(');
    }
    input.push('1');
    for _ in 0..100 {
        input.push(')');
    }
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(&input, file_id());
    // Should either succeed or return a recursion depth error, never panic
    let _ = result;
}

#[test]
fn deeply_nested_parentheses_300_levels() {
    // This exceeds MAX_RECURSION_DEPTH (256) and should return an error.
    // Run in a thread with a larger stack to avoid stack overflow before the
    // parser's own recursion depth check fires.
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // 32 MB stack
        .spawn(|| {
            let mut input = String::new();
            for _ in 0..300 {
                input.push('(');
            }
            input.push('1');
            for _ in 0..300 {
                input.push(')');
            }
            let parser = VerumParser::new();
            parser.parse_expr_str(&input, FileId::new(0))
        })
        .expect("thread spawn failed")
        .join()
        .expect("thread panicked");
    assert!(
        result.is_err(),
        "300 levels of nesting should exceed recursion depth limit"
    );
}

#[test]
fn deeply_nested_if_expressions_200_levels() {
    // Run with larger stack to avoid OS stack overflow before parser's depth check fires.
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut input = String::new();
            for _ in 0..200 {
                input.push_str("if true { ");
            }
            input.push('1');
            for _ in 0..200 {
                input.push_str(" }");
            }
            let parser = VerumParser::new();
            let lexer = Lexer::new(&input, FileId::new(0));
            parser.parse_module(lexer, FileId::new(0))
        })
        .expect("thread spawn failed")
        .join()
        .expect("thread panicked");
    // Should not stack overflow - should return error if too deep
    let _ = result;
}

#[test]
fn deeply_nested_binary_ops_500() {
    // a + a + a + ... (500 times) - this tests left-recursive pratt parsing
    let input = (0..500)
        .map(|_| "a")
        .collect::<Vec<_>>()
        .join(" + ");
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(&input, file_id());
    // Pratt parsing is iterative for left-assoc ops, so this should work
    let _ = result;
}

#[test]
fn deeply_nested_function_calls_100() {
    // f(f(f(f(... 100 deep)))) - run with larger stack for safety
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let mut input = String::new();
            for _ in 0..100 {
                input.push_str("f(");
            }
            input.push('1');
            for _ in 0..100 {
                input.push(')');
            }
            let parser = VerumParser::new();
            parser.parse_expr_str(&input, FileId::new(0))
        })
        .expect("thread spawn failed")
        .join()
        .expect("thread panicked");
    let _ = result;
}

// ============================================================================
// Unicode identifiers
// ============================================================================

#[test]
fn unicode_identifier_greek() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("\u{03B1} + \u{03B2}", file_id());
    // alpha + beta - should work if unicode identifiers are supported
    let _ = result;
}

#[test]
fn unicode_identifier_cjk() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("\u{4E16}\u{754C}", file_id());
    // Chinese characters - should either parse as identifier or return error
    let _ = result;
}

#[test]
fn unicode_identifier_emoji_should_fail() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("\u{1F600}", file_id());
    // Emoji should not be valid identifier - but must not panic
    let _ = result;
}

#[test]
fn unicode_in_string_literal() {
    let parser = VerumParser::new();
    let input = r#"let s = "Hello \u{1F600} world";"#;
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

// ============================================================================
// Reserved keywords as identifiers (should fail gracefully)
// ============================================================================

#[test]
fn reserved_keyword_let_as_function_name() {
    let parser = VerumParser::new();
    let input = "fn let() {}";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(
        result.is_err(),
        "'let' is reserved and cannot be used as a function name"
    );
}

#[test]
fn reserved_keyword_fn_as_variable() {
    let parser = VerumParser::new();
    let input = "let fn = 42;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(
        result.is_err(),
        "'fn' is reserved and cannot be used as a variable name"
    );
}

#[test]
fn reserved_keyword_is_as_variable() {
    let parser = VerumParser::new();
    let input = "let is = 42;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(
        result.is_err(),
        "'is' is reserved and cannot be used as a variable name"
    );
}

#[test]
fn reserved_keyword_let_as_type_name() {
    let parser = VerumParser::new();
    let input = "type let is { x: Int };";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(
        result.is_err(),
        "'let' is reserved and cannot be used as a type name"
    );
}

// ============================================================================
// Malformed inputs that must not crash
// ============================================================================

#[test]
fn null_byte_in_input() {
    let parser = VerumParser::new();
    let input = "let x\0 = 5;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Must not panic - error is fine
    let _ = result;
}

#[test]
fn only_operators() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str("+ - * /", file_id());
    assert!(result.is_err(), "Bare operators should not parse as expression");
}

#[test]
fn only_punctuation() {
    let parser = VerumParser::new();
    let input = "{ } ( ) [ ] ; , . : :: -> =>";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Should not panic
    let _ = result;
}

#[test]
fn extremely_long_number_literal() {
    let parser = VerumParser::new();
    // A number with 1000 digits
    let big_number = "9".repeat(1000);
    let input = format!("let x = {};", big_number);
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Should not panic even if the number overflows
    let _ = result;
}

#[test]
fn repeated_semicolons() {
    let parser = VerumParser::new();
    let input = ";;;;;;;;;;;;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

#[test]
fn incomplete_let_binding() {
    let parser = VerumParser::new();
    let input = "let";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Incomplete let binding should fail");
}

#[test]
fn incomplete_fn_declaration() {
    let parser = VerumParser::new();
    let input = "fn";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Incomplete fn declaration should fail");
}

#[test]
fn incomplete_type_declaration() {
    let parser = VerumParser::new();
    let input = "type";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Incomplete type declaration should fail");
}

#[test]
fn unmatched_open_paren() {
    let parser = VerumParser::new();
    let input = "fn f(";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Unmatched open paren should fail");
}

#[test]
fn unmatched_close_paren() {
    let parser = VerumParser::new();
    let result = parser.parse_expr_str(")", file_id());
    assert!(result.is_err(), "Unmatched close paren should fail");
}

#[test]
fn unmatched_open_brace() {
    let parser = VerumParser::new();
    let input = "fn f() {";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Unmatched open brace should fail");
}

#[test]
fn unmatched_close_brace() {
    let parser = VerumParser::new();
    let input = "}";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Should not panic
    let _ = result;
}

#[test]
fn many_consecutive_keywords() {
    let parser = VerumParser::new();
    let input = "let let let fn fn fn is is is";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_err(), "Consecutive reserved keywords should fail");
}

// ============================================================================
// Numeric edge cases
// ============================================================================

#[test]
fn hex_literal_max_u64() {
    let parser = VerumParser::new();
    let input = "let x = 0xFFFFFFFFFFFFFFFF;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    // Should parse without overflow panic
    let _ = result;
}

#[test]
fn binary_literal_64_bits() {
    let parser = VerumParser::new();
    let input = format!("let x = 0b{};", "1".repeat(64));
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

#[test]
fn float_literal_very_small() {
    let parser = VerumParser::new();
    let input = "let x = 0.000000000000000000000000000000000001;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

#[test]
fn float_literal_very_large() {
    let parser = VerumParser::new();
    let input = "let x = 999999999999999999999999999999999999.0;";
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

// ============================================================================
// String edge cases
// ============================================================================

#[test]
fn empty_string_literal() {
    let parser = VerumParser::new();
    let input = r#"fn main() { let x = ""; }"#;
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_ok(), "Empty string literal should parse");
}

#[test]
fn string_with_many_escapes() {
    let parser = VerumParser::new();
    let input = r#"let x = "\n\t\r\\\"\0";"#;
    let lexer = Lexer::new(input, file_id());
    let result = parser.parse_module(lexer, file_id());
    let _ = result;
}

// ============================================================================
// Stress: many items in a single file
// ============================================================================

#[test]
fn many_function_declarations() {
    let parser = VerumParser::new();
    let mut input = String::new();
    for i in 0..200 {
        input.push_str(&format!("fn func_{}() {{}}\n", i));
    }
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_ok(), "200 function declarations should parse");
}

#[test]
fn many_let_bindings_in_block() {
    let parser = VerumParser::new();
    let mut input = String::from("fn main() {\n");
    for i in 0..200 {
        input.push_str(&format!("    let x_{} = {};\n", i, i));
    }
    input.push_str("}\n");
    let lexer = Lexer::new(&input, file_id());
    let result = parser.parse_module(lexer, file_id());
    assert!(result.is_ok(), "200 let bindings should parse");
}
