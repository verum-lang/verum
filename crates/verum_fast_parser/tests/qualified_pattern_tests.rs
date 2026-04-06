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
// Integration tests for qualified path patterns
use verum_ast::{FileId, Item};
use verum_lexer::Lexer;
use verum_fast_parser::RecursiveParser;

fn parse_source(source: &str) -> Result<Vec<Item>, verum_fast_parser::error::ParseError> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let tokens: Vec<_> = lexer.filter_map(|r| r.ok()).collect();
    let mut parser = RecursiveParser::new(&tokens, file_id);
    parser.parse_module()
}

#[test]
fn test_maybe_qualified_patterns() {
    let source = r#"
        fn test_maybe(x: Maybe<Int>) {
            match x {
                Maybe.Some(value) => value,
                Maybe.None => 0,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse Maybe patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_result_qualified_patterns() {
    let source = r#"
        fn test_result(r: Result<Int, Text>) {
            match r {
                Result.Ok(value) => value,
                Result.Err(e) => 0,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse Result patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_nested_qualified_patterns() {
    let source = r#"
        fn test_nested(x: Maybe<(Int, Int)>) {
            match x {
                Maybe.Some((a, b)) => a + b,
                Maybe.None => 0,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse nested patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_qualified_record_patterns() {
    let source = r#"
        type Event is
            | UserCreated { id: Int, name: Text }
            | UserDeleted { id: Int };

        fn test_events(event: Event) {
            match event {
                Event.UserCreated { id, name } => println("User {} created", name),
                Event.UserDeleted { id } => println("User deleted"),
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse record patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_long_qualified_paths() {
    let source = r#"
        fn test_long_path(x: std.option.Maybe<Int>) {
            match x {
                std.option.Maybe.Some(value) => value,
                std.option.Maybe.None => 0,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse long qualified paths: {:?}",
        result.err()
    );
}

#[test]
fn test_qualified_unit_variants() {
    let source = r#"
        type Color is
            | Red
            | Green
            | Blue;

        fn test_color(c: Color) {
            match c {
                Color.Red => 1,
                Color.Green => 2,
                Color.Blue => 3,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse unit variant patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_mixed_qualified_and_simple_patterns() {
    let source = r#"
        fn test_mixed(x: Maybe<Int>, y: Int) {
            match (x, y) {
                (Maybe.Some(value), n) => value + n,
                (Maybe.None, n) => n,
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse mixed patterns: {:?}",
        result.err()
    );
}

#[test]
fn test_or_patterns_with_qualified_paths() {
    let source = r#"
        fn test_or(x: Maybe<Int>) {
            match x {
                Maybe.Some(1) | Maybe.Some(2) | Maybe.Some(3) => println("Small"),
                Maybe.Some(n) => println("Large: {}", n),
                Maybe.None => println("None"),
            }
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse OR patterns with qualified paths: {:?}",
        result.err()
    );
}

#[test]
fn test_ref_mut_patterns_still_work() {
    let source = r#"
        fn test_ref_mut(x: &Int) {
            let ref y = x;
            let mut z = 10;
            let ref mut w = 20;
        }
    "#;

    let result = parse_source(source);
    assert!(
        result.is_ok(),
        "Failed to parse ref/mut patterns: {:?}",
        result.err()
    );
}
