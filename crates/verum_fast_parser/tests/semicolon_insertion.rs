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
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

#[test]
#[ignore = "Semicolon insertion not yet implemented - requires language design decision"]
fn test_optional_semicolons_in_let_statements() {
    let source = r#"
fn test() {
    let mut a = 0
    let mut b = 1

    while i <= n {
        let temp = a + b
        a = b
        b = temp
    }

    for i in 0..10 {
        print(i)
    }
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();

    // Debug: print tokens around position 102
    println!("Tokens around position 102:");
    for (i, tok) in tokens.iter().enumerate() {
        if tok.span.start >= 90 && tok.span.start <= 110 {
            println!("  {}: {:?}", i, tok);
        }
    }

    let mut parser = RecursiveParser::new(&tokens[..], file_id);

    let result = parser.parse_item();

    // Should parse successfully without errors
    if !parser.errors.is_empty() {
        println!("Parser errors:");
        for err in &parser.errors {
            println!("  {:?}", err);
        }
    }

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    assert!(
        parser.errors.is_empty(),
        "Parser had errors: {:?}",
        parser.errors
    );
}

#[test]
#[ignore = "Semicolon insertion not yet implemented - requires language design decision"]
fn test_mixed_semicolons() {
    // Test that mixing explicit and implicit semicolons works
    let source = r#"
fn test() {
    let a = 1;
    let b = 2
    let c = 3;
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens[..], file_id);

    let result = parser.parse_item();

    // Should parse successfully with mixed semicolons
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    assert!(
        parser.errors.is_empty(),
        "Parser had errors: {:?}",
        parser.errors
    );
}

#[test]
fn test_explicit_semicolons_still_work() {
    let source = r#"
fn test() {
    let mut a = 0;
    let mut b = 1;

    while i <= n {
        let temp = a + b;
        a = b;
        b = temp;
    }
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens[..], file_id);

    let result = parser.parse_item();

    // Should parse successfully with explicit semicolons
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    assert!(
        parser.errors.is_empty(),
        "Parser had errors: {:?}",
        parser.errors
    );
}

#[test]
#[ignore = "Semicolon insertion not yet implemented - requires language design decision"]
fn test_if_else_without_semicolons() {
    // This is a critical test - semicolons should be optional before 'else'
    let source = r#"
fn test() {
    let x = 5
    if x > 3 {
        let y = x + 1
    } else {
        let y = x - 1
    }
}
"#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();

    println!("Tokens:");
    for (i, tok) in tokens.iter().enumerate() {
        println!("  {}: {:?}", i, tok);
    }

    let mut parser = RecursiveParser::new(&tokens[..], file_id);
    let result = parser.parse_item();

    if !parser.errors.is_empty() {
        println!("Parser errors:");
        for err in &parser.errors {
            println!("  {:?}", err);
        }
    }

    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    assert!(
        parser.errors.is_empty(),
        "Parser had errors: {:?}",
        parser.errors
    );
}
