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
//! Property-based tests for verum_fast_parser

use proptest::prelude::*;
use verum_ast::span::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

proptest! {
    #[test]
    fn parser_never_panics(s in "\\PC*") {
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&s, file_id);
        let parser = VerumParser::new();
        let _result = parser.parse_module(lexer, file_id);
    }

    #[test]
    fn valid_identifiers_parse_in_let(name in "[a-z][a-z0-9_]*") {
        // Filter out reserved keywords that shouldn't be used as identifiers
        // Reserved keywords: let, fn, is
        // Contextual keywords: type, match, import, where, if, else, while, for, loop, break, continue, return, yield
        // and many others
        let keywords = [
            "let", "fn", "is", "type", "match", "import", "where", "if", "else",
            "while", "for", "loop", "break", "continue", "return", "yield", "mut",
            "const", "static", "meta", "implement", "protocol", "module", "async",
            "await", "spawn", "unsafe", "ref", "move", "as", "in", "true", "false",
            "none", "some", "ok", "err", "self", "public", "pub", "internal", "protected",
            "private", "stream", "defer", "using", "context", "provide", "ffi", "try",
            "checked", "super", "cog", "invariant", "decreases", "tensor", "affine",
            "linear", "finally", "recover", "ensures", "requires", "result",
        ];

        if keywords.contains(&name.as_str()) {
            return Ok(());
        }

        // Wrap in a function since let is a statement, not an expression
        let input = format!("fn test() {{ let {} = 42; }}", name);
        let file_id = FileId::new(0);
        let lexer = Lexer::new(&input, file_id);
        let parser = VerumParser::new();
        let result = parser.parse_module(lexer, file_id);
        assert!(result.is_ok());
    }

    #[test]
    fn numeric_literals_always_parse(n in any::<i64>()) {
        let input = n.to_string();
        let file_id = FileId::new(0);
        let parser = VerumParser::new();
        let result = parser.parse_expr_str(&input, file_id);
        assert!(result.is_ok());
    }
}
