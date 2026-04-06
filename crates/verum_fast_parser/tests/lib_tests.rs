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
use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

#[test]
fn test_parser_creation() {
    let parser = VerumParser::new();
    // Parser is a zero-sized type (ZST) - just verify it can be created
    let _ = parser;
}

#[test]
fn test_empty_module() {
    let parser = VerumParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new("", file_id);

    let result = parser.parse_module(lexer, file_id);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().items.len(), 0);
}
