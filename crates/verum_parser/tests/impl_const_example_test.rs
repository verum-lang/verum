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
//! Integration test to verify that const in impl blocks works with a real example.

use std::fs;
use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

#[test]
fn test_parse_impl_with_const_example() {
    let source = fs::read_to_string("tests/examples/impl_with_const.vr")
        .expect("Failed to read example file");

    let file_id = FileId::new(0);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    let result = parser.parse_module(lexer, file_id);

    match result {
        Ok(module) => {
            // Should have 3 items: type Duration, implement Duration, and fn main
            assert_eq!(module.items.len(), 3, "Expected 3 top-level items");
        }
        Err(errors) => {
            panic!("Parse errors: {:?}", errors);
        }
    }
}
