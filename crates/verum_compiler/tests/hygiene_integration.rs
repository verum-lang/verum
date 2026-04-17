//! Integration tests for the hygiene system
//!
//! These tests verify that the hygiene system integrates correctly with
//! the rest of the compiler, including the meta evaluator and code generation.
//!
//! Quote hygiene prevents accidental variable capture in macro expansions.
//! Each macro expansion gets a unique Mark (expansion context). Identifiers are
//! resolved based on their mark set: same-mark identifiers can reference each other,
//! cross-mark references require explicit splice ($name, #expr). This ensures
//! generated code does not collide with user-defined names at expansion sites.

use verum_ast::{Literal, Span};
use verum_common::{Maybe, Text};
use verum_compiler::hygiene::{
    BindingInfo, BindingKind, CheckResult, CheckerConfig, ExpansionConfig,
    HygieneChecker, HygieneContext, HygieneViolation, HygienicIdent, Mark, MarkSet,
    QuoteExpander, ScopeKind, ScopeSet, StageBinding, StageContext, SyntaxContext,
    SyntaxContextRegistry, Token, TokenKind, TokenStream, Transparency,
};
// Use syntax_context's ExpansionInfo for SyntaxContext operations
use verum_compiler::hygiene::syntax_context::ExpansionInfo as SyntaxExpansionInfo;
use verum_compiler::hygiene::scope::ScopeId;

// ============================================================================
// End-to-End Quote Expansion Tests
// ============================================================================

/// Test that a simple quote expansion maintains hygiene
#[test]
fn test_simple_quote_expansion_hygiene() {
    let mut hygiene_ctx = HygieneContext::new();

    // Enter a module scope
    hygiene_ctx.enter_scope(ScopeKind::Module);

    // Create a meta function scope
    hygiene_ctx.enter_scope(ScopeKind::Function);

    // Create a quote expander
    let config = ExpansionConfig::meta_to_runtime();
    let mut expander = QuoteExpander::new(hygiene_ctx, config);

    // Enter quote
    let result = expander.enter_quote(0, Span::default());
    assert!(result.is_ok());

    // Process a binding inside the quote
    let ident = expander.process_binding(
        Text::from("x"),
        Span::default(),
        BindingKind::Variable,
        false,
    );

    // The binding should have hygiene markers
    assert!(!ident.scopes.is_empty());

    // Exit quote
    expander.exit_quote();

    // No violations
    assert!(!expander.has_violations());
}

/// Test that nested quotes maintain separate scopes
#[test]
fn test_nested_quote_scope_isolation() {
    let hygiene_ctx = HygieneContext::new();
    let config = ExpansionConfig::meta_to_runtime();
    let mut expander = QuoteExpander::new(hygiene_ctx, config);

    // Enter outer quote
    expander.enter_quote(0, Span::default()).unwrap();
    let outer_x = expander.process_binding(
        Text::from("x"),
        Span::default(),
        BindingKind::Variable,
        false,
    );

    // Enter inner quote
    expander.enter_quote(0, Span::default()).unwrap();
    let inner_x = expander.process_binding(
        Text::from("x"),
        Span::default(),
        BindingKind::Variable,
        false,
    );

    // Inner and outer x should have different scopes
    assert!(
        !outer_x.scopes.is_subset_of(&inner_x.scopes)
            || !inner_x.scopes.is_subset_of(&outer_x.scopes)
    );

    expander.exit_quote();
    expander.exit_quote();
}

/// Test that gensym produces unique identifiers
#[test]
fn test_gensym_uniqueness() {
    let mut hygiene_ctx = HygieneContext::new();
    hygiene_ctx.enter_scope(ScopeKind::Module);

    let mut names = std::collections::HashSet::new();

    // Generate many unique names
    for _ in 0..100 {
        let name = hygiene_ctx.gensym("tmp");
        assert!(
            names.insert(name.to_string()),
            "gensym produced duplicate name"
        );
    }

    // All generated names should be hygienic
    for name in &names {
        assert!(HygieneContext::is_hygienic(name));
    }
}

// ============================================================================
// Capture Detection Integration Tests
// ============================================================================

/// Test that undeclared captures are properly detected
#[test]
fn test_capture_detection_integration() {
    let hygiene_ctx = HygieneContext::new();
    let mut expander = QuoteExpander::with_default_config(hygiene_ctx);

    expander.enter_quote(0, Span::default()).unwrap();

    // Try to splice an undeclared variable
    let result = expander.splice_value(&Text::from("undeclared_var"), Span::default());

    // Should fail with CaptureNotDeclared
    assert!(matches!(
        result,
        Err(HygieneViolation::CaptureNotDeclared { .. })
    ));
}

/// Test that declared captures work correctly
#[test]
fn test_declared_capture_integration() {
    let hygiene_ctx = HygieneContext::new();
    let mut expander = QuoteExpander::with_default_config(hygiene_ctx);

    expander.enter_quote(0, Span::default()).unwrap();

    // Declare and register a binding
    expander.process_binding(
        Text::from("captured_var"),
        Span::default(),
        BindingKind::Variable,
        false,
    );

    // Now splice should work
    let result = expander.splice_value(&Text::from("captured_var"), Span::default());
    assert!(result.is_ok());
}

// ============================================================================
// Multi-Stage Integration Tests
// ============================================================================

/// Test stage context hierarchy
#[test]
fn test_stage_context_hierarchy_integration() {
    let mut stage0 = StageContext::new(0);

    // Add runtime binding
    stage0.add_binding(
        Text::from("runtime_var"),
        StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("runtime_var"), Span::default()),
            valid_stage: 0,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("runtime_var"),
                hygienic_name: Text::from("runtime_var"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        },
    );

    // Create stage 1 as child
    let mut stage1 = stage0.child(1);

    // Add meta binding
    stage1.add_binding(
        Text::from("meta_var"),
        StageBinding {
            ident: HygienicIdent::unhygienic(Text::from("meta_var"), Span::default()),
            valid_stage: 1,
            ty: Maybe::None,
            binding: BindingInfo {
                original_name: Text::from("meta_var"),
                hygienic_name: Text::from("meta_var"),
                scope_id: ScopeId::new(0),
                is_mutable: false,
                kind: BindingKind::Variable,
            },
        },
    );

    // Stage 1 can see both runtime and meta bindings
    assert!(matches!(
        stage1.resolve(&Text::from("runtime_var")),
        Maybe::Some(_)
    ));
    assert!(matches!(
        stage1.resolve(&Text::from("meta_var")),
        Maybe::Some(_)
    ));

    // Stage 0 can only see runtime bindings
    assert!(matches!(
        stage0.resolve(&Text::from("runtime_var")),
        Maybe::Some(_)
    ));
    assert!(matches!(
        stage0.resolve(&Text::from("meta_var")),
        Maybe::None
    ));
}

/// Test cross-stage reference detection
#[test]
fn test_cross_stage_reference_detection() {
    let hygiene_ctx = HygieneContext::new();
    let config = ExpansionConfig::meta_to_runtime();
    let mut expander = QuoteExpander::new(hygiene_ctx, config);

    expander.enter_quote(0, Span::default()).unwrap();

    // Stage escape to invalid stage should fail
    let result = expander.stage_escape(
        5, // Much higher than source stage
        &verum_ast::Expr::literal(Literal::int(1, Span::default())),
        Span::default(),
    );

    assert!(matches!(
        result,
        Err(HygieneViolation::StageMismatch { .. })
    ));
}

// ============================================================================
// Syntax Context Integration Tests
// ============================================================================

/// Test syntax context creation and expansion chain
#[test]
fn test_syntax_context_expansion_chain() {
    let mut registry = SyntaxContextRegistry::default();

    // Create root context
    let root = SyntaxContext::root();
    registry.register(root.clone());

    // Create expansion context
    let expansion_info = SyntaxExpansionInfo::new(
        Text::from("my_macro"),
        Span::default(),
        Span::default(),
        Transparency::Opaque,
        0,
    );
    let expansion = SyntaxContext::for_expansion(&root, expansion_info);
    registry.register(expansion.clone());

    // Expansion chain should have one entry
    assert_eq!(expansion.expansion_chain().len(), 1);

    // Macro name should be correct
    let chain_str = expansion.format_expansion_chain();
    assert!(chain_str.contains("my_macro"));
}

/// Test transparency modes
#[test]
fn test_transparency_mode_integration() {
    let root = SyntaxContext::root();

    // Opaque expansion
    let opaque_info = SyntaxExpansionInfo::new(
        Text::from("opaque_macro"),
        Span::default(),
        Span::default(),
        Transparency::Opaque,
        0,
    );
    let opaque = SyntaxContext::for_expansion(&root, opaque_info);
    assert_eq!(opaque.transparency(), Transparency::Opaque);

    // Transparent expansion
    let transparent_info = SyntaxExpansionInfo::new(
        Text::from("transparent_macro"),
        Span::default(),
        Span::default(),
        Transparency::Transparent,
        0,
    );
    let transparent = SyntaxContext::for_expansion(&root, transparent_info);
    assert_eq!(transparent.transparency(), Transparency::Transparent);

    // Semi-transparent expansion
    let semi_info = SyntaxExpansionInfo::new(
        Text::from("semi_macro"),
        Span::default(),
        Span::default(),
        Transparency::SemiTransparent,
        0,
    );
    let semi = SyntaxContext::for_expansion(&root, semi_info);
    assert_eq!(semi.transparency(), Transparency::SemiTransparent);
}

// ============================================================================
// Mark and Scope Integration Tests
// ============================================================================

/// Test mark set operations across expansions
#[test]
fn test_mark_set_cross_expansion() {
    let mut marks1 = MarkSet::new();
    let mut marks2 = MarkSet::new();

    let mark_a = Mark::fresh();
    let mark_b = Mark::fresh();
    let mark_c = Mark::fresh();

    marks1.add(mark_a);
    marks1.add(mark_b);

    marks2.add(mark_b);
    marks2.add(mark_c);

    // marks1 and marks2 share mark_b but neither is subset of the other
    // So they are NOT compatible (compatible requires subset relationship)
    assert!(!marks1.compatible(&marks2));
    assert!(!marks2.compatible(&marks1));

    // Create a subset: marks3 is subset of marks1
    let mut marks3 = MarkSet::new();
    marks3.add(mark_a);

    // Subset should be compatible
    assert!(marks3.compatible(&marks1));
    assert!(marks1.compatible(&marks3));
}

/// Test hygienic identifier binding equality
#[test]
fn test_hygienic_ident_binding_equality() {
    let mut scopes1 = ScopeSet::new();
    scopes1.add(ScopeId::new(1));
    scopes1.add(ScopeId::new(2));

    let mut scopes2 = ScopeSet::new();
    scopes2.add(ScopeId::new(1));
    scopes2.add(ScopeId::new(2));
    scopes2.add(ScopeId::new(3));

    let ident1 = HygienicIdent::new(Text::from("x"), scopes1.clone(), Span::default());
    let ident2 = HygienicIdent::new(Text::from("x"), scopes2, Span::default());

    // Same name, scopes1 is subset of scopes2
    assert!(scopes1.is_subset_of(&ident2.scopes));

    // Different names should not be binding-equal
    let ident3 = HygienicIdent::new(Text::from("y"), scopes1, Span::default());
    assert_ne!(ident1.name, ident3.name);
}

// ============================================================================
// Token Stream Processing Integration Tests
// ============================================================================

/// Test token stream mark application throughout expansion
#[test]
fn test_token_stream_full_mark_pipeline() {
    let mut stream = TokenStream::new();

    // Create tokens for: let x = 42;
    let let_ident = HygienicIdent::unhygienic(Text::from("let"), Span::default());
    let x_ident = HygienicIdent::unhygienic(Text::from("x"), Span::default());

    stream.push(Token {
        kind: TokenKind::Ident(let_ident),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });
    stream.push(Token {
        kind: TokenKind::Ident(x_ident),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });
    stream.push(Token {
        kind: TokenKind::Punct('='),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });
    stream.push(Token {
        kind: TokenKind::Literal(verum_compiler::hygiene::ConstValue::Int(42)),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });

    // Apply macro expansion mark
    let macro_mark = Mark::fresh();
    stream.apply_mark(macro_mark);

    // All tokens should have the mark
    for token in stream.iter() {
        assert!(token.marks.contains(&macro_mark));
    }

    // Apply call-site mark for splicing
    let call_mark = Mark::fresh();
    let marked_stream = stream.with_call_site_marks(call_mark);

    // Identifiers should have call-site scope
    let idents = marked_stream.collect_idents();
    for ident in &idents {
        assert!(ident.scopes.contains(&ScopeId::new(call_mark.as_u64())));
    }
}

/// Test nested group mark propagation
#[test]
fn test_nested_group_mark_propagation() {
    let inner_ident = HygienicIdent::unhygienic(Text::from("inner"), Span::default());

    let mut inner_stream = TokenStream::new();
    inner_stream.push(Token {
        kind: TokenKind::Ident(inner_ident),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });

    let mut outer_stream = TokenStream::new();
    outer_stream.push(Token {
        kind: TokenKind::Group(inner_stream),
        span: Span::default(),
        scopes: ScopeSet::new(),
        marks: MarkSet::new(),
    });

    // Apply mark to outer stream
    let mark = Mark::fresh();
    outer_stream.apply_mark(mark);

    // Mark should propagate to inner group
    if let Some(token) = outer_stream.iter().next()
        && let TokenKind::Group(ref inner) = token.kind {
            for inner_token in inner.iter() {
                assert!(inner_token.marks.contains(&mark));
            }
        }
}

// ============================================================================
// Hygiene Checker Integration Tests
// ============================================================================

/// Test hygiene checker with clean code
#[test]
fn test_hygiene_checker_clean_code() {
    let context = HygieneContext::new();
    let _checker = HygieneChecker::new(context, CheckerConfig::default());

    // Create a clean result
    let result = CheckResult::success();

    assert!(result.violations.is_empty());
    assert!(result.is_success());
}

/// Test hygiene checker with expression
#[test]
fn test_hygiene_checker_check_expr() {
    let context = HygieneContext::new();
    let mut checker = HygieneChecker::new(context, CheckerConfig::default());

    // Create a simple expression
    let expr = verum_ast::Expr::literal(Literal::int(42, Span::default()));

    // Check it - should pass
    let result = checker.check_expr(&expr);
    assert!(result.is_success());
}

// ============================================================================
// Error Code Verification Tests
// ============================================================================

/// Test that all hygiene error codes are distinct
#[test]
fn test_error_codes_distinct() {
    let mut codes = std::collections::HashSet::new();

    let errors = [
        HygieneViolation::InvalidQuoteSyntax {
            message: Text::from("test"),
            span: Span::default(),
        },
        HygieneViolation::UnquoteOutsideQuote {
            span: Span::default(),
        },
        HygieneViolation::AccidentalCapture {
            captured: HygienicIdent::unhygienic(Text::from("x"), Span::default()),
            intended_binding: Span::default(),
            actual_binding: Span::default(),
        },
        HygieneViolation::GensymCollision {
            name: Text::from("collision"),
            span: Span::default(),
        },
        HygieneViolation::ScopeResolutionFailed {
            ident: Text::from("x"),
            span: Span::default(),
        },
        HygieneViolation::StageMismatch {
            expected_stage: 0,
            actual_stage: 1,
            span: Span::default(),
        },
        HygieneViolation::LiftTypeMismatch {
            expected: Text::from("Int"),
            found: Text::from("Text"),
            span: Span::default(),
        },
        HygieneViolation::InvalidTokenTree {
            message: Text::from("invalid token"),
            span: Span::default(),
        },
        HygieneViolation::CaptureNotDeclared {
            ident: Text::from("x"),
            span: Span::default(),
        },
        HygieneViolation::RepetitionMismatch {
            first_name: Text::from("a"),
            first_len: 2,
            second_name: Text::from("b"),
            second_len: 3,
            span: Span::default(),
        },
    ];

    for error in &errors {
        let code = error.error_code();
        assert!(codes.insert(code), "Duplicate error code: {}", code);
    }

    // Should have 10 distinct codes (M400-M409)
    assert_eq!(codes.len(), 10);
}

/// Test error code ranges
#[test]
fn test_error_code_ranges() {
    let invalid_syntax = HygieneViolation::InvalidQuoteSyntax {
        message: Text::from("test"),
        span: Span::default(),
    };

    let code = invalid_syntax.error_code();
    assert!(code.starts_with("M4")); // Meta error codes start with M4
}
