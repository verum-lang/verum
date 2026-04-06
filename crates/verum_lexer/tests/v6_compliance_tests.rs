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
// Comprehensive tests for Verum v6.0-BALANCED specification compliance.
//
// Tests keyword classification (~41 keywords: 3 reserved + contextual),
// literal parsing (numeric, text, interpolated, tagged, contract, hex color),
// operator precedence, and delimiter handling per the Verum lexical grammar.

use verum_ast::span::FileId;
use verum_lexer::{Lexer, Token, TokenKind};
use verum_common::Text;
use verum_common::Maybe;

fn lex(source: &str) -> Vec<TokenKind> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    lexer.filter_map(|r| r.ok()).map(|t| t.kind).collect()
}

fn lex_tokens(source: &str) -> Vec<Token> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    lexer.filter_map(|r| r.ok()).collect()
}

#[test]
fn test_all_essential_keywords() {
    // Verum has ~41 keywords total: 3 reserved (let, fn, is) + contextual keywords.
    // Essential keywords (~20) cover core, primary, control flow, async, and modifiers.

    // Core reserved keywords (3)
    assert!(matches!(lex("let")[0], TokenKind::Let));
    assert!(matches!(lex("fn")[0], TokenKind::Fn));
    assert!(matches!(lex("is")[0], TokenKind::Is));

    // Primary keywords (3)
    assert!(matches!(lex("type")[0], TokenKind::Type));
    assert!(matches!(lex("where")[0], TokenKind::Where));
    assert!(matches!(lex("using")[0], TokenKind::Using));

    // Control flow keywords (9)
    assert!(matches!(lex("if")[0], TokenKind::If));
    assert!(matches!(lex("else")[0], TokenKind::Else));
    assert!(matches!(lex("match")[0], TokenKind::Match));
    assert!(matches!(lex("return")[0], TokenKind::Return));
    assert!(matches!(lex("for")[0], TokenKind::For));
    assert!(matches!(lex("while")[0], TokenKind::While));
    assert!(matches!(lex("loop")[0], TokenKind::Loop));
    assert!(matches!(lex("break")[0], TokenKind::Break));
    assert!(matches!(lex("continue")[0], TokenKind::Continue));

    // Async/Context keywords (5)
    assert!(matches!(lex("async")[0], TokenKind::Async));
    assert!(matches!(lex("await")[0], TokenKind::Await));
    assert!(matches!(lex("spawn")[0], TokenKind::Spawn));
    assert!(matches!(lex("defer")[0], TokenKind::Defer));
    assert!(matches!(lex("try")[0], TokenKind::Try));

    // Modifier keywords (4)
    assert!(matches!(lex("pub")[0], TokenKind::Pub));
    assert!(matches!(lex("mut")[0], TokenKind::Mut));
    assert!(matches!(lex("const")[0], TokenKind::Const));
    assert!(matches!(lex("unsafe")[0], TokenKind::Unsafe));
}

#[test]
fn test_context_keywords() {
    // Context system keywords: `context` declares DI interfaces, `provide` installs providers
    let tokens = lex("context provide");
    assert!(matches!(tokens[0], TokenKind::Context));
    assert!(matches!(tokens[1], TokenKind::Provide));
}

#[test]
fn test_v6_protocol_keywords() {
    // Protocol system: `protocol` defines trait-like interfaces,
    // `implement` provides implementations (replaces Rust's `trait`/`impl`)
    let tokens = lex("protocol implement");
    assert!(matches!(tokens[0], TokenKind::Protocol));
    assert!(matches!(tokens[1], TokenKind::Implement));
}

#[test]
fn test_composite_tagged_literals() {
    // Composite tagged literals: `tag#"content"` for domain-specific data
    // Grammar: composite_literal = identifier '#' composite_body

    // Interval notation
    let tokens = lex_tokens(r#"interval#"[0, 100)""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "interval")
    );

    // Mathematical expressions
    let tokens = lex_tokens(r#"mat#"[[1, 2], [3, 4]]""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "mat")
    );

    let tokens = lex_tokens(r#"vec#"<1, 2, 3>""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "vec")
    );

    // Chemical formulas
    let tokens = lex_tokens(r#"chem#"H2O""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "chem")
    );

    // Musical notation
    let tokens = lex_tokens(r#"music#"Cmaj7""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "music")
    );
}

#[test]
fn test_semantic_tagged_literals() {
    // Semantic tagged literals: `tag#"content"` for compile-time validated data
    // Tags: gql, rx, sql, url, email, json, xml, yaml (semantic_tag in grammar)

    let tokens = lex_tokens(r#"gql#"query { user { name } }""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "gql")
    );

    let tokens = lex_tokens(r#"rx#"^[a-z]+$""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "rx")
    );

    let tokens = lex_tokens(r#"sql#"SELECT * FROM users""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "sql")
    );

    let tokens = lex_tokens(r#"url#"https://example.com""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "url")
    );

    let tokens = lex_tokens(r#"email#"user@example.com""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "email")
    );

    let tokens = lex_tokens(r#"json#"{"key": "value"}""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "json")
    );

    let tokens = lex_tokens(r#"xml#"<root><item/></root>""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "xml")
    );

    let tokens = lex_tokens(r#"yaml#"key: value""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::TaggedLiteral(data) if data.tag == "yaml")
    );
}

#[test]
fn test_contract_literals() {
    // Contract literals are compiler intrinsics for formal verification (NOT in @tagged_literal registry).
    // Grammar: contract_literal = 'contract' '#' (plain_string | raw_string)
    // Contains requires/ensures/invariant clauses for SMT verification.

    // Contract literals are TokenKind::ContractLiteral (NOT TaggedLiteral)
    // They are compiler intrinsics for formal verification
    let tokens = lex_tokens(r#"contract#"requires x > 0""#);
    assert!(matches!(&tokens[0].kind, TokenKind::ContractLiteral(_)));

    let tokens = lex_tokens(r#"contract#"ensures result >= 0""#);
    assert!(matches!(&tokens[0].kind, TokenKind::ContractLiteral(_)));

    let tokens = lex_tokens(r#"contract#"invariant total == arr[0..i].sum()""#);
    assert!(matches!(&tokens[0].kind, TokenKind::ContractLiteral(_)));
}

#[test]
fn test_safe_interpolated_strings() {
    // Safe interpolated strings: `prefix"text {expr} text"` with automatic escaping.
    // Grammar: interpolated_string = identifier '"' { string_char | interpolation } '"'
    // Prefixes: f (format), sql (injection-safe), html (XSS-safe), url (encoding), gql

    // SQL with safe interpolation
    let tokens = lex_tokens(r#"sql"SELECT * FROM users WHERE id = {user_id}""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::InterpolatedString(data) if data.prefix == "sql")
    );

    // HTML with auto-escaping
    let tokens = lex_tokens(r#"html"<h1>{title}</h1>""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::InterpolatedString(data) if data.prefix == "html")
    );

    // URL with safe encoding
    let tokens = lex_tokens(r#"url"https://api.example.com/users?name={user_name}""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::InterpolatedString(data) if data.prefix == "url")
    );

    // GraphQL with interpolation
    let tokens = lex_tokens(r#"gql"query { user(id: {user_id}) { name } }""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::InterpolatedString(data) if data.prefix == "gql")
    );

    // Format string
    let tokens = lex_tokens(r#"f"Hello {name}, you are {age} years old""#);
    assert!(
        matches!(&tokens[0].kind, TokenKind::InterpolatedString(data) if data.prefix == "f")
    );
}

#[test]
fn test_numeric_literals_with_suffixes() {
    // Numeric literals with optional unit suffixes for units of measure.
    // Grammar: integer_lit = (decimal_lit | hex_lit | bin_lit) ['_' identifier]
    // float_lit = decimal '.' decimal [exponent] ['_' identifier]

    let tokens = lex_tokens("100_km");
    assert!(
        matches!(&tokens[0].kind, TokenKind::Integer(lit) if lit.as_i64() == Some(100) && lit.suffix.as_deref() == Some("km"))
    );

    let tokens = lex_tokens("90_deg");
    assert!(
        matches!(&tokens[0].kind, TokenKind::Integer(lit) if lit.as_i64() == Some(90) && lit.suffix.as_deref() == Some("deg"))
    );

    let tokens = lex_tokens("20_C");
    assert!(
        matches!(&tokens[0].kind, TokenKind::Integer(lit) if lit.as_i64() == Some(20) && lit.suffix.as_deref() == Some("C"))
    );

    let tokens = lex_tokens("1024_MB");
    assert!(
        matches!(&tokens[0].kind, TokenKind::Integer(lit) if lit.as_i64() == Some(1024) && lit.suffix.as_deref() == Some("MB"))
    );

    let tokens = lex_tokens("3.14_rad");
    assert!(
        matches!(&tokens[0].kind, TokenKind::Float(lit) if (lit.value - 3.14).abs() < 0.001 && lit.suffix.as_deref() == Some("rad"))
    );
}

#[test]
fn test_hex_color_literals() {
    // Hex color literals: #RRGGBB or #RRGGBBAA (context-adaptive)
    // Grammar: hex_color_literal = '#' hex_digit{6} [hex_digit{2}]

    let tokens = lex_tokens("#FF5733");
    assert!(
        matches!(&tokens[0].kind, TokenKind::HexColor(color) if color == &Text::from("FF5733"))
    );

    let tokens = lex_tokens("#00FF00FF");
    assert!(
        matches!(&tokens[0].kind, TokenKind::HexColor(color) if color == &Text::from("00FF00FF"))
    );
}

#[test]
fn test_raw_and_multiline_strings() {
    // Raw/multiline strings: `"""..."""` preserves content literally (no escape processing).
    // Grammar: multiline_string = '"""' { char } '"""'

    // Raw multiline string preserves backslashes literally (no escape processing)
    let tokens = lex_tokens(r#""""raw\nstring""""#);
    assert!(matches!(&tokens[0].kind, TokenKind::Text(s) if s.contains(r"\n")));

    // Raw multiline can contain embedded quotes
    let tokens = lex_tokens(r#"""""nested "quotes" inside"""""#);
    assert!(matches!(&tokens[0].kind, TokenKind::Text(_)));

    // Raw multiline spans multiple lines
    let tokens = lex_tokens("\"\"\"line1\nline2\nline3\"\"\"");
    assert!(matches!(&tokens[0].kind, TokenKind::Text(s) if s.contains("\n")));
}

#[test]
fn test_operators_complete() {
    // Verum operators: arithmetic (+,-,*,/,%,**), comparison (==,!=,<,>,<=,>=),
    // logical (&&,||,!), bitwise (&,|,^,<<,>>,~), range (..,..,=),
    // pipeline (|>), optional chaining (?.,??), arrows (->,=>)

    // Arithmetic
    let ops = lex("+ - * / % **");
    assert!(matches!(ops[0], TokenKind::Plus));
    assert!(matches!(ops[1], TokenKind::Minus));
    assert!(matches!(ops[2], TokenKind::Star));
    assert!(matches!(ops[3], TokenKind::Slash));
    assert!(matches!(ops[4], TokenKind::Percent));
    assert!(matches!(ops[5], TokenKind::StarStar));

    // Comparison
    let ops = lex("== != < > <= >=");
    assert!(matches!(ops[0], TokenKind::EqEq));
    assert!(matches!(ops[1], TokenKind::BangEq));
    assert!(matches!(ops[2], TokenKind::Lt));
    assert!(matches!(ops[3], TokenKind::Gt));
    assert!(matches!(ops[4], TokenKind::LtEq));
    assert!(matches!(ops[5], TokenKind::GtEq));

    // Logical
    let ops = lex("&& || !");
    assert!(matches!(ops[0], TokenKind::AmpersandAmpersand));
    assert!(matches!(ops[1], TokenKind::PipePipe));
    assert!(matches!(ops[2], TokenKind::Bang));

    // Bitwise
    let ops = lex("& | ^ << >> ~");
    assert!(matches!(ops[0], TokenKind::Ampersand));
    assert!(matches!(ops[1], TokenKind::Pipe));
    assert!(matches!(ops[2], TokenKind::Caret));
    assert!(matches!(ops[3], TokenKind::LtLt));
    assert!(matches!(ops[4], TokenKind::GtGt));
    assert!(matches!(ops[5], TokenKind::Tilde));

    // Range and pipeline
    let ops = lex(".. ..= |>");
    assert!(matches!(ops[0], TokenKind::DotDot));
    assert!(matches!(ops[1], TokenKind::DotDotEq));
    assert!(matches!(ops[2], TokenKind::PipeGt));

    // Optional chaining and null coalescing
    let ops = lex("?. ?? ?");
    assert!(matches!(ops[0], TokenKind::QuestionDot));
    assert!(matches!(ops[1], TokenKind::QuestionQuestion));
    assert!(matches!(ops[2], TokenKind::Question));

    // Arrows
    let ops = lex("-> =>");
    assert!(matches!(ops[0], TokenKind::RArrow));
    assert!(matches!(ops[1], TokenKind::FatArrow));
}

#[test]
fn test_attribute_annotations() {
    // Attributes use `@` prefix: @derive(...), @verify, @repr(C), etc.
    // All compile-time constructs use @ prefix (no Rust-style `!` macros)

    let tokens = lex("@ derive");
    assert!(matches!(tokens[0], TokenKind::At));
    assert!(matches!(&tokens[1], TokenKind::Ident(name) if name == &Text::from("derive")));

    let tokens = lex("@ verify");
    assert!(matches!(tokens[0], TokenKind::At));
    assert!(matches!(&tokens[1], TokenKind::Ident(name) if name == &Text::from("verify")));

    let tokens = lex("@ inline");
    assert!(matches!(tokens[0], TokenKind::At));
    assert!(matches!(&tokens[1], TokenKind::Ident(name) if name == &Text::from("inline")));

    let tokens = lex("@ context");
    assert!(matches!(tokens[0], TokenKind::At));
    assert!(matches!(tokens[1], TokenKind::Context));
}

#[test]
fn test_complete_v6_program() {
    // Integration test with all v6.0-BALANCED features
    let source = r#"
        type Positive is Int{> 0};

        fn factorial(n: Positive) -> Int using Math {
            match n {
                1 => 1,
                n => n * factorial(n - 1)
            }
        }

        context Database {
            fn query(sql: Text) -> Result<Rows>
        }

        let pattern = rx#"^[a-z]+$";
        let query = sql"SELECT * FROM users WHERE id = {user_id}";
        let distance = 100_km;
    "#;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<Token> = lexer.filter_map(|r| r.ok()).collect();

    // Verify we can lex the entire program without errors
    assert!(tokens.len() > 50);

    // Check key tokens are present
    let kinds: Vec<&TokenKind> = tokens.iter().map(|t| &t.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Type)));
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Is)));
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Fn)));
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Using)));
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Context)));
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, TokenKind::TaggedLiteral(_)))
    );
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, TokenKind::InterpolatedString(_)))
    );
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::Integer(_))));
}
