//! Tests for tagged literal type inference
//!
//! Enhanced Tagged Literals
//! Verifies that format-specific type inference works correctly

use verum_parser::Parser;
use verum_types::infer::{InferMode, TypeChecker};

/// Helper to create a type checker
fn create_checker() -> TypeChecker {
    TypeChecker::new()
}

/// Helper to parse an expression and check its inferred type
fn infer_type(code: &str) -> String {
    let mut parser = Parser::new(code);
    let expr = parser.parse_expr().expect("Failed to parse");

    let mut checker = create_checker();
    let result = checker.infer(&expr, InferMode::Synth).expect("Failed to infer type");
    format!("{}", result.ty)
}

#[test]
fn test_json_tagged_literal_type() {
    let ty = infer_type(r#"json#"{ \"name\": \"Alice\" }""#);
    assert!(
        ty.contains("JsonValue") || ty.contains("Json"),
        "Expected JsonValue type, got: {}",
        ty
    );
}

#[test]
fn test_yaml_tagged_literal_type() {
    let ty = infer_type(r#"yaml#"name: Alice""#);
    assert!(
        ty.contains("YamlValue") || ty.contains("Yaml"),
        "Expected YamlValue type, got: {}",
        ty
    );
}

#[test]
fn test_sql_tagged_literal_type() {
    let ty = infer_type(r#"sql#"SELECT * FROM users""#);
    assert!(
        ty.contains("SqlQuery") || ty.contains("Sql"),
        "Expected SqlQuery type, got: {}",
        ty
    );
}

#[test]
fn test_regex_tagged_literal_type() {
    let ty = infer_type(r#"rx#"[a-zA-Z]+""#);
    assert!(ty.contains("Regex"), "Expected Regex type, got: {}", ty);
}

#[test]
fn test_url_tagged_literal_type() {
    let ty = infer_type(r#"url#"https://example.com""#);
    // Note: url# returns Uri type (URI is the general term, URL is a subset)
    assert!(
        ty.contains("Uri") || ty.contains("Url"),
        "Expected Uri/Url type, got: {}",
        ty
    );
}

#[test]
fn test_datetime_tagged_literal_type() {
    let ty = infer_type(r#"d#"2025-01-04""#);
    assert!(
        ty.contains("DateTime") || ty.contains("Date"),
        "Expected DateTime type, got: {}",
        ty
    );
}

#[test]
fn test_duration_tagged_literal_type() {
    let ty = infer_type(r#"dur#"3h30m""#);
    assert!(
        ty.contains("Duration"),
        "Expected Duration type, got: {}",
        ty
    );
}

#[test]
fn test_ip_tagged_literal_type() {
    let ty = infer_type(r#"ip#"192.168.1.1""#);
    assert!(
        ty.contains("IpAddr") || ty.contains("Ip"),
        "Expected IpAddr type, got: {}",
        ty
    );
}

#[test]
fn test_unknown_tag_fallback_to_text() {
    let ty = infer_type(r#"unknown#"some content""#);
    assert!(
        ty.contains("Text") || ty == "Text",
        "Expected Text type for unknown tag, got: {}",
        ty
    );
}

#[test]
fn test_multiline_json_type() {
    let ty = infer_type(
        r#"json#"""
{
    "name": "Alice",
    "age": 30
}
""""#,
    );
    assert!(
        ty.contains("JsonValue") || ty.contains("Json"),
        "Expected JsonValue type for multiline json, got: {}",
        ty
    );
}

#[test]
fn test_graphql_tagged_literal_type() {
    let ty = infer_type(r#"gql#"{ user { name } }""#);
    assert!(
        ty.contains("GraphQL") || ty.contains("Gql"),
        "Expected GraphQLQuery type, got: {}",
        ty
    );
}

#[test]
fn test_toml_tagged_literal_type() {
    let ty = infer_type(r#"toml#"[server]\nport = 8080""#);
    assert!(
        ty.contains("TomlValue") || ty.contains("Toml"),
        "Expected TomlValue type, got: {}",
        ty
    );
}

#[test]
fn test_xml_tagged_literal_type() {
    let ty = infer_type(r#"xml#"<root><item>value</item></root>""#);
    assert!(
        ty.contains("XmlDocument") || ty.contains("Xml"),
        "Expected XmlDocument type, got: {}",
        ty
    );
}

#[test]
fn test_email_tagged_literal_type() {
    let ty = infer_type(r#"email#"user@example.com""#);
    assert!(ty.contains("Email"), "Expected Email type, got: {}", ty);
}

#[test]
fn test_uuid_tagged_literal_type() {
    let ty = infer_type(r#"uuid#"550e8400-e29b-41d4-a716-446655440000""#);
    assert!(ty.contains("Uuid"), "Expected Uuid type, got: {}", ty);
}

// ============================================================================
// Extended format tag tests
// ============================================================================

#[test]
fn test_cidr_tagged_literal_type() {
    let ty = infer_type(r#"cidr#"192.168.0.0/16""#);
    assert!(
        ty.contains("CidrRange") || ty.contains("Cidr"),
        "Expected CidrRange type, got: {}",
        ty
    );
}

#[test]
fn test_mac_tagged_literal_type() {
    let ty = infer_type(r#"mac#"AA:BB:CC:DD:EE:FF""#);
    assert!(
        ty.contains("MacAddr") || ty.contains("Mac"),
        "Expected MacAddr type, got: {}",
        ty
    );
}

#[test]
fn test_glob_tagged_literal_type() {
    let ty = infer_type(r#"glob#"*.txt""#);
    assert!(
        ty.contains("GlobPattern") || ty.contains("Glob"),
        "Expected GlobPattern type, got: {}",
        ty
    );
}

#[test]
fn test_xpath_tagged_literal_type() {
    let ty = infer_type(r#"xpath#"//book/title""#);
    assert!(
        ty.contains("XPathExpr") || ty.contains("XPath"),
        "Expected XPathExpr type, got: {}",
        ty
    );
}

#[test]
fn test_path_tagged_literal_type() {
    let ty = infer_type(r#"path#"/usr/local/bin""#);
    assert!(
        ty.contains("PathBuf") || ty.contains("Path"),
        "Expected PathBuf type, got: {}",
        ty
    );
}

#[test]
fn test_timezone_tagged_literal_type() {
    let ty = infer_type(r#"tz#"America/New_York""#);
    assert!(
        ty.contains("Timezone") || ty.contains("Tz"),
        "Expected Timezone type, got: {}",
        ty
    );
}

#[test]
fn test_semver_tagged_literal_type() {
    let ty = infer_type(r#"semver#"1.2.3-beta.1""#);
    assert!(
        ty.contains("Version") || ty.contains("Semver"),
        "Expected Version type, got: {}",
        ty
    );
}

#[test]
fn test_base64_tagged_literal_type() {
    let ty = infer_type(r#"b64#"SGVsbG8gV29ybGQ=""#);
    assert!(
        ty.contains("Base64") || ty.contains("B64"),
        "Expected Base64 type, got: {}",
        ty
    );
}

#[test]
fn test_shell_tagged_literal_type() {
    let ty = infer_type(r#"sh#"echo hello world""#);
    assert!(
        ty.contains("ShellCommand") || ty.contains("Shell"),
        "Expected ShellCommand type, got: {}",
        ty
    );
}

#[test]
fn test_geo_tagged_literal_type() {
    let ty = infer_type(r#"geo#"40.7128,-74.0060""#);
    assert!(
        ty.contains("GeoCoord") || ty.contains("Geo"),
        "Expected GeoCoord type, got: {}",
        ty
    );
}

#[test]
fn test_cypher_tagged_literal_type() {
    let ty = infer_type(r#"cypher#"MATCH (n) RETURN n""#);
    assert!(
        ty.contains("CypherQuery") || ty.contains("Cypher"),
        "Expected CypherQuery type, got: {}",
        ty
    );
}

#[test]
fn test_csv_tagged_literal_type() {
    let ty = infer_type(r#"csv#"a,b,c""#);
    assert!(
        ty.contains("CsvData") || ty.contains("Csv"),
        "Expected CsvData type, got: {}",
        ty
    );
}

#[test]
fn test_alternative_regex_syntax() {
    // All regex variants should return Regex type
    let ty1 = infer_type(r#"rx#"[a-z]+""#);
    let ty2 = infer_type(r#"re#"[a-z]+""#);
    let ty3 = infer_type(r#"regex#"[a-z]+""#);

    assert!(ty1.contains("Regex"), "rx# should return Regex, got: {}", ty1);
    assert!(ty2.contains("Regex"), "re# should return Regex, got: {}", ty2);
    assert!(
        ty3.contains("Regex"),
        "regex# should return Regex, got: {}",
        ty3
    );
}
