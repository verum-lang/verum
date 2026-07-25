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
//! Safety tests for verum_fast_parser
//!
//! Tests memory safety, panic-free guarantees, and error recovery.

use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_parser_never_panics_on_empty_input() {
    let file_id = FileId::new(0);
    let lexer = Lexer::new("", file_id);
    let parser = VerumParser::new();
    let _result = parser.parse_module(lexer, file_id);
    // Should not panic
}

#[test]
fn test_parser_handles_deep_nesting() {
    let mut input = String::new();
    for _ in 0..100 {
        input.push_str("if true { ");
    }
    input.push('1');
    for _ in 0..100 {
        input.push_str(" }");
    }

    let file_id = FileId::new(0);
    let lexer = Lexer::new(&input, file_id);
    let parser = VerumParser::new();
    let _result = parser.parse_module(lexer, file_id);
    // Should not stack overflow
}

#[test]
fn test_parser_recovers_from_syntax_errors() {
    let input = "fn bad(( { let x = 5; }";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(input, file_id);
    let parser = VerumParser::new();
    let result = parser.parse_module(lexer, file_id);

    // Should return error, not panic
    assert!(result.is_err());
}

#[test]
fn test_parser_handles_unterminated_blocks() {
    let test_cases = vec!["fn f() {", "if true { 1", "loop { 1"];

    for input in test_cases {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(input, file_id);
        let parser = VerumParser::new();
        let _result = parser.parse_module(lexer, file_id);
    }
}

#[test]
fn test_parser_handles_mismatched_delimiters() {
    let test_cases = vec!["fn f(] {}", "let x = [};", "if (true] { 1 }"];

    for input in test_cases {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(input, file_id);
        let parser = VerumParser::new();
        let _result = parser.parse_module(lexer, file_id);
    }
}
