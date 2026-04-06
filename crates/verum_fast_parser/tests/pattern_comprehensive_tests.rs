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
//! Comprehensive Pattern Parser Tests
//!
//! This test suite provides exhaustive coverage for ALL PatternKind variants.
//! Tests use direct pattern parsing for structure verification and contextual
//! parsing for integration testing.
//!
//! Pattern kinds tested:
//! 1. Wildcard - `_`
//! 2. Rest - `..`
//! 3. Ident - `x`, `mut x`, `ref x`, `x @ pattern`
//! 4. Literal - `42`, `"hello"`, `true`
//! 5. Tuple - `(a, b)`
//! 6. Array - `[a, b, c]`
//! 7. Slice - `[a, .., b]`
//! 8. Record - `Point { x, y }`
//! 9. Variant - `Some(x)`, `None`
//! 10. Or - `a | b`
//! 11. Reference - `&x`, `&mut x`
//! 12. Range - `1..10`, `1..=10`
//! 13. Paren - `(x)`
//! 14. Active - `Even()`, `ParseInt()(n)` (F# style)
//! 15. And - `Even() & Positive()`
//! 16. Guard - `(x if x > 0)` (RFC 3637)
//! 17. TypeTest - `x is Int`
//! 18. Stream - `stream[a, b, ...rest]`

use verum_ast::pattern::{PatternKind, VariantPatternData};
use verum_ast::{FileId, Module, Pattern};
use verum_fast_parser::VerumParser;
use verum_lexer::{Lexer, Token};

/// Parse a pattern directly (like the internal tests)
fn parse_pattern(source: &str) -> Result<Pattern, verum_fast_parser::error::ParseError> {
    use verum_fast_parser::RecursiveParser;
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);
    parser.parse_pattern()
}

/// Parse and assert success, returning the pattern
fn parse_ok(source: &str) -> Pattern {
    parse_pattern(source).unwrap_or_else(|e| panic!("Failed to parse '{}': {:?}", source, e))
}

/// Parse a full module to test patterns in context
fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Parse failed: {:?}\nSource: {}", e, source))
}

/// Assert that code parses successfully in function context
fn assert_parses_in_fn(code: &str) {
    let wrapped = format!("fn __test__() {{ {} }}", code);
    parse_module(&wrapped);
}

/// Unwrap Paren to get inner pattern (guards are wrapped in parens)
fn unwrap_paren(pattern: &Pattern) -> &Pattern {
    match &pattern.kind {
        PatternKind::Paren(inner) => inner.as_ref(),
        _ => pattern,
    }
}

// ============================================================================
// 1. WILDCARD PATTERN TESTS
// ============================================================================

mod wildcard_patterns {
    use super::*;

    #[test]
    fn wildcard_basic() {
        let pattern = parse_ok("_");
        assert!(matches!(pattern.kind, PatternKind::Wildcard));
    }

    #[test]
    fn wildcard_in_tuple() {
        let pattern = parse_ok("(_, y)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            assert!(matches!(elems[0].kind, PatternKind::Wildcard));
        } else {
            panic!("Expected tuple pattern");
        }
    }

    #[test]
    fn wildcard_in_match_context() {
        assert_parses_in_fn("match x { _ => {} };");
    }

    #[test]
    fn multiple_wildcards() {
        let pattern = parse_ok("(_, _, _)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 3);
            for elem in &elems {
                assert!(matches!(elem.kind, PatternKind::Wildcard));
            }
        } else {
            panic!("Expected tuple pattern");
        }
    }
}

// ============================================================================
// 2. REST PATTERN TESTS
// ============================================================================

mod rest_patterns {
    use super::*;

    #[test]
    fn rest_standalone() {
        let pattern = parse_ok("..");
        assert!(matches!(pattern.kind, PatternKind::Rest));
    }

    #[test]
    fn rest_in_slice_end() {
        let pattern = parse_ok("[first, ..]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert_eq!(before.len(), 1);
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert!(after.is_empty());
        } else {
            panic!("Expected slice pattern, got {:?}", pattern.kind);
        }
    }

    #[test]
    fn rest_in_slice_start() {
        let pattern = parse_ok("[.., last]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert!(before.is_empty());
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert_eq!(after.len(), 1);
        } else {
            panic!("Expected slice pattern");
        }
    }

    #[test]
    fn rest_in_slice_middle() {
        let pattern = parse_ok("[first, .., last]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert_eq!(before.len(), 1);
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert_eq!(after.len(), 1);
        } else {
            panic!("Expected slice pattern");
        }
    }

    #[test]
    fn rest_with_binding() {
        let pattern = parse_ok("[first, ..rest]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert_eq!(before.len(), 1);
            assert!(rest.is_some());
            assert!(after.is_empty());
        } else {
            panic!("Expected slice pattern");
        }
    }

    #[test]
    fn rest_only_in_slice() {
        // [..] - just rest
        let pattern = parse_ok("[..]");
        // This could be an Array pattern with a single Rest element
        // or a Slice pattern - depends on parser implementation
        match pattern.kind {
            PatternKind::Slice { before, rest, after } => {
                assert!(before.is_empty());
                // Bare `..` without binding sets rest: None
                assert!(rest.is_none());
                assert!(after.is_empty());
            }
            PatternKind::Array(elems) => {
                assert_eq!(elems.len(), 1);
                assert!(matches!(elems[0].kind, PatternKind::Rest));
            }
            _ => panic!("Expected slice or array with rest, got {:?}", pattern.kind),
        }
    }
}

// ============================================================================
// 3. IDENTIFIER PATTERN TESTS
// ============================================================================

mod ident_patterns {
    use super::*;

    #[test]
    fn ident_basic() {
        let pattern = parse_ok("x");
        if let PatternKind::Ident {
            by_ref, mutable, name, subpattern
        } = pattern.kind
        {
            assert!(!by_ref);
            assert!(!mutable);
            assert_eq!(name.name.as_str(), "x");
            assert!(subpattern.is_none());
        } else {
            panic!("Expected ident pattern");
        }
    }

    #[test]
    fn ident_mutable() {
        let pattern = parse_ok("mut x");
        if let PatternKind::Ident { mutable, .. } = pattern.kind {
            assert!(mutable);
        } else {
            panic!("Expected mutable ident pattern");
        }
    }

    #[test]
    fn ident_ref() {
        let pattern = parse_ok("ref x");
        if let PatternKind::Ident { by_ref, mutable, .. } = pattern.kind {
            assert!(by_ref);
            assert!(!mutable);
        } else {
            panic!("Expected ref ident pattern");
        }
    }

    #[test]
    fn ident_ref_mut() {
        let pattern = parse_ok("ref mut x");
        if let PatternKind::Ident { by_ref, mutable, .. } = pattern.kind {
            assert!(by_ref);
            assert!(mutable);
        } else {
            panic!("Expected ref mut ident pattern");
        }
    }

    #[test]
    fn ident_with_at_binding() {
        let pattern = parse_ok("x @ Some(y)");
        if let PatternKind::Ident { name, subpattern, .. } = pattern.kind {
            assert_eq!(name.name.as_str(), "x");
            assert!(subpattern.is_some());
            let sub = subpattern.unwrap();
            assert!(matches!(sub.kind, PatternKind::Variant { .. }));
        } else {
            panic!("Expected ident pattern with @ binding");
        }
    }

    #[test]
    fn ident_at_binding_with_range() {
        let pattern = parse_ok("n @ 1..=10");
        if let PatternKind::Ident { name, subpattern, .. } = pattern.kind {
            assert_eq!(name.name.as_str(), "n");
            assert!(subpattern.is_some());
            let sub = subpattern.unwrap();
            assert!(matches!(sub.kind, PatternKind::Range { .. }));
        } else {
            panic!("Expected ident @ range pattern");
        }
    }
}

// ============================================================================
// 4. LITERAL PATTERN TESTS
// ============================================================================

mod literal_patterns {
    use super::*;
    use verum_ast::literal::LiteralKind;

    #[test]
    fn literal_integer() {
        let pattern = parse_ok("42");
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Int(_)));
        } else {
            panic!("Expected literal pattern");
        }
    }

    #[test]
    fn literal_negative_integer() {
        let pattern = parse_ok("-42");
        if let PatternKind::Literal(lit) = pattern.kind {
            if let LiteralKind::Int(int_lit) = &lit.kind {
                assert_eq!(int_lit.value, -42);
            } else {
                panic!("Expected int literal");
            }
        } else {
            panic!("Expected literal pattern");
        }
    }

    #[test]
    fn literal_hex() {
        let pattern = parse_ok("0xFF");
        assert!(matches!(pattern.kind, PatternKind::Literal(_)));
    }

    #[test]
    fn literal_binary() {
        let pattern = parse_ok("0b1010");
        assert!(matches!(pattern.kind, PatternKind::Literal(_)));
    }

    #[test]
    fn literal_float() {
        let pattern = parse_ok("3.14");
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Float(_)));
        } else {
            panic!("Expected float literal pattern");
        }
    }

    #[test]
    fn literal_string() {
        let pattern = parse_ok(r#""hello""#);
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Text(_)));
        } else {
            panic!("Expected string literal pattern");
        }
    }

    #[test]
    fn literal_char() {
        let pattern = parse_ok("'a'");
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Char(_)));
        } else {
            panic!("Expected char literal pattern");
        }
    }

    #[test]
    fn literal_true() {
        let pattern = parse_ok("true");
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Bool(true)));
        } else {
            panic!("Expected true literal pattern");
        }
    }

    #[test]
    fn literal_false() {
        let pattern = parse_ok("false");
        if let PatternKind::Literal(lit) = pattern.kind {
            assert!(matches!(lit.kind, LiteralKind::Bool(false)));
        } else {
            panic!("Expected false literal pattern");
        }
    }
}

// ============================================================================
// 5. TUPLE PATTERN TESTS
// ============================================================================

mod tuple_patterns {
    use super::*;

    #[test]
    fn tuple_empty() {
        let pattern = parse_ok("()");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert!(elems.is_empty());
        } else {
            panic!("Expected empty tuple pattern");
        }
    }

    #[test]
    fn tuple_single() {
        let pattern = parse_ok("(x,)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 1);
        } else {
            panic!("Expected single-element tuple pattern");
        }
    }

    #[test]
    fn tuple_pair() {
        let pattern = parse_ok("(x, y)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
        } else {
            panic!("Expected pair tuple pattern");
        }
    }

    #[test]
    fn tuple_nested() {
        let pattern = parse_ok("((a, b), (c, d))");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            assert!(matches!(elems[0].kind, PatternKind::Tuple(_)));
            assert!(matches!(elems[1].kind, PatternKind::Tuple(_)));
        } else {
            panic!("Expected nested tuple pattern");
        }
    }

    #[test]
    fn tuple_with_wildcards() {
        let pattern = parse_ok("(_, x, _)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 3);
            assert!(matches!(elems[0].kind, PatternKind::Wildcard));
            assert!(matches!(elems[2].kind, PatternKind::Wildcard));
        } else {
            panic!("Expected tuple with wildcards");
        }
    }

    #[test]
    fn tuple_trailing_comma() {
        let pattern = parse_ok("(a, b, c,)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 3);
        } else {
            panic!("Expected tuple with trailing comma");
        }
    }
}

// ============================================================================
// 6. ARRAY PATTERN TESTS
// ============================================================================

mod array_patterns {
    use super::*;

    #[test]
    fn array_empty() {
        let pattern = parse_ok("[]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert!(elems.is_empty());
        } else {
            panic!("Expected empty array pattern");
        }
    }

    #[test]
    fn array_single() {
        let pattern = parse_ok("[x]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert_eq!(elems.len(), 1);
        } else {
            panic!("Expected single-element array pattern");
        }
    }

    #[test]
    fn array_multiple() {
        let pattern = parse_ok("[a, b, c]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert_eq!(elems.len(), 3);
        } else {
            panic!("Expected multi-element array pattern");
        }
    }

    #[test]
    fn array_nested() {
        let pattern = parse_ok("[[a, b], [c, d]]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            assert!(matches!(elems[0].kind, PatternKind::Array(_)));
        } else {
            panic!("Expected nested array pattern");
        }
    }
}

// ============================================================================
// 7. SLICE PATTERN TESTS (with rest)
// ============================================================================

mod slice_patterns {
    use super::*;

    #[test]
    fn slice_with_rest_end() {
        let pattern = parse_ok("[a, b, ..]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert_eq!(before.len(), 2);
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert!(after.is_empty());
        } else {
            panic!("Expected slice pattern with rest at end");
        }
    }

    #[test]
    fn slice_with_rest_start() {
        let pattern = parse_ok("[.., y, z]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert!(before.is_empty());
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert_eq!(after.len(), 2);
        } else {
            panic!("Expected slice pattern with rest at start");
        }
    }

    #[test]
    fn slice_with_rest_middle() {
        let pattern = parse_ok("[a, b, .., y, z]");
        if let PatternKind::Slice { before, rest, after } = pattern.kind {
            assert_eq!(before.len(), 2);
            // Bare `..` without binding sets rest: None
            assert!(rest.is_none());
            assert_eq!(after.len(), 2);
        } else {
            panic!("Expected slice pattern with rest in middle");
        }
    }
}

// ============================================================================
// 8. RECORD PATTERN TESTS
// ============================================================================

mod record_patterns {
    use super::*;

    #[test]
    fn record_single_field() {
        let pattern = parse_ok("Point { x }");
        if let PatternKind::Record { path, fields, rest } = pattern.kind {
            assert_eq!(path.segments.len(), 1);
            assert_eq!(fields.len(), 1);
            assert!(!rest);
        } else {
            panic!("Expected record pattern");
        }
    }

    #[test]
    fn record_multiple_fields() {
        let pattern = parse_ok("Point { x, y, z }");
        if let PatternKind::Record { fields, .. } = pattern.kind {
            assert_eq!(fields.len(), 3);
        } else {
            panic!("Expected record pattern with multiple fields");
        }
    }

    #[test]
    fn record_with_renaming() {
        let pattern = parse_ok("Point { x: px, y: py }");
        if let PatternKind::Record { fields, .. } = pattern.kind {
            assert_eq!(fields.len(), 2);
            // Field patterns should have pattern specified
            assert!(fields[0].pattern.is_some());
        } else {
            panic!("Expected record pattern with renaming");
        }
    }

    #[test]
    fn record_with_rest() {
        let pattern = parse_ok("Point { x, .. }");
        if let PatternKind::Record { fields, rest, .. } = pattern.kind {
            assert_eq!(fields.len(), 1);
            assert!(rest);
        } else {
            panic!("Expected record pattern with rest");
        }
    }

    #[test]
    fn record_nested() {
        let pattern = parse_ok("Outer { inner: Point { x, y } }");
        if let PatternKind::Record { fields, .. } = pattern.kind {
            assert_eq!(fields.len(), 1);
            let inner = fields[0].pattern.as_ref().unwrap();
            assert!(matches!(inner.kind, PatternKind::Record { .. }));
        } else {
            panic!("Expected nested record pattern");
        }
    }
}

// ============================================================================
// 9. VARIANT PATTERN TESTS
// ============================================================================

mod variant_patterns {
    use super::*;

    #[test]
    fn variant_unit() {
        let pattern = parse_ok("None");
        if let PatternKind::Variant { path, data } = pattern.kind {
            assert_eq!(path.segments.len(), 1);
            assert!(data.is_none());
        } else {
            panic!("Expected unit variant pattern");
        }
    }

    #[test]
    fn variant_tuple_single() {
        let pattern = parse_ok("Some(x)");
        if let PatternKind::Variant { data, .. } = pattern.kind {
            if let Some(VariantPatternData::Tuple(elems)) = data {
                assert_eq!(elems.len(), 1);
            } else {
                panic!("Expected tuple variant data");
            }
        } else {
            panic!("Expected tuple variant pattern");
        }
    }

    #[test]
    fn variant_tuple_multiple() {
        let pattern = parse_ok("Color(r, g, b)");
        if let PatternKind::Variant { data, .. } = pattern.kind {
            if let Some(VariantPatternData::Tuple(elems)) = data {
                assert_eq!(elems.len(), 3);
            } else {
                panic!("Expected tuple variant data");
            }
        } else {
            panic!("Expected variant pattern");
        }
    }

    #[test]
    fn variant_nested() {
        let pattern = parse_ok("Some(Ok(x))");
        if let PatternKind::Variant { data, .. } = pattern.kind {
            if let Some(VariantPatternData::Tuple(elems)) = data {
                assert!(matches!(elems[0].kind, PatternKind::Variant { .. }));
            } else {
                panic!("Expected nested variant");
            }
        } else {
            panic!("Expected variant pattern");
        }
    }

    #[test]
    fn variant_qualified_path() {
        let pattern = parse_ok("MyEnum.Variant(x)");
        if let PatternKind::Variant { path, .. } = pattern.kind {
            assert_eq!(path.segments.len(), 2);
        } else {
            panic!("Expected qualified variant pattern");
        }
    }
}

// ============================================================================
// 10. OR PATTERN TESTS
// ============================================================================

mod or_patterns {
    use super::*;

    #[test]
    fn or_two_patterns() {
        let pattern = parse_ok("a | b");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
        } else {
            panic!("Expected or pattern");
        }
    }

    #[test]
    fn or_three_patterns() {
        let pattern = parse_ok("a | b | c");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 3);
        } else {
            panic!("Expected or pattern with 3 alternatives");
        }
    }

    #[test]
    fn or_literals() {
        let pattern = parse_ok("1 | 2 | 3");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 3);
            for alt in &alts {
                assert!(matches!(alt.kind, PatternKind::Literal(_)));
            }
        } else {
            panic!("Expected or pattern with literals");
        }
    }

    #[test]
    fn or_variants() {
        let pattern = parse_ok("Some(x) | None");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
        } else {
            panic!("Expected or pattern with variants");
        }
    }

    #[test]
    fn or_nested_tuples() {
        let pattern = parse_ok("(1, x) | (2, x)");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
            for alt in &alts {
                assert!(matches!(alt.kind, PatternKind::Tuple(_)));
            }
        } else {
            panic!("Expected or pattern with tuples");
        }
    }
}

// ============================================================================
// 11. REFERENCE PATTERN TESTS
// ============================================================================

mod reference_patterns {
    use super::*;

    #[test]
    fn reference_immutable() {
        let pattern = parse_ok("&x");
        if let PatternKind::Reference { mutable, inner } = pattern.kind {
            assert!(!mutable);
            assert!(matches!(inner.kind, PatternKind::Ident { .. }));
        } else {
            panic!("Expected immutable reference pattern");
        }
    }

    #[test]
    fn reference_mutable() {
        let pattern = parse_ok("&mut x");
        if let PatternKind::Reference { mutable, inner } = pattern.kind {
            assert!(mutable);
            assert!(matches!(inner.kind, PatternKind::Ident { .. }));
        } else {
            panic!("Expected mutable reference pattern");
        }
    }

    #[test]
    fn reference_with_variant() {
        let pattern = parse_ok("&Some(x)");
        if let PatternKind::Reference { inner, .. } = pattern.kind {
            assert!(matches!(inner.kind, PatternKind::Variant { .. }));
        } else {
            panic!("Expected reference to variant pattern");
        }
    }

    #[test]
    fn reference_with_tuple() {
        let pattern = parse_ok("&(a, b)");
        if let PatternKind::Reference { inner, .. } = pattern.kind {
            assert!(matches!(inner.kind, PatternKind::Tuple(_)));
        } else {
            panic!("Expected reference to tuple pattern");
        }
    }

    #[test]
    fn reference_nested() {
        // Nested reference with explicit parens: &(&x)
        let pattern = parse_ok("&(&x)");
        if let PatternKind::Reference { inner, .. } = pattern.kind {
            // inner is Paren(&x)
            let unwrapped = unwrap_paren(&inner);
            assert!(matches!(unwrapped.kind, PatternKind::Reference { .. }));
        } else {
            panic!("Expected nested reference pattern");
        }
    }
}

// ============================================================================
// 12. RANGE PATTERN TESTS
// ============================================================================

mod range_patterns {
    use super::*;

    #[test]
    fn range_exclusive() {
        let pattern = parse_ok("1..10");
        if let PatternKind::Range { start, end, inclusive } = pattern.kind {
            assert!(start.is_some());
            assert!(end.is_some());
            assert!(!inclusive);
        } else {
            panic!("Expected exclusive range pattern");
        }
    }

    #[test]
    fn range_inclusive() {
        let pattern = parse_ok("1..=10");
        if let PatternKind::Range { start, end, inclusive } = pattern.kind {
            assert!(start.is_some());
            assert!(end.is_some());
            assert!(inclusive);
        } else {
            panic!("Expected inclusive range pattern");
        }
    }

    #[test]
    fn range_from() {
        let pattern = parse_ok("100..");
        if let PatternKind::Range { start, end, .. } = pattern.kind {
            assert!(start.is_some());
            assert!(end.is_none());
        } else {
            panic!("Expected range-from pattern");
        }
    }

    #[test]
    fn range_to() {
        let pattern = parse_ok("..10");
        if let PatternKind::Range { start, end, .. } = pattern.kind {
            assert!(start.is_none());
            assert!(end.is_some());
        } else {
            panic!("Expected range-to pattern");
        }
    }

    #[test]
    fn range_char() {
        let pattern = parse_ok("'a'..='z'");
        if let PatternKind::Range { start, end, inclusive } = pattern.kind {
            assert!(start.is_some());
            assert!(end.is_some());
            assert!(inclusive);
        } else {
            panic!("Expected char range pattern");
        }
    }
}

// ============================================================================
// 13. PAREN PATTERN TESTS
// ============================================================================

mod paren_patterns {
    use super::*;

    #[test]
    fn paren_simple() {
        let pattern = parse_ok("(x)");
        // (x) could be Paren or just the inner ident - both are valid
        assert!(
            matches!(pattern.kind, PatternKind::Paren(_))
                || matches!(pattern.kind, PatternKind::Ident { .. })
        );
    }

    #[test]
    fn paren_nested() {
        let pattern = parse_ok("((x))");
        // Nested parens
        if let PatternKind::Paren(inner) = pattern.kind {
            assert!(
                matches!(inner.kind, PatternKind::Paren(_))
                    || matches!(inner.kind, PatternKind::Ident { .. })
            );
        }
    }
}

// ============================================================================
// 14. ACTIVE PATTERN TESTS (F#-style user-defined patterns)
// Active pattern tests: user-defined pattern decomposition functions
// ============================================================================

mod active_patterns {
    use super::*;

    #[test]
    fn active_total_no_params() {
        // Even() - total pattern, no params
        let pattern = parse_ok("Even()");
        if let PatternKind::Active { name, params, bindings } = pattern.kind {
            assert_eq!(name.name.as_str(), "Even");
            assert!(params.is_empty());
            assert!(bindings.is_empty());
        } else {
            panic!("Expected active pattern, got {:?}", pattern.kind);
        }
    }

    #[test]
    fn active_total_with_params() {
        // InRange(0, 100)() - parameterized total pattern
        let pattern = parse_ok("InRange(0, 100)()");
        if let PatternKind::Active { name, params, bindings } = pattern.kind {
            assert_eq!(name.name.as_str(), "InRange");
            assert_eq!(params.len(), 2);
            assert!(bindings.is_empty());
        } else {
            panic!("Expected active pattern with params");
        }
    }

    #[test]
    fn active_partial_single_binding() {
        // ParseInt()(n) - partial pattern with extraction
        let pattern = parse_ok("ParseInt()(n)");
        if let PatternKind::Active { name, params, bindings } = pattern.kind {
            assert_eq!(name.name.as_str(), "ParseInt");
            assert!(params.is_empty());
            assert_eq!(bindings.len(), 1);
        } else {
            panic!("Expected partial active pattern");
        }
    }

    #[test]
    fn active_partial_multiple_bindings() {
        // HeadTail()(h, t) - partial pattern with multiple bindings
        let pattern = parse_ok("HeadTail()(h, t)");
        if let PatternKind::Active { name, bindings, .. } = pattern.kind {
            assert_eq!(name.name.as_str(), "HeadTail");
            assert_eq!(bindings.len(), 2);
        } else {
            panic!("Expected active pattern with multiple bindings");
        }
    }

    #[test]
    fn active_partial_with_params() {
        // RegexMatch("\\d+")(matched) - parameterized partial pattern
        let pattern = parse_ok(r#"RegexMatch("\\d+")(matched)"#);
        if let PatternKind::Active { name, params, bindings } = pattern.kind {
            assert_eq!(name.name.as_str(), "RegexMatch");
            assert_eq!(params.len(), 1);
            assert_eq!(bindings.len(), 1);
        } else {
            panic!("Expected parameterized partial active pattern");
        }
    }

    #[test]
    fn active_partial_complex_bindings() {
        // SplitAt(2, 3)(before, middle, after) - multi-param multi-binding
        let pattern = parse_ok("SplitAt(2, 3)(before, middle, after)");
        if let PatternKind::Active { name, params, bindings } = pattern.kind {
            assert_eq!(name.name.as_str(), "SplitAt");
            assert_eq!(params.len(), 2);
            assert_eq!(bindings.len(), 3);
        } else {
            panic!("Expected complex active pattern");
        }
    }

    #[test]
    fn active_in_or_pattern() {
        // Even() | Odd() - active patterns in or
        let pattern = parse_ok("Even() | Odd()");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
            for alt in &alts {
                assert!(matches!(alt.kind, PatternKind::Active { .. }));
            }
        } else {
            panic!("Expected or pattern with active patterns");
        }
    }

    #[test]
    fn active_nested_binding() {
        // ParseInt()(Some(n)) - nested pattern in binding
        let pattern = parse_ok("ParseInt()(Some(n))");
        if let PatternKind::Active { name, bindings, .. } = pattern.kind {
            assert_eq!(name.name.as_str(), "ParseInt");
            assert_eq!(bindings.len(), 1);
            // The binding should be a variant pattern
            assert!(matches!(bindings[0].kind, PatternKind::Variant { .. }));
        } else {
            panic!("Expected active pattern with nested binding");
        }
    }
}

// ============================================================================
// 15. AND PATTERN TESTS (Pattern Combination)
// Active pattern tests: user-defined pattern decomposition functions
// ============================================================================

mod and_patterns {
    use super::*;

    #[test]
    fn and_two_active_patterns() {
        // Even() & Positive() - both must match
        let pattern = parse_ok("Even() & Positive()");
        if let PatternKind::And(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 2);
            for p in &patterns {
                assert!(matches!(p.kind, PatternKind::Active { .. }));
            }
        } else {
            panic!("Expected and pattern");
        }
    }

    #[test]
    fn and_three_patterns() {
        // A() & B() & C() - chain of patterns
        let pattern = parse_ok("Even() & Positive() & Small()");
        if let PatternKind::And(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 3);
        } else {
            panic!("Expected and pattern with 3 elements");
        }
    }

    #[test]
    fn and_mixed_patterns() {
        // Active & identifier - can combine different pattern kinds
        let pattern = parse_ok("Even() & n");
        if let PatternKind::And(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 2);
            assert!(matches!(patterns[0].kind, PatternKind::Active { .. }));
            assert!(matches!(patterns[1].kind, PatternKind::Ident { .. }));
        } else {
            panic!("Expected mixed and pattern");
        }
    }

    #[test]
    fn and_in_tuple() {
        // (Even() & x, y) - and pattern inside tuple
        let pattern = parse_ok("(Even() & x, y)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            assert!(matches!(elems[0].kind, PatternKind::And(_)));
        } else {
            panic!("Expected tuple with and pattern");
        }
    }
}

// ============================================================================
// 16. GUARD PATTERN TESTS (RFC 3637 - Nested Guards)
// ============================================================================

mod guard_patterns {
    use super::*;

    #[test]
    fn guard_in_parens() {
        // (x if x > 0) - guard inside parens, wrapped as Paren(Guard {...})
        let pattern = parse_ok("(x if x > 0)");
        let inner = unwrap_paren(&pattern);
        if let PatternKind::Guard { pattern: guard_inner, guard } = &inner.kind {
            assert!(matches!(guard_inner.kind, PatternKind::Ident { .. }));
            // guard should be an expression
            assert!(!guard.span.is_dummy());
        } else {
            panic!("Expected guard pattern, got {:?}", inner.kind);
        }
    }

    #[test]
    fn guard_in_or_pattern() {
        // (x if x > 0) | (y if y < 0) - guards nested in or
        let pattern = parse_ok("(x if x > 0) | (y if y < 0)");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
            for alt in &alts {
                // Each alternative is Paren(Guard {...})
                let inner = unwrap_paren(alt);
                assert!(matches!(inner.kind, PatternKind::Guard { .. }));
            }
        } else {
            panic!("Expected or pattern with guards");
        }
    }

    #[test]
    fn guard_with_variant() {
        // (Some(x) if x > 0) - guard on variant, wrapped as Paren(Guard {...})
        let pattern = parse_ok("(Some(x) if x > 0)");
        let inner = unwrap_paren(&pattern);
        if let PatternKind::Guard { pattern: guard_inner, .. } = &inner.kind {
            assert!(matches!(guard_inner.kind, PatternKind::Variant { .. }));
        } else {
            panic!("Expected guard pattern, got {:?}", inner.kind);
        }
    }

    #[test]
    fn guard_complex_condition() {
        // (n if n > 0 && n < 100) - complex guard expression, wrapped as Paren(Guard {...})
        let pattern = parse_ok("(n if n > 0 && n < 100)");
        let inner = unwrap_paren(&pattern);
        if let PatternKind::Guard { pattern: guard_inner, .. } = &inner.kind {
            assert!(matches!(guard_inner.kind, PatternKind::Ident { .. }));
        } else {
            panic!("Expected guard pattern with complex condition, got {:?}", inner.kind);
        }
    }

    #[test]
    fn guard_in_tuple() {
        // ((x if x > 0), y) - guard inside tuple element, wrapped as Paren(Guard {...})
        let pattern = parse_ok("((x if x > 0), y)");
        if let PatternKind::Tuple(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            // First element is Paren(Guard {...})
            let inner = unwrap_paren(&elems[0]);
            assert!(matches!(inner.kind, PatternKind::Guard { .. }));
        } else {
            panic!("Expected tuple with guard");
        }
    }

    #[test]
    fn guard_nested_in_array() {
        // [(x if x > 0), y] - guard inside array element, wrapped as Paren(Guard {...})
        let pattern = parse_ok("[(x if x > 0), y]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert_eq!(elems.len(), 2);
            // First element is Paren(Guard {...})
            let inner = unwrap_paren(&elems[0]);
            assert!(matches!(inner.kind, PatternKind::Guard { .. }));
        } else {
            panic!("Expected array with guard");
        }
    }

    #[test]
    fn guard_different_conditions_per_alternative() {
        // Per RFC 3637: each or-alternative can have its own guard
        let pattern = parse_ok("(Regular if credit >= 100) | (Premium if credit >= 80)");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
            for alt in &alts {
                // Each alternative is Paren(Guard {...})
                let inner = unwrap_paren(alt);
                assert!(matches!(inner.kind, PatternKind::Guard { .. }));
            }
        } else {
            panic!("Expected or pattern with per-alternative guards");
        }
    }
}

// ============================================================================
// 17. TYPE TEST PATTERN TESTS
// Unknown type pattern: `x is unknown` for safe dynamic typing
// ============================================================================

mod type_test_patterns {
    use super::*;

    #[test]
    fn type_test_basic() {
        // x is Int - test if value is Int type
        let pattern = parse_ok("x is Int");
        if let PatternKind::TypeTest { binding, .. } = pattern.kind {
            assert_eq!(binding.name.as_str(), "x");
        } else {
            panic!("Expected type test pattern, got {:?}", pattern.kind);
        }
    }

    #[test]
    fn type_test_text() {
        // s is Text - test for Text type
        let pattern = parse_ok("s is Text");
        if let PatternKind::TypeTest { binding, .. } = pattern.kind {
            assert_eq!(binding.name.as_str(), "s");
        } else {
            panic!("Expected type test pattern");
        }
    }

    #[test]
    fn type_test_in_or_pattern() {
        // x is Int | y is Float - multiple type tests
        let pattern = parse_ok("x is Int | y is Float");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
            for alt in &alts {
                assert!(matches!(alt.kind, PatternKind::TypeTest { .. }));
            }
        } else {
            panic!("Expected or pattern with type tests");
        }
    }

    #[test]
    fn type_test_generic() {
        // x is List<Int> - generic type test
        let pattern = parse_ok("x is List<Int>");
        if let PatternKind::TypeTest { binding, .. } = pattern.kind {
            assert_eq!(binding.name.as_str(), "x");
        } else {
            panic!("Expected generic type test pattern");
        }
    }
}

// ============================================================================
// 18. STREAM PATTERN TESTS
// Stream pattern matching: `stream[first, second, ...rest]` for lazy sequences
// ============================================================================

mod stream_patterns {
    use super::*;

    #[test]
    fn stream_empty() {
        // stream[] - match exhausted iterator
        let pattern = parse_ok("stream[]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert!(head_patterns.is_empty());
            assert!(rest.is_none());
        } else {
            panic!("Expected empty stream pattern");
        }
    }

    #[test]
    fn stream_single_head() {
        // stream[first] - consume one element
        let pattern = parse_ok("stream[first]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 1);
            assert!(rest.is_none());
        } else {
            panic!("Expected stream pattern with single head");
        }
    }

    #[test]
    fn stream_multiple_heads() {
        // stream[a, b, c] - consume exact count
        let pattern = parse_ok("stream[a, b, c]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 3);
            assert!(rest.is_none());
        } else {
            panic!("Expected stream pattern with multiple heads");
        }
    }

    #[test]
    fn stream_with_rest_binding() {
        // stream[first, ...rest] - consume one, bind remaining iterator
        let pattern = parse_ok("stream[first, ...rest]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 1);
            assert!(rest.is_some());
            assert_eq!(rest.unwrap().name.as_str(), "rest");
        } else {
            panic!("Expected stream pattern with rest binding");
        }
    }

    #[test]
    fn stream_multiple_heads_with_rest() {
        // stream[a, b, ...tail] - consume multiple, bind rest
        let pattern = parse_ok("stream[a, b, ...tail]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 2);
            assert!(rest.is_some());
        } else {
            panic!("Expected stream with multiple heads and rest");
        }
    }

    #[test]
    fn stream_discard_rest() {
        // stream[first, ...] - consume and discard rest (no binding)
        let pattern = parse_ok("stream[first, ...]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 1);
            // rest should be None when just '...' without binding
            assert!(rest.is_none());
        } else {
            panic!("Expected stream with discarded rest");
        }
    }

    #[test]
    fn stream_only_rest() {
        // stream[...all] - bind entire iterator without consuming
        let pattern = parse_ok("stream[...all]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert!(head_patterns.is_empty());
            assert!(rest.is_some());
            assert_eq!(rest.unwrap().name.as_str(), "all");
        } else {
            panic!("Expected stream pattern with only rest");
        }
    }

    #[test]
    fn stream_nested_patterns() {
        // stream[(a, b), (c, d), ...rest] - tuples in stream
        let pattern = parse_ok("stream[(a, b), (c, d), ...rest]");
        if let PatternKind::Stream { head_patterns, rest } = pattern.kind {
            assert_eq!(head_patterns.len(), 2);
            for head in &head_patterns {
                assert!(matches!(head.kind, PatternKind::Tuple(_)));
            }
            assert!(rest.is_some());
        } else {
            panic!("Expected stream with nested patterns");
        }
    }

    #[test]
    fn stream_with_wildcards() {
        // stream[_, second, ...] - wildcard in stream
        let pattern = parse_ok("stream[_, second, ...]");
        if let PatternKind::Stream { head_patterns, .. } = pattern.kind {
            assert_eq!(head_patterns.len(), 2);
            assert!(matches!(head_patterns[0].kind, PatternKind::Wildcard));
        } else {
            panic!("Expected stream with wildcard");
        }
    }
}

// ============================================================================
// COMPLEX PATTERN COMBINATIONS
// ============================================================================

mod complex_combinations {
    use super::*;

    #[test]
    fn all_pattern_types_in_context() {
        // Comprehensive parsing of multiple pattern types in function context
        assert_parses_in_fn(r#"
            match value {
                _ => {},
                x => {},
                42 => {},
                (a, b) => {},
                [x, y] => {},
                [first, .., last] => {},
                Point { x, y } => {},
                Some(x) => {},
                a | b => {},
                &x => {},
                1..=10 => {},
                (x) => {},
            };
        "#);
    }

    #[test]
    fn deeply_nested_patterns() {
        // Very deep nesting
        let pattern = parse_ok("Some(Ok((a, [x, y, z], Point { px, py })))");
        if let PatternKind::Variant { data, .. } = pattern.kind {
            assert!(data.is_some());
        } else {
            panic!("Expected deeply nested pattern");
        }
    }

    #[test]
    fn guard_with_and_pattern() {
        // Guard combined with and pattern inside parens, wrapped as Paren(Guard {...})
        let pattern = parse_ok("(Even() & n if n > 0)");
        let inner = unwrap_paren(&pattern);
        if let PatternKind::Guard { pattern: guard_inner, .. } = &inner.kind {
            assert!(matches!(guard_inner.kind, PatternKind::And(_)));
        } else {
            panic!("Expected guard with and pattern, got {:?}", inner.kind);
        }
    }

    #[test]
    fn or_with_guards_and_active() {
        // Complex or pattern with guards and active patterns
        let pattern = parse_ok("(Even() if x > 0) | (Odd() if x < 0)");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
        } else {
            panic!("Expected or with guards and active patterns");
        }
    }

    #[test]
    fn stream_in_context() {
        // Stream pattern in function context
        assert_parses_in_fn("let stream[head, ...tail] = iterator;");
    }

    #[test]
    fn type_test_in_complex_context() {
        // Type test with other patterns
        let pattern = parse_ok("(x is Int) | (x is Float)");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 2);
        } else {
            panic!("Expected or with type tests");
        }
    }

    #[test]
    fn at_binding_with_guard() {
        // @ binding combined with guard, wrapped as Paren(Guard {...})
        let pattern = parse_ok("(n @ Some(x) if x > 0)");
        let inner = unwrap_paren(&pattern);
        if let PatternKind::Guard { pattern: guard_inner, .. } = &inner.kind {
            // Check it's an ident with @ binding (subpattern is Maybe<Heap<Pattern>>)
            if let PatternKind::Ident { subpattern, .. } = &guard_inner.kind {
                assert!(subpattern.is_some());
            } else {
                panic!("Expected ident pattern with @ binding, got {:?}", guard_inner.kind);
            }
        } else {
            panic!("Expected guard with @ binding, got {:?}", inner.kind);
        }
    }
}

// ============================================================================
// EDGE CASES AND BOUNDARY CONDITIONS
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn many_or_alternatives() {
        // Many alternatives in or pattern
        let pattern = parse_ok("1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10");
        if let PatternKind::Or(alts) = pattern.kind {
            assert_eq!(alts.len(), 10);
        } else {
            panic!("Expected or pattern with many alternatives");
        }
    }

    #[test]
    fn many_and_patterns() {
        // Many patterns combined with and
        let pattern = parse_ok("A() & B() & C() & D() & E()");
        if let PatternKind::And(patterns) = pattern.kind {
            assert_eq!(patterns.len(), 5);
        } else {
            panic!("Expected and pattern with many elements");
        }
    }

    #[test]
    fn deep_tuple_nesting() {
        let pattern = parse_ok("((((a))))");
        // Deep parens should work
        assert!(!pattern.span.is_dummy());
    }

    #[test]
    fn deep_array_nesting() {
        let pattern = parse_ok("[[[a]]]");
        if let PatternKind::Array(elems) = pattern.kind {
            assert_eq!(elems.len(), 1);
            assert!(matches!(elems[0].kind, PatternKind::Array(_)));
        } else {
            panic!("Expected nested array");
        }
    }

    #[test]
    fn empty_struct_fields() {
        // Record with just rest
        let pattern = parse_ok("Point { .. }");
        if let PatternKind::Record { fields, rest, .. } = pattern.kind {
            assert!(fields.is_empty());
            assert!(rest);
        } else {
            panic!("Expected record with just rest");
        }
    }

    #[test]
    fn single_field_record() {
        let pattern = parse_ok("Point { x }");
        if let PatternKind::Record { fields, .. } = pattern.kind {
            assert_eq!(fields.len(), 1);
        } else {
            panic!("Expected single-field record");
        }
    }

    #[test]
    fn pattern_in_for_loop() {
        // Patterns in for loops
        assert_parses_in_fn("for (k, v) in pairs { }");
        assert_parses_in_fn("for Point { x, y } in points { }");
        assert_parses_in_fn("for _ in items { }");
    }

    #[test]
    fn pattern_in_if_let() {
        // Patterns in if-let
        assert_parses_in_fn("if let Some(x) = opt { }");
        assert_parses_in_fn("if let (a, b) = pair { }");
    }

    #[test]
    fn pattern_in_while_let() {
        // Patterns in while-let
        assert_parses_in_fn("while let Some(x) = iter.next() { }");
    }
}
