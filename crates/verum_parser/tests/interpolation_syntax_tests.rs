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
// Tests for interpolation string syntax (sh"", rx"", format strings).
//
// Tests for interpolation syntax: f"...", sql"...", html"..." tagged interpolation
// Interpolation handlers provide safe, domain-specific string interpolation with compile-time validation.

use verum_ast::{Expr, ExprKind, FileId, Item, ItemKind};
use verum_common::List;
use verum_lexer::Lexer;
use verum_parser::{ParseError, VerumParser};
use verum_common::Text;

/// Helper to parse a module from source.
fn parse(source: &str) -> Result<List<Item>, List<ParseError>> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).map(|m| m.items)
}

// ============================================================================
// Shell String Interpolation (sh"...") Tests
// ============================================================================

#[test]
fn test_shell_string_literal_parsing() {
    // Basic shell string parsing
    let source = r#"
        fn test() {
            let cmd = sh"echo hello";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_shell_string_with_single_interpolation() {
    // Shell string with single interpolation
    let source = r#"
        fn test() {
            let name = "World";
            let cmd = sh"echo Hello, {name}!";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_shell_string_with_multiple_interpolations() {
    // Shell string with multiple interpolations
    let source = r#"
        fn test() {
            let user = "admin";
            let file = "data.txt";
            let cmd = sh"sudo -u {user} cat {file}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_shell_string_with_special_chars() {
    // Shell strings preserve special shell characters
    let source = r#"
        fn test() {
            let cmd = sh"grep -E '[0-9]+' file.txt | sort";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_shell_string_with_escaped_braces() {
    // Double braces escape to single brace
    let source = r#"
        fn test() {
            let cmd = sh"echo {{literal}} {var}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_shell_string_multiline() {
    // Multi-line shell scripts
    let source = r#"
        fn test() {
            let script = sh"
                #!/bin/bash
                echo 'Start'
                ls -la
                echo 'Done'
            ";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Regex String Interpolation (rx"...") Tests
// ============================================================================

#[test]
fn test_regex_string_literal_parsing() {
    // Basic regex string parsing
    let source = r#"
        fn test() {
            let regex_pattern = rx"[a-z]+";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_regex_string_with_flags() {
    // Regex with flags
    let source = r#"
        fn test() {
            let case_insensitive = rx"(?i)[a-z]+";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_regex_string_with_named_groups() {
    // Regex with named capture groups
    let source = r#"
        fn test() {
            let email_pattern = rx"(?P<user>[a-z]+)@(?P<domain>[a-z.]+)";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_regex_string_with_interpolation() {
    // Regex with interpolated patterns
    let source = r#"
        fn test() {
            let domain = "example.com";
            let regex_pattern = rx"\w+@{domain}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_regex_string_complex_pattern() {
    // Complex regex patterns
    let source = r#"
        fn test() {
            let url_pattern = rx"https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&/=]*)";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// SQL String Interpolation (sql"...") Tests
// ============================================================================

#[test]
fn test_sql_string_literal_parsing() {
    // Basic SQL string
    let source = r#"
        fn test() {
            let query = sql"SELECT * FROM users";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_sql_string_with_parameters() {
    // SQL with parameterized queries
    let source = r#"
        fn test() {
            let user_id = 42;
            let query = sql"SELECT * FROM users WHERE id = {user_id}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_sql_string_with_multiple_parameters() {
    // SQL with multiple parameters
    let source = r#"
        fn test() {
            let min_age = 18;
            let max_age = 65;
            let query = sql"SELECT * FROM users WHERE age BETWEEN {min_age} AND {max_age}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_sql_string_multiline() {
    // Multi-line SQL queries
    let source = r#"
        fn test() {
            let query = sql"
                SELECT u.id, u.name, COUNT(*) as order_count
                FROM users u
                JOIN orders o ON u.id = o.user_id
                GROUP BY u.id, u.name
                HAVING COUNT(*) > {min_orders}
            ";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Format String Tests (standard f"..." syntax)
// ============================================================================

#[test]
fn test_format_string_simple() {
    // Basic format strings
    let source = r#"
        fn test() {
            let name = "Alice";
            let msg = f"Hello, {name}!";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_format_string_multiple_interpolations() {
    // Format with multiple interpolations
    let source = r#"
        fn test() {
            let x = 5;
            let y = 10;
            let msg = f"x={x}, y={y}, sum={x + y}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_format_string_with_expressions() {
    // Format with complex expressions
    let source = r#"
        fn test() {
            let nums = List.from([1, 2, 3]);
            let msg = f"Numbers: {nums.join(\", \")}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Interpolation Handler Integration Tests
// ============================================================================

#[test]
fn test_interpolation_handler_registration() {
    // Test that handlers are properly registered
    let source = r#"
        @interpolation_handler("custom")
        type CustomInterpolation is protocol {
            fn handle(template: Text, args: List<Expr>) -> CustomResult;
        };
    "#;

    // This would be for documentation - actual validation happens in type checker
    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_custom_interpolation_string() {
    // Custom interpolation handlers
    let source = r#"
        fn test() {
            let html = html"<div>{content}</div>";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Interpolation Edge Cases
// ============================================================================

#[test]
fn test_nested_braces_in_expression() {
    // Nested braces in interpolated expressions
    let source = r#"
        fn test() {
            let data = Map.from([("key", "value")]);
            let msg = f"Data: {data.get(\"key\").unwrap_or(\"default\")}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_interpolation_with_string_literals() {
    // String literals within interpolations
    let source = r#"
        fn test() {
            let name = "world";
            let msg = f"Hello, {\"dear \" + name}!";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_empty_interpolation_string() {
    // Empty interpolation strings
    let source = r#"
        fn test() {
            let empty = sh"";
            let empty_sql = sql"";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Interpolation String Escaping Tests
// ============================================================================

#[test]
fn test_escaped_quote_in_interpolation() {
    // Escaped quotes in interpolation strings
    let source = r#"
        fn test() {
            let query = sql"SELECT * FROM users WHERE name = 'O\'Brien'";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_multiple_escape_sequences() {
    // Various escape sequences
    let source = r#"
        fn test() {
            let script = sh"echo 'Line 1\nLine 2'";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

// ============================================================================
// Interpolation Syntax Compliance Tests
// ============================================================================

#[test]
fn test_sh_string_prefix_recognized() {
    // sh prefix specifically recognized
    let source = "fn test() { let cmd = sh\"ls -la\"; }";
    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "sh prefix should be recognized");
}

#[test]
fn test_rx_string_prefix_recognized() {
    // rx prefix specifically recognized
    let source = "fn test() { let regex = rx\"\\d+\"; }";
    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "rx prefix should be recognized");
}

#[test]
fn test_f_string_prefix_recognized() {
    // f prefix for format strings
    let source = "fn test() { let msg = f\"Value: {x}\"; }";
    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "f prefix should be recognized");
}

// ============================================================================
// Interpolation in Different Contexts
// ============================================================================

#[test]
fn test_interpolation_in_function_argument() {
    // Interpolation strings as function arguments
    let source = r#"
        fn process_command(cmd: Text) { }

        fn main() {
            process_command(sh"ls -la");
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert_eq!(items.len(), 2, "Should parse function and usage");
}

#[test]
fn test_interpolation_in_return_statement() {
    // Interpolation in return values
    let source = r#"
        fn build_query(id: Int) -> Text {
            return sql"SELECT * FROM users WHERE id = {id}";
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}

#[test]
fn test_interpolation_in_match_pattern() {
    // Interpolation in complex expressions
    let source = r#"
        fn test() {
            let result = if user_id > 0 {
                sql"SELECT * FROM users WHERE id = {user_id}"
            } else {
                sql"SELECT * FROM users LIMIT 10"
            };
        }
    "#;

    let items = parse(source).expect("parsing failed");
    assert!(!items.is_empty(), "Should parse successfully");
}
