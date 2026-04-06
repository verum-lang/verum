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
    unused_assignments,
    clippy::approx_constant
)]
// Comprehensive tests for the v3.0 Revolutionary Literal System
//
// Tests cover all literal types including:
// - Integer literals (decimal, hex, binary) with suffixes
// - Float literals with suffixes
// - String literals (regular, raw, multiline)
// - Tagged literals (d#, sql#, rx#)
// - Interpolated strings (safe prefixes)
// - Composite literals (mat#, vec#, chem#, music#, interval#)
// - Context-adaptive literals (#FF5733)
// - Contract literals
//
// Tests for literal AST nodes: integers, floats, strings, tagged, composite.

use verum_ast::literal::*;
use verum_ast::span::*;
use verum_common::Text;

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

// ============================================================================
// CORRECTNESS TESTS - Basic Literal Construction
// ============================================================================

#[test]
fn test_int_literal_construction() {
    let span = test_span();
    let lit = Literal::int(42, span);

    match lit.kind {
        LiteralKind::Int(ref int_lit) => {
            assert_eq!(int_lit.value, 42);
            assert_eq!(int_lit.suffix, None);
        }
        _ => panic!("Expected Int literal"),
    }
    assert_eq!(lit.span, span);
}

#[test]
fn test_int_literal_with_suffix() {
    let span = test_span();
    let int_lit = IntLit::with_suffix(42, IntSuffix::I32);
    let lit = Literal::new(LiteralKind::Int(int_lit), span);

    match lit.kind {
        LiteralKind::Int(ref int_lit) => {
            assert_eq!(int_lit.value, 42);
            assert_eq!(int_lit.suffix, Some(IntSuffix::I32));
        }
        _ => panic!("Expected Int literal"),
    }
}

#[test]
fn test_int_literal_all_suffixes() {
    let span = test_span();
    let suffixes = vec![
        IntSuffix::I8,
        IntSuffix::I16,
        IntSuffix::I32,
        IntSuffix::I64,
        IntSuffix::I128,
        IntSuffix::Isize,
        IntSuffix::U8,
        IntSuffix::U16,
        IntSuffix::U32,
        IntSuffix::U64,
        IntSuffix::U128,
        IntSuffix::Usize,
    ];

    for suffix in suffixes {
        let int_lit = IntLit::with_suffix(100, suffix.clone());
        let lit = Literal::new(LiteralKind::Int(int_lit), span);

        match lit.kind {
            LiteralKind::Int(ref int_lit) => {
                assert_eq!(int_lit.suffix, Some(suffix));
            }
            _ => panic!("Expected Int literal"),
        }
    }
}

#[test]
fn test_int_literal_custom_suffix() {
    let span = test_span();
    let custom_suffix = IntSuffix::Custom(Text::from("km"));
    let int_lit = IntLit::with_suffix(100, custom_suffix.clone());
    let lit = Literal::new(LiteralKind::Int(int_lit), span);

    match lit.kind {
        LiteralKind::Int(ref int_lit) => {
            assert!(int_lit.suffix.as_ref().unwrap().is_custom());
            assert_eq!(int_lit.suffix.as_ref().unwrap().as_str(), Text::from("km"));
        }
        _ => panic!("Expected Int literal"),
    }
}

#[test]
fn test_float_literal_construction() {
    let span = test_span();
    let lit = Literal::float(3.14, span);

    match lit.kind {
        LiteralKind::Float(ref float_lit) => {
            assert_eq!(float_lit.value, 3.14);
            assert_eq!(float_lit.suffix, None);
        }
        _ => panic!("Expected Float literal"),
    }
}

#[test]
fn test_float_literal_with_suffix() {
    let span = test_span();
    let float_lit = FloatLit::with_suffix(3.14, FloatSuffix::F32);
    let lit = Literal::new(LiteralKind::Float(float_lit), span);

    match lit.kind {
        LiteralKind::Float(ref float_lit) => {
            assert_eq!(float_lit.value, 3.14);
            assert_eq!(float_lit.suffix, Some(FloatSuffix::F32));
        }
        _ => panic!("Expected Float literal"),
    }
}

#[test]
fn test_float_literal_custom_suffix() {
    let span = test_span();
    let custom_suffix = FloatSuffix::Custom(Text::from("m"));
    let float_lit = FloatLit::with_suffix(9.8, custom_suffix.clone());
    let lit = Literal::new(LiteralKind::Float(float_lit), span);

    match lit.kind {
        LiteralKind::Float(ref float_lit) => {
            assert!(float_lit.suffix.as_ref().unwrap().is_custom());
            assert_eq!(float_lit.suffix.as_ref().unwrap().as_str(), Text::from("m"));
        }
        _ => panic!("Expected Float literal"),
    }
}

#[test]
fn test_string_literal_regular() {
    let span = test_span();
    let lit = Literal::string("hello world".to_string().into(), span);

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert_eq!(str_lit.as_str(), "hello world");
            assert!(matches!(str_lit, StringLit::Regular(_)));
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
/// Test that MultiLine (triple-quoted) strings are raw (no escape processing).
fn test_string_literal_multiline_is_raw() {
    let span = test_span();
    let lit = Literal::new(
        LiteralKind::Text(StringLit::MultiLine(r#"C:\path\to\file"#.to_string().into())),
        span,
    );

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert_eq!(str_lit.as_str(), r#"C:\path\to\file"#);
            assert!(matches!(str_lit, StringLit::MultiLine(_)));
            assert!(str_lit.is_raw());
            assert!(str_lit.is_multiline());
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
/// Test that Regular strings are NOT raw.
fn test_regular_string_not_raw() {
    let span = test_span();
    let lit = Literal::new(
        LiteralKind::Text(StringLit::Regular("hello".to_string().into())),
        span,
    );

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert!(!str_lit.is_raw());
            assert!(!str_lit.is_multiline());
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
fn test_string_literal_multiline() {
    let span = test_span();
    let multiline = "line1\nline2\nline3";
    let lit = Literal::new(
        LiteralKind::Text(StringLit::MultiLine(multiline.to_string().into())),
        span,
    );

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert_eq!(str_lit.as_str(), multiline);
            assert!(matches!(str_lit, StringLit::MultiLine(_)));
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
fn test_char_literal() {
    let span = test_span();
    let lit = Literal::char('x', span);

    match lit.kind {
        LiteralKind::Char(c) => {
            assert_eq!(c, 'x');
        }
        _ => panic!("Expected Char literal"),
    }
}

#[test]
fn test_bool_literal() {
    let span = test_span();

    let lit_true = Literal::bool(true, span);
    match lit_true.kind {
        LiteralKind::Bool(b) => assert!(b),
        _ => panic!("Expected Bool literal"),
    }

    let lit_false = Literal::bool(false, span);
    match lit_false.kind {
        LiteralKind::Bool(b) => assert!(!b),
        _ => panic!("Expected Bool literal"),
    }
}

// ============================================================================
// TAGGED LITERALS TESTS
// ============================================================================

#[test]
fn test_tagged_literal_date() {
    let span = test_span();
    let lit = Literal::tagged(Text::from("d"), Text::from("2025-11-05"), span);

    match lit.kind {
        LiteralKind::Tagged {
            ref tag,
            ref content,
        } => {
            assert_eq!(tag.as_str(), "d");
            assert_eq!(content.as_str(), "2025-11-05");
        }
        _ => panic!("Expected Tagged literal"),
    }
}

#[test]
fn test_tagged_literal_sql() {
    let span = test_span();
    let lit = Literal::tagged(Text::from("sql"), Text::from("SELECT * FROM users"), span);

    match lit.kind {
        LiteralKind::Tagged {
            ref tag,
            ref content,
        } => {
            assert_eq!(tag.as_str(), "sql");
            assert_eq!(content.as_str(), "SELECT * FROM users");
        }
        _ => panic!("Expected Tagged literal"),
    }
}

#[test]
fn test_tagged_literal_regex() {
    let span = test_span();
    let lit = Literal::tagged(Text::from("rx"), Text::from(r"\d{3}-\d{4}"), span);

    match lit.kind {
        LiteralKind::Tagged {
            ref tag,
            ref content,
        } => {
            assert_eq!(tag.as_str(), "rx");
            assert_eq!(content.as_str(), r"\d{3}-\d{4}");
        }
        _ => panic!("Expected Tagged literal"),
    }
}

// ============================================================================
// INTERPOLATED STRING TESTS
// ============================================================================

#[test]
fn test_interpolated_string_format() {
    let span = test_span();
    let lit = Literal::interpolated_string(Text::from("f"), Text::from("Hello {name}"), span);

    match lit.kind {
        LiteralKind::InterpolatedString(ref interp) => {
            assert_eq!(interp.prefix.as_str(), "f");
            assert_eq!(interp.content.as_str(), "Hello {name}");
            assert!(!interp.is_safe_interpolation());
        }
        _ => panic!("Expected InterpolatedString literal"),
    }
}

#[test]
fn test_interpolated_string_sql_safe() {
    let span = test_span();
    let lit = Literal::interpolated_string(
        Text::from("sql"),
        Text::from("SELECT * FROM users WHERE id = {user_id}"),
        span,
    );

    match lit.kind {
        LiteralKind::InterpolatedString(ref interp) => {
            assert_eq!(interp.prefix.as_str(), "sql");
            assert!(interp.is_safe_interpolation());
            assert_eq!(interp.desugaring_target(), Some("SQL.query"));
        }
        _ => panic!("Expected InterpolatedString literal"),
    }
}

#[test]
fn test_interpolated_string_all_safe_prefixes() {
    let span = test_span();
    let safe_prefixes = vec![
        ("sql", "SQL.query"),
        ("html", "HTML.escape"),
        ("uri", "URI.encode"),
        ("json", "JSON.encode"),
        ("xml", "XML.escape"),
        ("gql", "GraphQL.query"),
    ];

    for (prefix, expected_target) in safe_prefixes {
        let lit =
            Literal::interpolated_string(Text::from(prefix), Text::from("content {expr}"), span);

        match lit.kind {
            LiteralKind::InterpolatedString(ref interp) => {
                assert!(interp.is_safe_interpolation());
                assert_eq!(interp.desugaring_target(), Some(expected_target));
            }
            _ => panic!("Expected InterpolatedString literal"),
        }
    }
}

// ============================================================================
// COMPOSITE LITERAL TESTS - domain-specific structured data
// ============================================================================

#[test]
fn test_composite_literal_matrix() {
    let span = test_span();
    let lit = Literal::composite(
        Text::from("mat"),
        Text::from("[[1, 2], [3, 4]]"),
        CompositeDelimiter::Quote,
        span,
    );

    match lit.kind {
        LiteralKind::Composite(ref comp) => {
            assert_eq!(comp.tag.as_str(), "mat");
            assert_eq!(comp.content.as_str(), "[[1, 2], [3, 4]]");
            assert_eq!(comp.delimiter, CompositeDelimiter::Quote);
            assert!(comp.is_recognized());
            assert_eq!(comp.composite_type(), Some(CompositeType::Matrix));
            assert!(comp.validate().is_ok());
        }
        _ => panic!("Expected Composite literal"),
    }
}

#[test]
fn test_composite_literal_vector() {
    let span = test_span();
    let lit = Literal::composite(
        Text::from("vec"),
        Text::from("<1, 2, 3>"),
        CompositeDelimiter::Quote,
        span,
    );

    match lit.kind {
        LiteralKind::Composite(ref comp) => {
            assert_eq!(comp.tag.as_str(), "vec");
            assert!(comp.is_recognized());
            assert_eq!(comp.composite_type(), Some(CompositeType::Vector));
            assert!(comp.validate().is_ok());
        }
        _ => panic!("Expected Composite literal"),
    }
}

#[test]
fn test_composite_literal_chemistry() {
    let span = test_span();
    let lit = Literal::composite(
        Text::from("chem"),
        Text::from("H2O"),
        CompositeDelimiter::Quote,
        span,
    );

    match lit.kind {
        LiteralKind::Composite(ref comp) => {
            assert_eq!(comp.tag.as_str(), "chem");
            assert_eq!(comp.content.as_str(), "H2O");
            assert!(comp.is_recognized());
            assert_eq!(comp.composite_type(), Some(CompositeType::Chemistry));
            assert!(comp.validate().is_ok());
        }
        _ => panic!("Expected Composite literal"),
    }
}

#[test]
fn test_composite_literal_music() {
    let span = test_span();
    let lit = Literal::composite(
        Text::from("music"),
        Text::from("C4 D4 E4 F4"),
        CompositeDelimiter::Quote,
        span,
    );

    match lit.kind {
        LiteralKind::Composite(ref comp) => {
            assert_eq!(comp.tag.as_str(), "music");
            assert!(comp.is_recognized());
            assert_eq!(comp.composite_type(), Some(CompositeType::Music));
            assert!(comp.validate().is_ok());
        }
        _ => panic!("Expected Composite literal"),
    }
}

#[test]
fn test_composite_literal_interval() {
    let span = test_span();
    let lit = Literal::composite(
        Text::from("interval"),
        Text::from("[0, 100)"),
        CompositeDelimiter::Quote,
        span,
    );

    match lit.kind {
        LiteralKind::Composite(ref comp) => {
            assert_eq!(comp.tag.as_str(), "interval");
            assert!(comp.is_recognized());
            assert_eq!(comp.composite_type(), Some(CompositeType::Interval));
            assert!(comp.validate().is_ok());
        }
        _ => panic!("Expected Composite literal"),
    }
}

#[test]
fn test_composite_delimiter_wrap() {
    assert_eq!(CompositeDelimiter::Quote.wrap("test"), "\"test\"");
    assert_eq!(CompositeDelimiter::Paren.wrap("test"), "(test)");
    assert_eq!(CompositeDelimiter::Bracket.wrap("test"), "[test]");
    assert_eq!(CompositeDelimiter::Brace.wrap("test"), "{test}");
}

// ============================================================================
// CONTEXT-ADAPTIVE LITERAL TESTS - type-driven literal interpretation
// ============================================================================

#[test]
fn test_context_adaptive_hex() {
    let span = test_span();
    let lit = Literal::hex_adaptive(0xFF5733, Text::from("#FF5733"), span);

    match lit.kind {
        LiteralKind::ContextAdaptive(ref ctx) => {
            assert_eq!(ctx.raw.as_str(), "#FF5733");
            match ctx.kind {
                ContextAdaptiveKind::Hex(value) => {
                    assert_eq!(value, 0xFF5733);
                }
                _ => panic!("Expected Hex context-adaptive literal"),
            }
        }
        _ => panic!("Expected ContextAdaptive literal"),
    }
}

#[test]
fn test_context_adaptive_numeric() {
    let span = test_span();
    let lit = Literal::context_adaptive(
        ContextAdaptiveKind::Numeric(Text::from("100")),
        Text::from("100"),
        span,
    );

    match lit.kind {
        LiteralKind::ContextAdaptive(ref ctx) => match &ctx.kind {
            ContextAdaptiveKind::Numeric(value) => {
                assert_eq!(value.as_str(), "100");
            }
            _ => panic!("Expected Numeric context-adaptive literal"),
        },
        _ => panic!("Expected ContextAdaptive literal"),
    }
}

#[test]
fn test_context_adaptive_identifier() {
    let span = test_span();
    let lit = Literal::context_adaptive(
        ContextAdaptiveKind::Identifier(Text::from("@username")),
        Text::from("@username"),
        span,
    );

    match lit.kind {
        LiteralKind::ContextAdaptive(ref ctx) => match &ctx.kind {
            ContextAdaptiveKind::Identifier(ident) => {
                assert_eq!(ident.as_str(), "@username");
            }
            _ => panic!("Expected Identifier context-adaptive literal"),
        },
        _ => panic!("Expected ContextAdaptive literal"),
    }
}

// ============================================================================
// CONTRACT LITERAL TESTS
// ============================================================================

#[test]
fn test_contract_literal() {
    let span = test_span();
    let lit = Literal::contract(Text::from("it > 0"), span);

    match lit.kind {
        LiteralKind::Contract(ref content) => {
            assert_eq!(content.as_str(), "it > 0");
        }
        _ => panic!("Expected Contract literal"),
    }
}

// ============================================================================
// EDGE CASE TESTS
// ============================================================================

#[test]
fn test_int_literal_edge_values() {
    let span = test_span();

    // Maximum positive value
    let lit_max = Literal::int(i128::MAX, span);
    match lit_max.kind {
        LiteralKind::Int(ref int_lit) => {
            assert_eq!(int_lit.value, i128::MAX);
        }
        _ => panic!("Expected Int literal"),
    }

    // Minimum negative value
    let lit_min = Literal::int(i128::MIN, span);
    match lit_min.kind {
        LiteralKind::Int(ref int_lit) => {
            assert_eq!(int_lit.value, i128::MIN);
        }
        _ => panic!("Expected Int literal"),
    }

    // Zero
    let lit_zero = Literal::int(0, span);
    match lit_zero.kind {
        LiteralKind::Int(ref int_lit) => {
            assert_eq!(int_lit.value, 0);
        }
        _ => panic!("Expected Int literal"),
    }
}

#[test]
fn test_float_literal_edge_values() {
    let span = test_span();

    // Positive infinity
    let lit_inf = Literal::float(f64::INFINITY, span);
    match lit_inf.kind {
        LiteralKind::Float(ref float_lit) => {
            assert!(float_lit.value.is_infinite() && float_lit.value > 0.0);
        }
        _ => panic!("Expected Float literal"),
    }

    // Negative infinity
    let lit_neg_inf = Literal::float(f64::NEG_INFINITY, span);
    match lit_neg_inf.kind {
        LiteralKind::Float(ref float_lit) => {
            assert!(float_lit.value.is_infinite() && float_lit.value < 0.0);
        }
        _ => panic!("Expected Float literal"),
    }

    // Zero
    let lit_zero = Literal::float(0.0, span);
    match lit_zero.kind {
        LiteralKind::Float(ref float_lit) => {
            assert_eq!(float_lit.value, 0.0);
        }
        _ => panic!("Expected Float literal"),
    }
}

#[test]
fn test_string_literal_empty() {
    let span = test_span();
    let lit = Literal::string("".to_string().into(), span);

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert_eq!(str_lit.as_str(), "");
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
fn test_string_literal_unicode() {
    let span = test_span();
    let unicode = "Hello 世界 🌍";
    let lit = Literal::string(unicode.to_string().into(), span);

    match lit.kind {
        LiteralKind::Text(ref str_lit) => {
            assert_eq!(str_lit.as_str(), unicode);
        }
        _ => panic!("Expected String literal"),
    }
}

#[test]
fn test_char_literal_unicode() {
    let span = test_span();
    let lit = Literal::char('🚀', span);

    match lit.kind {
        LiteralKind::Char(c) => {
            assert_eq!(c, '🚀');
        }
        _ => panic!("Expected Char literal"),
    }
}

// ============================================================================
// VALIDATION TESTS
// ============================================================================

#[test]
fn test_composite_matrix_validation_invalid() {
    let comp = CompositeLiteral::new(
        Text::from("mat"),
        Text::from("[1, 2, 3]"), // Missing outer brackets
        CompositeDelimiter::Quote,
    );

    assert!(comp.validate().is_err());
}

#[test]
fn test_composite_chemistry_validation_invalid() {
    let comp = CompositeLiteral::new(
        Text::from("chem"),
        Text::from("123"), // No element letters
        CompositeDelimiter::Quote,
    );

    assert!(comp.validate().is_err());
}

#[test]
fn test_composite_music_validation_invalid() {
    let comp = CompositeLiteral::new(
        Text::from("music"),
        Text::from("123"), // No note letters
        CompositeDelimiter::Quote,
    );

    assert!(comp.validate().is_err());
}

#[test]
fn test_composite_interval_validation_invalid() {
    // Test with content that has no separator (neither comma nor ..)
    // This is invalid because interval must have start and end values
    let comp = CompositeLiteral::new(
        Text::from("interval"),
        Text::from("100"), // No separator - just a single value
        CompositeDelimiter::Quote,
    );

    assert!(comp.validate().is_err());
}

#[test]
fn test_composite_unknown_type() {
    let comp = CompositeLiteral::new(
        Text::from("unknown"),
        Text::from("content"),
        CompositeDelimiter::Quote,
    );

    assert!(!comp.is_recognized());
    assert_eq!(comp.composite_type(), None);
    assert!(comp.validate().is_err());
}

// ============================================================================
// DISPLAY/DEBUG TESTS
// ============================================================================

#[test]
fn test_int_suffix_display() {
    assert_eq!(IntSuffix::I32.as_str(), Text::from("i32"));
    assert_eq!(IntSuffix::U64.as_str(), Text::from("u64"));
    assert_eq!(
        IntSuffix::Custom(Text::from("km")).as_str(),
        Text::from("km")
    );
}

#[test]
fn test_float_suffix_display() {
    assert_eq!(FloatSuffix::F32.as_str(), Text::from("f32"));
    assert_eq!(FloatSuffix::F64.as_str(), Text::from("f64"));
    assert_eq!(
        FloatSuffix::Custom(Text::from("m")).as_str(),
        Text::from("m")
    );
}

#[test]
fn test_string_literal_display() {
    let regular = StringLit::Regular("hello".to_string().into());
    assert_eq!(format!("{}", regular), "\"hello\"");

    let multiline = StringLit::MultiLine("line1\nline2".to_string().into());
    assert_eq!(format!("{}", multiline), "\"\"\"line1\nline2\"\"\"");

    let raw_path = StringLit::MultiLine("path\\to\\file".to_string().into());
    assert_eq!(format!("{}", raw_path), "\"\"\"path\\to\\file\"\"\"");
}

#[test]
fn test_interpolated_string_display() {
    let interp = InterpolatedStringLit::new(Text::from("sql"), Text::from("SELECT * FROM {table}"));

    assert_eq!(format!("{}", interp), "sql\"SELECT * FROM {table}\"");
}

#[test]
fn test_composite_literal_display() {
    let comp = CompositeLiteral::new(
        Text::from("mat"),
        Text::from("[[1, 2]]"),
        CompositeDelimiter::Quote,
    );

    assert_eq!(format!("{}", comp), "mat#\"[[1, 2]]\"");
}

// ============================================================================
// SAFETY TESTS - No panics
// ============================================================================

#[test]
fn test_literal_construction_never_panics() {
    let span = test_span();

    // All constructors should work
    let _ = Literal::int(42, span);
    let _ = Literal::float(3.14, span);
    let _ = Literal::string("test".to_string().into(), span);
    let _ = Literal::char('x', span);
    let _ = Literal::bool(true, span);
    let _ = Literal::tagged(Text::from("tag"), Text::from("content"), span);
    let _ = Literal::interpolated_string(Text::from("f"), Text::from("{x}"), span);
    let _ = Literal::contract(Text::from("it > 0"), span);
    let _ = Literal::hex_adaptive(0xFF, Text::from("#FF"), span);
    let _ = Literal::composite(
        Text::from("mat"),
        Text::from("[[1]]"),
        CompositeDelimiter::Quote,
        span,
    );
}

#[test]
fn test_validation_never_panics() {
    let test_cases = vec![
        ("mat", ""),
        ("vec", ""),
        ("chem", ""),
        ("music", ""),
        ("interval", ""),
        ("mat", "invalid"),
        ("vec", "<<>>"),
        ("chem", "!!!"),
        ("music", "xyz"),
        ("interval", "???"),
    ];

    for (tag, content) in test_cases {
        let comp = CompositeLiteral::new(
            Text::from(tag),
            Text::from(content),
            CompositeDelimiter::Quote,
        );

        // Should never panic, just return Ok or Err
        let _ = comp.validate();
    }
}

// ============================================================================
// INT BASE TESTS
// ============================================================================

#[test]
fn test_int_base_radix() {
    assert_eq!(IntBase::Decimal.radix(), 10);
    assert_eq!(IntBase::Hexadecimal.radix(), 16);
    assert_eq!(IntBase::Binary.radix(), 2);
}
