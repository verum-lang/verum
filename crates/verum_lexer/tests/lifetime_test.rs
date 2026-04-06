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
use verum_lexer::{Lexer, TokenKind};

#[test]
fn test_raw_lifetime() {
    // Test that standalone 'a gets tokenized correctly
    let source = "'a";
    let lexer = Lexer::new(source, FileId::new(0));
    let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

    println!(
        "Source: {:?}, Tokens: {:?}",
        source,
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
    assert_eq!(tokens.len(), 2); // Lifetime + Eof

    match &tokens[0].kind {
        TokenKind::Lifetime(name) => assert_eq!(name.as_str(), "a"),
        other => panic!("Expected Lifetime, got {:?}", other),
    }
}

#[test]
fn test_lifetime_followed_by_gt() {
    // Test that <'a> gets tokenized as Lt + Lifetime('a') + Gt
    let source = "<'a>";
    let lexer = Lexer::new(source, FileId::new(0));

    println!("Lexing: {:?}", source);
    let result: Result<Vec<_>, _> = lexer
        .inspect(|r| {
            match r {
                Ok(t) => println!("Token: {:?}", t.kind),
                Err(e) => println!("Error: {:?}", e),
            }
        })
        .collect();

    match result {
        Ok(toks) => {
            println!(
                "Successfully tokenized: {:?}",
                toks.iter().map(|t| &t.kind).collect::<Vec<_>>()
            );
            assert_eq!(toks.len(), 4); // Lt + Lifetime + Gt + Eof
            match &toks[0].kind {
                TokenKind::Lt => {}
                other => panic!("Expected Lt, got {:?}", other),
            }
            match &toks[1].kind {
                TokenKind::Lifetime(name) => assert_eq!(name.as_str(), "a"),
                other => panic!("Expected Lifetime, got {:?}", other),
            }
            match &toks[2].kind {
                TokenKind::Gt => {}
                other => panic!("Expected Gt, got {:?}", other),
            }
        }
        Err(e) => panic!("Tokenization failed: {:?}", e),
    }
}

#[test]
fn test_lifetime_tokens() {
    // Test each lifetime separately to avoid char literal matching
    let test_cases = vec![("'a", "a"), ("'b", "b"), ("'static", "static"), ("'_", "_")];

    for (source, expected_name) in test_cases {
        let lexer = Lexer::new(source, FileId::new(0));
        let tokens: Vec<_> = lexer.map(|r| r.unwrap()).collect();

        println!(
            "Source: {:?}, Tokens: {:?}",
            source,
            tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
        );
        // Lexer includes Eof token
        assert_eq!(
            tokens.len(),
            2,
            "Expected 2 tokens (Lifetime + Eof) for {}",
            source
        );

        match &tokens[0].kind {
            TokenKind::Lifetime(name) => assert_eq!(
                name.as_str(),
                expected_name,
                "Wrong lifetime name for {}",
                source
            ),
            other => panic!("Expected Lifetime for {}, got {:?}", source, other),
        }
    }
}

#[test]
fn test_lifetime_in_generic_context() {
    let source = "for<'a>";
    let lexer = Lexer::new(source, FileId::new(0));

    println!("Lexing: {:?}", source);
    let tokens: Vec<_> = lexer
        .map(|r| match r {
            Ok(t) => {
                println!("Token: {:?}", t.kind);
                Ok(t)
            }
            Err(e) => {
                println!("Error: {:?}", e);
                Err(e)
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    println!(
        "Tokens for 'for<'a>': {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    // Lexer includes Eof, so we expect: For, Lt, Lifetime, Gt, Eof = 5 tokens
    assert_eq!(tokens.len(), 5, "Expected 5 tokens (for, <, 'a, >, eof)");

    match &tokens[0].kind {
        TokenKind::For => {}
        other => panic!("Expected For, got {:?}", other),
    }

    match &tokens[1].kind {
        TokenKind::Lt => {}
        other => panic!("Expected Lt (<), got {:?}", other),
    }

    match &tokens[2].kind {
        TokenKind::Lifetime(name) => assert_eq!(name.as_str(), "a"),
        other => panic!("Expected Lifetime('a'), got {:?}", other),
    }

    match &tokens[3].kind {
        TokenKind::Gt => {}
        other => panic!("Expected Gt (>), got {:?}", other),
    }
}
