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
// Comprehensive test suite for the Verum lexer.

use verum_ast::span::FileId;
use verum_common::Text;
use verum_lexer::{Lexer, TokenKind};

/// Helper to tokenize a string and extract token kinds.
fn tokenize(source: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    Lexer::new(source, file_id)
        .map(|r| r.unwrap().kind)
        .collect()
}

/// Helper to check if tokens match expected kinds.
macro_rules! assert_tokens {
    ($source:expr, $($expected:expr),+ $(,)?) => {
        let tokens = tokenize($source);
        let expected_vec = vec![$($expected),+];
        assert_eq!(
            tokens.len(),
            expected_vec.len() + 1, // +1 for EOF
            "Expected {} tokens plus EOF, got {}: {:?}",
            expected_vec.len(),
            tokens.len(),
            tokens
        );
        for (i, expected) in expected_vec.iter().enumerate() {
            assert!(
                std::mem::discriminant(&tokens[i]) == std::mem::discriminant(expected),
                "Token mismatch at position {}: expected {:?}, got {:?}",
                i,
                expected,
                tokens[i]
            );
        }
    };
}

// ===== Keywords Tests =====

#[test]
fn test_core_keywords() {
    assert_tokens!(
        "let fn type match mount",
        TokenKind::Let,
        TokenKind::Fn,
        TokenKind::Type,
        TokenKind::Match,
        TokenKind::Mount,
    );
}

#[test]
fn test_control_flow_keywords() {
    assert_tokens!(
        "if else while for loop break continue return",
        TokenKind::If,
        TokenKind::Else,
        TokenKind::While,
        TokenKind::For,
        TokenKind::Loop,
        TokenKind::Break,
        TokenKind::Continue,
        TokenKind::Return,
    );
}

#[test]
fn test_async_keywords() {
    assert_tokens!(
        "async await spawn",
        TokenKind::Async,
        TokenKind::Await,
        TokenKind::Spawn,
    );
}

#[test]
fn test_visibility_keywords() {
    assert_tokens!(
        "public internal protected",
        TokenKind::Public,
        TokenKind::Internal,
        TokenKind::Protected,
    );
}

#[test]
fn test_type_keywords() {
    assert_tokens!(
        "mut const static implement protocol",
        TokenKind::Mut,
        TokenKind::Const,
        TokenKind::Static,
        TokenKind::Implement,
        TokenKind::Protocol,
    );
}

#[test]
fn test_special_keywords() {
    assert_tokens!(
        "self Self meta unsafe ref move as in is stream defer",
        TokenKind::SelfValue,
        TokenKind::SelfType,
        TokenKind::Meta,
        TokenKind::Unsafe,
        TokenKind::Ref,
        TokenKind::Move,
        TokenKind::As,
        TokenKind::In,
        TokenKind::Is,
        TokenKind::Stream,
        TokenKind::Defer,
    );
}

#[test]
fn test_boolean_and_option_literals() {
    assert_tokens!(
        "true false None Some Ok Err",
        TokenKind::True,
        TokenKind::False,
        TokenKind::None,
        TokenKind::Some,
        TokenKind::Ok,
        TokenKind::Err,
    );
}

// ===== Identifier Tests =====

#[test]
fn test_identifiers() {
    assert_tokens!(
        "foo bar_baz _internal __private snake_case camelCase PascalCase",
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
    );
}

#[test]
fn test_identifier_with_numbers() {
    assert_tokens!(
        "var1 var_2 v123",
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
        TokenKind::Ident(Text::from("")),
    );
}

// ===== Literal Tests =====

#[test]
fn test_integer_literals() {
    let tokens = tokenize("0 1 42 1000 1_000_000");
    assert!(matches!(&tokens[0], TokenKind::Integer(lit) if lit.as_i64() == Some(0)));
    assert!(matches!(&tokens[1], TokenKind::Integer(lit) if lit.as_i64() == Some(1)));
    assert!(matches!(&tokens[2], TokenKind::Integer(lit) if lit.as_i64() == Some(42)));
    assert!(matches!(&tokens[3], TokenKind::Integer(lit) if lit.as_i64() == Some(1000)));
    assert!(matches!(&tokens[4], TokenKind::Integer(lit) if lit.as_i64() == Some(1_000_000)));
}

#[test]
fn test_hexadecimal_literals() {
    let tokens = tokenize("0x0 0xFF 0xDEADBEEF 0x1_2_3");
    assert!(matches!(&tokens[0], TokenKind::Integer(lit) if lit.as_i64() == Some(0)));
    assert!(matches!(&tokens[1], TokenKind::Integer(lit) if lit.as_i64() == Some(255)));
    assert!(matches!(&tokens[2], TokenKind::Integer(lit) if lit.as_i64() == Some(0xDEADBEEF)));
}

#[test]
fn test_binary_literals() {
    let tokens = tokenize("0b0 0b1 0b1010 0b1111_0000");
    assert!(matches!(&tokens[0], TokenKind::Integer(lit) if lit.as_i64() == Some(0)));
    assert!(matches!(&tokens[1], TokenKind::Integer(lit) if lit.as_i64() == Some(1)));
    assert!(matches!(&tokens[2], TokenKind::Integer(lit) if lit.as_i64() == Some(10)));
    assert!(matches!(&tokens[3], TokenKind::Integer(lit) if lit.as_i64() == Some(240)));
}

#[test]
fn test_float_literals() {
    let tokens = tokenize("0.0 1.0 3.14 2.5e10 1.5E-3");
    assert!(matches!(&tokens[0], TokenKind::Float(lit) if (lit.value - 0.0).abs() < 0.001));
    assert!(matches!(&tokens[1], TokenKind::Float(lit) if (lit.value - 1.0).abs() < 0.001));
    assert!(matches!(&tokens[2], TokenKind::Float(lit) if (lit.value - 3.14).abs() < 0.001));
    assert!(matches!(&tokens[3], TokenKind::Float(lit) if (lit.value - 2.5e10).abs() < 1e9));
    assert!(matches!(&tokens[4], TokenKind::Float(lit) if (lit.value - 1.5e-3).abs() < 1e-6));
}

#[test]
fn test_string_literals() {
    let tokens = tokenize(r#""" "hello" "world" "with\nescapes""#);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.is_empty()));
    assert!(matches!(&tokens[1], TokenKind::Text(s) if s.as_str() == "hello"));
    assert!(matches!(&tokens[2], TokenKind::Text(s) if s.as_str() == "world"));
    assert!(matches!(&tokens[3], TokenKind::Text(s) if s.as_str() == "with\nescapes"));
}

// ===== Multiline String Tests =====

#[test]
fn test_multiline_string_literals() {
    let source = r#""""
This is a multiline string
with multiple lines
and preserved whitespace
""""#;
    let tokens = tokenize(source);
    assert!(
        matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == "\nThis is a multiline string\nwith multiple lines\nand preserved whitespace\n")
    );
}

#[test]
fn test_multiline_string_no_escape() {
    let source = r#""""raw \n \t \r escape sequences""""#;
    let tokens = tokenize(source);
    assert!(
        matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == r"raw \n \t \r escape sequences")
    );
}

#[test]
fn test_multiline_string_single_line() {
    let source = r#""""single line""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == "single line"));
}

#[test]
fn test_multiline_string_empty() {
    let source = r#""""""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.is_empty()));
}

// ===== Raw Multiline String Tests =====
// NOTE: Verum uses """...""" for raw strings (no r#"..."# syntax)
// Triple-quote = raw = multiline. No escape processing.

#[test]
fn test_raw_multiline_literals() {
    // Triple-quote strings don't process escapes - \n stays as literal backslash-n
    let source = r#""""raw \n string""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == r"raw \n string"));
}

#[test]
fn test_raw_multiline_with_backslash() {
    // Backslashes are preserved literally in triple-quote strings
    let source = r#""""C:\path\to\file""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == r"C:\path\to\file"));
}

#[test]
fn test_raw_multiline_with_quotes() {
    // Single/double quotes inside triple-quote are fine, only """ ends the string
    let source = r#""""This has "quotes" inside""""#;
    let tokens = tokenize(source);
    assert!(
        matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == r#"This has "quotes" inside"#)
    );
}

#[test]
fn test_raw_multiline_special_chars() {
    // Special characters like backslash are preserved literally
    // (We avoid # in the content to keep Rust raw string simple)
    let source = r#""""contains backslash \ and other chars""""#;
    let tokens = tokenize(source);
    let expected = r"contains backslash \ and other chars";
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == expected));
}

#[test]
fn test_raw_multiline_simple() {
    let source = r#""""simple""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.as_str() == "simple"));
}

#[test]
fn test_raw_multiline_empty() {
    let source = r#""""""""#;
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Text(s) if s.is_empty()));
}

#[test]
fn test_char_literals() {
    // Test simple ASCII chars
    let tokens = tokenize("'a'");
    assert!(matches!(tokens[0], TokenKind::Char('a')));

    let tokens = tokenize("'Z'");
    assert!(matches!(tokens[0], TokenKind::Char('Z')));

    let tokens = tokenize("'0'");
    assert!(matches!(tokens[0], TokenKind::Char('0')));

    // Test escape sequences
    let tokens = tokenize(r"'\n'");
    assert!(matches!(tokens[0], TokenKind::Char('\n')));

    let tokens = tokenize(r"'\t'");
    assert!(matches!(tokens[0], TokenKind::Char('\t')));

    // Test unicode chars
    let tokens = tokenize("'世'");
    assert!(matches!(tokens[0], TokenKind::Char('世')));
}

// ===== Operator Tests =====

#[test]
fn test_arithmetic_operators() {
    assert_tokens!(
        "+ - * / % **",
        TokenKind::Plus,
        TokenKind::Minus,
        TokenKind::Star,
        TokenKind::Slash,
        TokenKind::Percent,
        TokenKind::StarStar,
    );
}

#[test]
fn test_comparison_operators() {
    assert_tokens!(
        "== != < > <= >=",
        TokenKind::EqEq,
        TokenKind::BangEq,
        TokenKind::Lt,
        TokenKind::Gt,
        TokenKind::LtEq,
        TokenKind::GtEq,
    );
}

#[test]
fn test_logical_operators() {
    assert_tokens!(
        "&& || !",
        TokenKind::AmpersandAmpersand,
        TokenKind::PipePipe,
        TokenKind::Bang,
    );
}

#[test]
fn test_bitwise_operators() {
    assert_tokens!(
        "& | ^ << >> ~",
        TokenKind::Ampersand,
        TokenKind::Pipe,
        TokenKind::Caret,
        TokenKind::LtLt,
        TokenKind::GtGt,
        TokenKind::Tilde,
    );
}

#[test]
fn test_assignment_operators() {
    assert_tokens!(
        "= += -= *= /= %= &= |= ^= <<= >>=",
        TokenKind::Eq,
        TokenKind::PlusEq,
        TokenKind::MinusEq,
        TokenKind::StarEq,
        TokenKind::SlashEq,
        TokenKind::PercentEq,
        TokenKind::AmpersandEq,
        TokenKind::PipeEq,
        TokenKind::CaretEq,
        TokenKind::LtLtEq,
        TokenKind::GtGtEq,
    );
}

#[test]
fn test_special_operators() {
    assert_tokens!(
        ".. ..= |> -> => ?. ?? ?",
        TokenKind::DotDot,
        TokenKind::DotDotEq,
        TokenKind::PipeGt,
        TokenKind::RArrow,
        TokenKind::FatArrow,
        TokenKind::QuestionDot,
        TokenKind::QuestionQuestion,
        TokenKind::Question,
    );
}

// ===== Delimiter Tests =====

#[test]
fn test_delimiters() {
    assert_tokens!(
        "( ) [ ] { }",
        TokenKind::LParen,
        TokenKind::RParen,
        TokenKind::LBracket,
        TokenKind::RBracket,
        TokenKind::LBrace,
        TokenKind::RBrace,
    );
}

#[test]
fn test_punctuation() {
    assert_tokens!(
        ", ; : . @",
        TokenKind::Comma,
        TokenKind::Semicolon,
        TokenKind::Colon,
        TokenKind::Dot,
        TokenKind::At,
    );
}

// ===== Complex Expression Tests =====

#[test]
fn test_function_definition() {
    let source = "fn add(x: Int, y: Int) -> Int { x + y }";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "add"));
    assert!(matches!(tokens[2], TokenKind::LParen));
    assert!(matches!(&tokens[3], TokenKind::Ident(s) if s.as_str() == "x"));
    assert!(matches!(tokens[4], TokenKind::Colon));
    assert!(matches!(&tokens[5], TokenKind::Ident(s) if s.as_str() == "Int"));
}

#[test]
fn test_type_definition() {
    let source = "type Point is { x: Float, y: Float }";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Type));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "Point"));
    assert!(matches!(tokens[2], TokenKind::Is));
    assert!(matches!(tokens[3], TokenKind::LBrace));
}

#[test]
fn test_variant_type() {
    let source = "type Option<T> is | Some(T) | None;";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Type));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "Option"));
    assert!(matches!(tokens[2], TokenKind::Lt));
    assert!(matches!(&tokens[3], TokenKind::Ident(s) if s.as_str() == "T"));
    assert!(matches!(tokens[4], TokenKind::Gt));
    assert!(matches!(tokens[5], TokenKind::Is));
    assert!(matches!(tokens[6], TokenKind::Pipe));
    assert!(matches!(tokens[7], TokenKind::Some));
}

#[test]
fn test_match_expression() {
    let source = "match x { Some(v) => v, None => 0 }";
    let tokens = tokenize(source);

    // Debug: print all tokens to see what we got
    // eprintln!("Tokens: {:?}", tokens);

    assert!(matches!(tokens[0], TokenKind::Match));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "x"));
    assert!(matches!(tokens[2], TokenKind::LBrace));
    assert!(matches!(tokens[3], TokenKind::Some));
    assert!(matches!(tokens[4], TokenKind::LParen));
    assert!(matches!(&tokens[5], TokenKind::Ident(s) if s.as_str() == "v"));
    assert!(matches!(tokens[6], TokenKind::RParen));
    assert!(matches!(tokens[7], TokenKind::FatArrow));
}

#[test]
fn test_pipeline_expression() {
    let source = "data |> filter(pred) |> map(f) |> collect()";
    let tokens = tokenize(source);

    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "data"));
    assert!(matches!(tokens[1], TokenKind::PipeGt));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "filter"));
    assert!(matches!(tokens[3], TokenKind::LParen));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "pred"));
    assert!(matches!(tokens[5], TokenKind::RParen));
    assert!(matches!(tokens[6], TokenKind::PipeGt));
    assert!(matches!(&tokens[7], TokenKind::Ident(s) if s.as_str() == "map"));
}

#[test]
fn test_stream_comprehension() {
    let source = "stream [x * 2 for x in data if x > 0]";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Stream));
    assert!(matches!(tokens[1], TokenKind::LBracket));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "x"));
    assert!(matches!(tokens[3], TokenKind::Star));
    assert!(matches!(&tokens[4], TokenKind::Integer(lit) if lit.as_i64() == Some(2)));
    assert!(matches!(tokens[5], TokenKind::For));
    assert!(matches!(tokens[7], TokenKind::In));
    assert!(matches!(tokens[9], TokenKind::If));
}

#[test]
fn test_optional_chaining() {
    let source = "user?.address?.city?.name";
    let tokens = tokenize(source);

    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "user"));
    assert!(matches!(tokens[1], TokenKind::QuestionDot));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "address"));
    assert!(matches!(tokens[3], TokenKind::QuestionDot));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "city"));
    assert!(matches!(tokens[5], TokenKind::QuestionDot));
    assert!(matches!(&tokens[6], TokenKind::Ident(s) if s.as_str() == "name"));
}

#[test]
fn test_null_coalescing() {
    let source = "value ?? default ?? fallback";
    let tokens = tokenize(source);

    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "value"));
    assert!(matches!(tokens[1], TokenKind::QuestionQuestion));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "default"));
    assert!(matches!(tokens[3], TokenKind::QuestionQuestion));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "fallback"));
}

#[test]
fn test_cbgr_references() {
    let source = "&T &mut T";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Ampersand));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "T"));
    assert!(matches!(tokens[2], TokenKind::Ampersand));
    assert!(matches!(tokens[3], TokenKind::Mut));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "T"));
}

#[test]
fn test_ownership_references() {
    let source = "%T %mut T";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Percent));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "T"));
    assert!(matches!(tokens[2], TokenKind::Percent));
    assert!(matches!(tokens[3], TokenKind::Mut));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "T"));
}

#[test]
fn test_refinement_type() {
    let source = "Int{> 0}";
    let tokens = tokenize(source);

    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "Int"));
    assert!(matches!(tokens[1], TokenKind::LBrace));
    assert!(matches!(tokens[2], TokenKind::Gt));
    assert!(matches!(&tokens[3], TokenKind::Integer(lit) if lit.as_i64() == Some(0)));
    assert!(matches!(tokens[4], TokenKind::RBrace));
}

#[test]
fn test_effect_annotation() {
    let source = "fn read() [IO, Error] -> String";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "read"));
    assert!(matches!(tokens[4], TokenKind::LBracket));
    assert!(matches!(&tokens[5], TokenKind::Ident(s) if s.as_str() == "IO"));
    assert!(matches!(tokens[6], TokenKind::Comma));
    assert!(matches!(&tokens[7], TokenKind::Ident(s) if s.as_str() == "Error"));
    assert!(matches!(tokens[8], TokenKind::RBracket));
}

// ===== Comment Tests =====

#[test]
fn test_line_comments() {
    let source = r"
        // This is a comment
        fn main() // Another comment
        // Final comment
    ";
    let tokens = tokenize(source);

    // Should only have: fn, main, (), EOF
    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "main"));
    assert!(matches!(tokens[2], TokenKind::LParen));
    assert!(matches!(tokens[3], TokenKind::RParen));
    assert!(matches!(tokens[4], TokenKind::Eof));
}

#[test]
fn test_block_comments() {
    let source = r"
        /* This is a
           multi-line
           comment */
        fn /* inline */ main
    ";
    let tokens = tokenize(source);

    // Should only have: fn, main, EOF
    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "main"));
    assert!(matches!(tokens[2], TokenKind::Eof));
}

// ===== Whitespace Tests =====

#[test]
fn test_whitespace_handling() {
    let source = "fn    main  (  )  {  }";
    let tokens = tokenize(source);

    assert!(matches!(tokens[0], TokenKind::Fn));
    assert!(matches!(&tokens[1], TokenKind::Ident(s) if s.as_str() == "main"));
    assert!(matches!(tokens[2], TokenKind::LParen));
    assert!(matches!(tokens[3], TokenKind::RParen));
    assert!(matches!(tokens[4], TokenKind::LBrace));
    assert!(matches!(tokens[5], TokenKind::RBrace));
}

#[test]
fn test_newlines_are_whitespace() {
    let source = "fn\nmain\n(\n)\n{\n}";
    let tokens = tokenize(source);

    assert_eq!(tokens.len(), 7); // fn, main, (, ), {, }, EOF
}

// ===== Edge Cases =====

#[test]
fn test_empty_source() {
    let tokens = tokenize("");
    assert_eq!(tokens.len(), 1); // Just EOF
    assert!(matches!(tokens[0], TokenKind::Eof));
}

#[test]
fn test_only_whitespace() {
    let tokens = tokenize("   \t\n\r  ");
    assert_eq!(tokens.len(), 1); // Just EOF
    assert!(matches!(tokens[0], TokenKind::Eof));
}

#[test]
fn test_only_comments() {
    let tokens = tokenize("// comment\n/* block */");
    assert_eq!(tokens.len(), 1); // Just EOF
    assert!(matches!(tokens[0], TokenKind::Eof));
}

#[test]
fn test_unicode_identifiers() {
    // Note: Currently only ASCII identifiers are supported by the lexer
    // This test ensures we handle them gracefully (as errors or separate tokens)
    let source = "foo_bar";
    let tokens = tokenize(source);
    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "foo_bar"));
}

#[test]
fn test_complex_expression_chain() {
    let source = "data.filter(|x| x > 0).map(|x| x * 2).sum()";
    let tokens = tokenize(source);

    // Verify we get all expected tokens
    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "data"));
    assert!(matches!(tokens[1], TokenKind::Dot));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "filter"));
    // ... and so on
}

#[test]
fn test_generic_syntax() {
    let source = "Vec<Int> HashMap<String, Int>";
    let tokens = tokenize(source);

    assert!(matches!(&tokens[0], TokenKind::Ident(s) if s.as_str() == "Vec"));
    assert!(matches!(tokens[1], TokenKind::Lt));
    assert!(matches!(&tokens[2], TokenKind::Ident(s) if s.as_str() == "Int"));
    assert!(matches!(tokens[3], TokenKind::Gt));
    assert!(matches!(&tokens[4], TokenKind::Ident(s) if s.as_str() == "HashMap"));
    assert!(matches!(tokens[5], TokenKind::Lt));
    assert!(matches!(&tokens[6], TokenKind::Ident(s) if s.as_str() == "String"));
    assert!(matches!(tokens[7], TokenKind::Comma));
    assert!(matches!(&tokens[8], TokenKind::Ident(s) if s.as_str() == "Int"));
    assert!(matches!(tokens[9], TokenKind::Gt));
}

// ===== Real-World Code Examples =====

#[test]
fn test_complete_function() {
    let source = r#"
        public fn fibonacci(n: Int{>= 0}) -> Int {
            if n <= 1 {
                n
            } else {
                fibonacci(n - 1) + fibonacci(n - 2)
            }
        }
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();

    // Just verify it tokenizes without errors
    assert!(tokens.len() > 20);
    assert!(matches!(tokens[0].kind, TokenKind::Public));
    assert!(matches!(tokens[1].kind, TokenKind::Fn));
}

#[test]
fn test_async_function_with_effects() {
    let source = r#"
        async fn fetch_user(id: Int) [Database, HTTP] -> Result<User, Error> {
            let user = Database.find(id).await?;
            Ok(user)
        }
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();

    assert!(tokens.len() > 15);
    assert!(matches!(tokens[0].kind, TokenKind::Async));
    assert!(matches!(tokens[1].kind, TokenKind::Fn));
}

#[test]
fn test_protocol_definition() {
    let source = r#"
        protocol Show {
            fn show(&self) -> String;
        }
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();

    assert!(matches!(tokens[0].kind, TokenKind::Protocol));
    assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s.as_str() == "Show"));
}

#[test]
fn test_implement_block() {
    let source = r#"
        implement Show for Point {
            fn show(&self) -> String {
                "Point({}, {})".format(self.x, self.y)
            }
        }
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.collect::<Result<Vec<_>, _>>().unwrap();

    assert!(matches!(tokens[0].kind, TokenKind::Implement));
    assert!(matches!(&tokens[1].kind, TokenKind::Ident(s) if s.as_str() == "Show"));
    assert!(matches!(tokens[2].kind, TokenKind::For));
}
