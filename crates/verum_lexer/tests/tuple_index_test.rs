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
use verum_lexer::{Lexer, TokenKind};

#[test]
fn test_tuple_index_lexing() {
    // Note: `field` is a keyword in Verum (proof tactic), so use `some_field` instead
    let source = "pair.0 pair.1 pair.some_field";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();

    println!("Tokens:");
    for token in &tokens {
        println!("  {:?}", token.kind);
    }

    // Should be: Ident("pair"), Dot, Integer(0), Ident("pair"), Dot, Integer(1), Ident("pair"), Dot, Ident("some_field"), Eof
    assert_eq!(tokens.len(), 10); // Including EOF

    // First: pair.0 (tuple index access)
    assert!(matches!(tokens[0].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[1].kind, TokenKind::Dot));
    assert!(matches!(tokens[2].kind, TokenKind::Integer(_)));

    // Second: pair.1 (tuple index access)
    assert!(matches!(tokens[3].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[4].kind, TokenKind::Dot));
    assert!(matches!(tokens[5].kind, TokenKind::Integer(_)));

    // Third: pair.some_field (record field access)
    assert!(matches!(tokens[6].kind, TokenKind::Ident(_)));
    assert!(matches!(tokens[7].kind, TokenKind::Dot));
    assert!(matches!(tokens[8].kind, TokenKind::Ident(_)));

    assert!(matches!(tokens[9].kind, TokenKind::Eof));
}
