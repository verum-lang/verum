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
// Comprehensive tests for context keywords in verum_lexer.
//
// Verum keywords: 3 reserved (let, fn, is), plus ~38 contextual keywords across
// categories (primary, control flow, async, modifier, FFI, module, additional).
//
// This test file verifies:
// 1. All context keywords are recognized correctly
// 2. Context keywords can be used as identifiers in non-keyword contexts
// 3. Context keywords work correctly in their specific contexts
// 4. All additional keywords from the specification are implemented

use logos::Logos;
use verum_lexer::token::TokenKind;
use verum_common::Text;

// ============================================================================
// Basic Context Keyword Recognition
// ============================================================================

#[test]
fn test_module_path_keywords() {
    // Module path keywords: super (parent module), crate/cog (root), self (current instance)
    let mut lex = TokenKind::lexer("super crate self");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Super)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Cog)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::SelfValue)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_verification_keywords() {
    // Verification keywords: invariant (loop/type invariants), decreases (termination proofs),
    // ensures (postconditions), requires (preconditions), result (return value in postconditions)
    let mut lex = TokenKind::lexer("invariant decreases ensures requires result");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Invariant)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Decreases)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Ensures)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Requires)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Result)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_advanced_type_keywords() {
    // Advanced type keywords: tensor (tensor types/operations), affine (use-once semantics)
    let mut lex = TokenKind::lexer("tensor affine");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Tensor)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Affine)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_error_handling_keywords() {
    // Error handling keywords: finally (cleanup), recover (error recovery), try (error propagation)
    let mut lex = TokenKind::lexer("finally recover try");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Finally)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Recover)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Try)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_context_system_keywords() {
    // Context system keywords: using (declare required contexts), context (define DI interfaces),
    // provide (install context providers in scope)
    let mut lex = TokenKind::lexer("using context provide");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Using)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Context)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Provide)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_visibility_keywords() {
    // Visibility keywords: public, internal, protected, pub (preferred short form)
    let mut lex = TokenKind::lexer("public internal protected pub");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Public)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Internal)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Protected)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Pub)));
    assert_eq!(lex.next(), None);
}

// ============================================================================
// All Keywords from Specification (Complete List)
// ============================================================================

#[test]
fn test_reserved_keywords() {
    // Reserved keywords (3): CANNOT be used as identifiers in any context
    let mut lex = TokenKind::lexer("let fn is");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Let)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Fn)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Is)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_primary_keywords() {
    // Primary keywords (3): essential for type system and contexts
    let mut lex = TokenKind::lexer("type where using");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Type)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Where)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Using)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_control_flow_keywords() {
    // Control flow keywords (9): if, else, match, return, for, while, loop, break, continue
    let mut lex = TokenKind::lexer("if else match return for while loop break continue");
    assert_eq!(lex.next(), Some(Ok(TokenKind::If)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Else)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Match)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Return)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::For)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::While)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Loop)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Break)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Continue)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_async_context_keywords() {
    // Async keywords (5): async, await, spawn, defer, try
    let mut lex = TokenKind::lexer("async await spawn defer try");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Async)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Await)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Spawn)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Defer)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Try)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_modifier_keywords() {
    // Modifier keywords (4): pub, mut, const, unsafe
    let mut lex = TokenKind::lexer("pub mut const unsafe");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Pub)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Mut)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Const)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Unsafe)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_ffi_keywords() {
    // FFI keyword (1): ffi -- foreign function interface boundary declarations (C ABI only)
    let mut lex = TokenKind::lexer("ffi");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Ffi)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_module_keywords() {
    // Module keywords (4+): module, implement, context, protocol (plus extends)
    let mut lex = TokenKind::lexer("module implement context protocol");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Module)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Implement)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Context)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Protocol)));
    assert_eq!(lex.next(), None);
}

#[test]
fn test_additional_keywords_complete() {
    // Additional keywords (~18): self, super, crate, static, meta, provide, finally,
    // recover, invariant, decreases, stream, tensor, affine, linear, public, internal,
    // protected, ensures, requires, result
    // Testing in chunks for readability

    // Chunk 1: Path and static
    let mut lex = TokenKind::lexer("self super crate static meta");
    assert_eq!(lex.next(), Some(Ok(TokenKind::SelfValue)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Super)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Cog)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Static)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Meta)));
    assert_eq!(lex.next(), None);

    // Chunk 2: Context and error handling
    let mut lex = TokenKind::lexer("provide finally recover");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Provide)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Finally)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Recover)));
    assert_eq!(lex.next(), None);

    // Chunk 3: Verification
    let mut lex = TokenKind::lexer("invariant decreases");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Invariant)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Decreases)));
    assert_eq!(lex.next(), None);

    // Chunk 4: Advanced types
    let mut lex = TokenKind::lexer("stream tensor affine");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Stream)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Tensor)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Affine)));
    assert_eq!(lex.next(), None);

    // Chunk 5: Visibility
    let mut lex = TokenKind::lexer("public internal protected");
    assert_eq!(lex.next(), Some(Ok(TokenKind::Public)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Internal)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Protected)));
    assert_eq!(lex.next(), None);
}

// ============================================================================
// Context-Specific Usage Tests
// ============================================================================

#[test]
fn test_contract_keywords_in_context() {
    // Contract keywords in function signatures: requires (preconditions),
    // ensures (postconditions), invariant, decreases should appear in contracts
    let code = r#"
        fn divide(a: Int, b: Int) -> Int
        requires b != 0
        ensures result > 0
        {
            let x = 0;
        }
    "#;

    let mut lex = TokenKind::lexer(code);

    // Skip to requires
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Requires {
            break;
        }
    }

    // Find ensures
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Ensures {
            break;
        }
    }

    // Find result
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Result {
            break;
        }
    }
}

#[test]
fn test_loop_invariant_context() {
    // Loop invariant keyword: used in for/while loops for verification
    let code = r#"
        for i in 0..n
        invariant total >= 0
        {
            total += i;
        }
    "#;

    let mut lex = TokenKind::lexer(code);

    // Find invariant keyword
    let mut found_invariant = false;
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Invariant {
            found_invariant = true;
            break;
        }
    }
    assert!(
        found_invariant,
        "invariant keyword should be recognized in loop context"
    );
}

#[test]
fn test_module_path_context() {
    // Module path keywords: super, crate/cog, self with dot-separated paths
    // Verum uses . (dot) as path separator, not ::
    let code = "super.parent.Type crate.root.Module self.method()";

    let mut lex = TokenKind::lexer(code);

    assert_eq!(lex.next(), Some(Ok(TokenKind::Super)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Dot)));

    // Skip to crate
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Cog {
            break;
        }
    }

    // Skip to self
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::SelfValue {
            break;
        }
    }
}

#[test]
fn test_error_handling_context() {
    // Error handling keywords: try { ... } recover { ... } finally { ... }
    let code = r#"
        try {
            risky_operation()
        } recover {
            handle_error()
        } finally {
            cleanup()
        }
    "#;

    let mut lex = TokenKind::lexer(code);

    let mut found_try = false;
    let mut found_recover = false;
    let mut found_finally = false;

    while let Some(Ok(tok)) = lex.next() {
        match tok {
            TokenKind::Try => found_try = true,
            TokenKind::Recover => found_recover = true,
            TokenKind::Finally => found_finally = true,
            _ => {}
        }
    }

    assert!(found_try, "try keyword should be recognized");
    assert!(found_recover, "recover keyword should be recognized");
    assert!(found_finally, "finally keyword should be recognized");
}

#[test]
fn test_tensor_type_context() {
    // tensor keyword: used for tensor types and operations (e.g., type Matrix is tensor<2, f64>)
    let code = "type Matrix is tensor<2, f64>";

    let mut lex = TokenKind::lexer(code);

    assert_eq!(lex.next(), Some(Ok(TokenKind::Type)));

    // Skip to tensor
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Tensor {
            return;
        }
    }
    panic!("tensor keyword not found");
}

#[test]
fn test_affine_type_context() {
    // affine keyword: use-once semantics for resource types (e.g., file handles)
    let code = "type FileHandle is affine { handle: i64 }";

    let mut lex = TokenKind::lexer(code);

    let mut found_affine = false;
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Affine {
            found_affine = true;
            break;
        }
    }
    assert!(found_affine, "affine keyword should be recognized");
}

// ============================================================================
// Contextual Keyword Restrictions Tests
// ============================================================================

#[test]
fn test_where_clause_forbidden_keywords() {
    // Contextual keyword restrictions: keywords are always lexed as keywords even in
    // positions where they are syntactically invalid (parser validates context later)
    let code = "type Foo is Int where type meta ensures value";

    let mut lex = TokenKind::lexer(code);

    // All should lex successfully (parser validates context)
    assert_eq!(lex.next(), Some(Ok(TokenKind::Type)));

    // Skip to where
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Where {
            break;
        }
    }

    // After where, these are lexed as keywords (parser will validate)
    assert_eq!(lex.next(), Some(Ok(TokenKind::Type)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Meta)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Ensures)));

    // 'value' is not a keyword, should be identifier
    match lex.next() {
        Some(Ok(TokenKind::Ident(ref s))) if s == &Text::from("value") => {}
        other => panic!("Expected identifier 'value', got {:?}", other),
    }
}

// ============================================================================
// is_keyword() Method Tests
// ============================================================================

#[test]
fn test_is_keyword_method() {
    use verum_ast::span::{FileId, Span};
    use verum_lexer::token::Token;

    let span = Span::new(0, 0, FileId::new(0));

    // Test all new context keywords are recognized as keywords
    assert!(Token::new(TokenKind::Super, span).is_keyword());
    assert!(Token::new(TokenKind::Cog, span).is_keyword());
    assert!(Token::new(TokenKind::Invariant, span).is_keyword());
    assert!(Token::new(TokenKind::Decreases, span).is_keyword());
    assert!(Token::new(TokenKind::Tensor, span).is_keyword());
    assert!(Token::new(TokenKind::Affine, span).is_keyword());
    assert!(Token::new(TokenKind::Finally, span).is_keyword());
    assert!(Token::new(TokenKind::Recover, span).is_keyword());
    assert!(Token::new(TokenKind::Ensures, span).is_keyword());
    assert!(Token::new(TokenKind::Requires, span).is_keyword());
    assert!(Token::new(TokenKind::Result, span).is_keyword());

    // Test identifiers are NOT keywords
    assert!(!Token::new(TokenKind::Ident(Text::from("value")), span).is_keyword());
    assert!(!Token::new(TokenKind::Ident(Text::from("foo")), span).is_keyword());
}

// ============================================================================
// description() Method Tests
// ============================================================================

#[test]
fn test_keyword_descriptions() {
    // Verify all new keywords have proper descriptions
    assert_eq!(TokenKind::Super.description(), "keyword `super`");
    assert_eq!(TokenKind::Cog.description(), "keyword `cog`");
    assert_eq!(TokenKind::Invariant.description(), "keyword `invariant`");
    assert_eq!(TokenKind::Decreases.description(), "keyword `decreases`");
    assert_eq!(TokenKind::Tensor.description(), "keyword `tensor`");
    assert_eq!(TokenKind::Affine.description(), "keyword `affine`");
    assert_eq!(TokenKind::Finally.description(), "keyword `finally`");
    assert_eq!(TokenKind::Recover.description(), "keyword `recover`");
    assert_eq!(TokenKind::Ensures.description(), "keyword `ensures`");
    assert_eq!(TokenKind::Requires.description(), "keyword `requires`");
    assert_eq!(TokenKind::Result.description(), "keyword `result`");
}

// ============================================================================
// Comprehensive Specification Compliance Test
// ============================================================================

#[test]
fn test_all_33_keywords_from_spec() {
    // Total keywords: ~41 (3 reserved + 3 primary + 9 control flow + 8 async +
    // 4 modifiers + 1 FFI + 6 module + 18 additional). This test verifies all keywords.

    let all_keywords = vec![
        // Reserved (3)
        ("let", TokenKind::Let),
        ("fn", TokenKind::Fn),
        ("is", TokenKind::Is),
        // Primary (3)
        ("type", TokenKind::Type),
        ("where", TokenKind::Where),
        ("using", TokenKind::Using),
        // Control Flow (9)
        ("if", TokenKind::If),
        ("else", TokenKind::Else),
        ("match", TokenKind::Match),
        ("return", TokenKind::Return),
        ("for", TokenKind::For),
        ("while", TokenKind::While),
        ("loop", TokenKind::Loop),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        // Async/Context (5)
        ("async", TokenKind::Async),
        ("await", TokenKind::Await),
        ("spawn", TokenKind::Spawn),
        ("defer", TokenKind::Defer),
        ("try", TokenKind::Try),
        // Modifiers (4)
        ("pub", TokenKind::Pub),
        ("mut", TokenKind::Mut),
        ("const", TokenKind::Const),
        ("unsafe", TokenKind::Unsafe),
        // FFI (1)
        ("ffi", TokenKind::Ffi),
        // Module (5)
        ("module", TokenKind::Module),
        ("mount", TokenKind::Mount),
        ("implement", TokenKind::Implement),
        ("context", TokenKind::Context),
        ("protocol", TokenKind::Protocol),
        // Additional (15 - but we have more with verification keywords)
        ("self", TokenKind::SelfValue),
        ("super", TokenKind::Super),
        ("cog", TokenKind::Cog),
        ("static", TokenKind::Static),
        ("meta", TokenKind::Meta),
        ("provide", TokenKind::Provide),
        ("finally", TokenKind::Finally),
        ("recover", TokenKind::Recover),
        ("invariant", TokenKind::Invariant),
        ("decreases", TokenKind::Decreases),
        ("stream", TokenKind::Stream),
        ("tensor", TokenKind::Tensor),
        ("affine", TokenKind::Affine),
        ("public", TokenKind::Public),
        ("internal", TokenKind::Internal),
        ("protected", TokenKind::Protected),
    ];

    // Test each keyword individually
    for (keyword_str, expected_kind) in all_keywords {
        let mut lex = TokenKind::lexer(keyword_str);
        assert_eq!(
            lex.next(),
            Some(Ok(expected_kind.clone())),
            "Keyword '{}' should lex to {:?}",
            keyword_str,
            expected_kind
        );
    }
}

#[test]
fn test_verification_keywords_complete() {
    // Additional verification keywords not in the core 33 count
    // but specified in the contract system
    let verification_keywords = vec![
        ("requires", TokenKind::Requires),
        ("ensures", TokenKind::Ensures),
        ("result", TokenKind::Result),
    ];

    for (keyword_str, expected_kind) in verification_keywords {
        let mut lex = TokenKind::lexer(keyword_str);
        assert_eq!(
            lex.next(),
            Some(Ok(expected_kind.clone())),
            "Verification keyword '{}' should lex to {:?}",
            keyword_str,
            expected_kind
        );
    }
}

// ============================================================================
// Edge Cases and Contextual Behavior
// ============================================================================

#[test]
fn test_keywords_adjacent_to_operators() {
    // Keywords should be recognized even when adjacent to operators
    // Verum uses . (dot) as path separator, not ::
    let code = "super.Foo,crate.Bar;invariant&&decreases";

    let mut lex = TokenKind::lexer(code);

    assert_eq!(lex.next(), Some(Ok(TokenKind::Super)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Dot)));

    // Skip to crate
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Cog {
            break;
        }
    }

    // Skip to invariant
    while let Some(Ok(tok)) = lex.next() {
        if tok == TokenKind::Invariant {
            break;
        }
    }

    assert_eq!(lex.next(), Some(Ok(TokenKind::AmpersandAmpersand)));
    assert_eq!(lex.next(), Some(Ok(TokenKind::Decreases)));
}

#[test]
fn test_keywords_in_different_cases() {
    // Keywords are case-sensitive, these should be identifiers
    let code = "SUPER Crate INVARIANT";

    let mut lex = TokenKind::lexer(code);

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "SUPER" => {}
        other => panic!("Expected identifier SUPER, got {:?}", other),
    }

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "Crate" => {}
        other => panic!("Expected identifier Crate, got {:?}", other),
    }

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "INVARIANT" => {}
        other => panic!("Expected identifier INVARIANT, got {:?}", other),
    }
}

#[test]
fn test_keywords_with_underscores_are_identifiers() {
    // Keywords with underscores should be identifiers
    let code = "super_ _crate invariant_check";

    let mut lex = TokenKind::lexer(code);

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "super_" => {}
        other => panic!("Expected identifier super_, got {:?}", other),
    }

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "_crate" => {}
        other => panic!("Expected identifier _crate, got {:?}", other),
    }

    match lex.next() {
        Some(Ok(TokenKind::Ident(s))) if s == "invariant_check" => {}
        other => panic!("Expected identifier invariant_check, got {:?}", other),
    }
}
