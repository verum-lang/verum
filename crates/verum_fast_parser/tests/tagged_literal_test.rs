//! Test tagged literal parsing

use verum_ast::FileId;
use verum_fast_parser::FastParser;

fn parse_ok(name: &str, source: &str) {
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    match parser.parse_module_str(source, file_id) {
        Ok(_) => {}
        Err(errors) => {
            eprintln!("FAILED: {name}");
            for e in errors.iter() {
                eprintln!("  {:?}", e);
            }
            panic!("Parsing failed for {name}");
        }
    }
}

fn parse_fail(name: &str, source: &str) {
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    match parser.parse_module_str(source, file_id) {
        Ok(_) => panic!("{name}: expected parse failure but succeeded"),
        Err(_) => {}
    }
}

// =============================================================================
// VCS integration: parse the full tagged_literals.vr spec file
// =============================================================================

#[test]
fn test_vcs_tagged_literals_file() {
    let content = include_str!("../../../vcs/specs/parser/success/lexer/tagged_literals.vr");
    parse_ok("tagged_literals.vr", content);
}

// =============================================================================
// Byte string extensions: b64#, hex#, pct#
// =============================================================================

#[test]
fn test_b64_tagged_literal() {
    parse_ok("b64 single", r#"fn f() { let x = b64#"SGVsbG8="; }"#);
    parse_ok("b64 multiline", r#"fn f() { let x = b64#"""SGVsbG8gV29ybGQ="""; }"#);
}

#[test]
fn test_hex_tagged_literal() {
    parse_ok("hex single", r#"fn f() { let x = hex#"deadbeef"; }"#);
    parse_ok("hex multiline", r#"fn f() { let x = hex#"""CAFEBABE"""; }"#);
}

#[test]
fn test_pct_tagged_literal() {
    parse_ok("pct single", r#"fn f() { let x = pct#"Hello%20World"; }"#);
}

// =============================================================================
// Tagged literal delimiters
// =============================================================================

#[test]
fn test_tagged_triple_quote() {
    parse_ok("json triple-quote", r#"fn f() { let x = json#"""{"key": "value"}"""; }"#);
    parse_ok("rx triple-quote", r#"fn f() { let x = rx#"""[a-z]+\d+"""; }"#);
    parse_ok("path triple-quote", r#"fn f() { let x = path#"""C:\Users\Name"""; }"#);
}

#[test]
fn test_tagged_multiline_content() {
    let source = r#"fn f() {
    let x = sql#"""
        SELECT *
        FROM users
        WHERE id = 1
    """;
}"#;
    parse_ok("sql multiline", source);
}

// =============================================================================
// Various tag categories
// =============================================================================

#[test]
fn test_data_format_tags() {
    parse_ok("json", r#"fn f() { let x = json#"{}"; }"#);
    parse_ok("yaml", r#"fn f() { let x = yaml#"key: value"; }"#);
    parse_ok("toml", r#"fn f() { let x = toml#"[section]"; }"#);
    parse_ok("xml", r#"fn f() { let x = xml#"<root/>"; }"#);
}

#[test]
fn test_query_tags() {
    parse_ok("sql", r#"fn f() { let x = sql#"SELECT 1"; }"#);
    parse_ok("gql", r#"fn f() { let x = gql#"{ user { id } }"; }"#);
}

#[test]
fn test_pattern_tags() {
    parse_ok("rx", r#"fn f() { let x = rx#"[a-z]+"; }"#);
    parse_ok("re", r#"fn f() { let x = re#"^\d+$"; }"#);
}

#[test]
fn test_network_tags() {
    parse_ok("url", r#"fn f() { let x = url#"https://example.com"; }"#);
    parse_ok("uri", r#"fn f() { let x = uri#"urn:isbn:123"; }"#);
    parse_ok("email", r#"fn f() { let x = email#"user@example.com"; }"#);
    parse_ok("ip", r#"fn f() { let x = ip#"192.168.1.1"; }"#);
    parse_ok("cidr", r#"fn f() { let x = cidr#"10.0.0.0/8"; }"#);
    parse_ok("mac", r#"fn f() { let x = mac#"00:11:22:33:44:55"; }"#);
}

#[test]
fn test_time_tags() {
    parse_ok("d date", r#"fn f() { let x = d#"2024-03-15"; }"#);
    parse_ok("dur", r#"fn f() { let x = dur#"1h30m"; }"#);
    parse_ok("tz", r#"fn f() { let x = tz#"America/New_York"; }"#);
}

#[test]
fn test_version_tags() {
    parse_ok("ver", r#"fn f() { let x = ver#"1.2.3"; }"#);
}

#[test]
fn test_structured_tags() {
    parse_ok("mat", r#"fn f() { let x = mat#"[[1,2],[3,4]]"; }"#);
    parse_ok("vec", r#"fn f() { let x = vec#"[1, 2, 3]"; }"#);
    parse_ok("interval", r#"fn f() { let x = interval#"[0,100]"; }"#);
    parse_ok("ratio", r#"fn f() { let x = ratio#"3/4"; }"#);
}

#[test]
fn test_code_tags() {
    parse_ok("sh", r#"fn f() { let x = sh#"echo hello"; }"#);
    parse_ok("c", r#"fn f() { let x = c#"int x = 0;"; }"#);
}

// =============================================================================
// Invalid syntax - parse failures
// =============================================================================

#[test]
fn test_vcs_invalid_tagged_literal() {
    let content = include_str!("../../../vcs/specs/parser/fail/lexer/invalid_tagged_literal.vr");
    parse_fail("invalid_tagged_literal.vr", content);
}

#[test]
fn test_vcs_unterminated_triple_quoted() {
    let content = include_str!("../../../vcs/specs/parser/fail/lexer/unterminated_triple_quoted.vr");
    parse_fail("unterminated_triple_quoted.vr", content);
}

// =============================================================================
// Original tests
// =============================================================================

#[test]
fn test_tagged_literal_simple() {
    parse_ok(
        "simple json tagged",
        r#"fn test() { let a = json#"{\"key\": \"value\"}"; }"#,
    );
}

#[test]
fn test_tagged_literal_in_function() {
    parse_ok(
        "json tagged in function",
        r#"fn test() {
    let a = json#"{\"key\": \"value\"}";
}"#,
    );
}
