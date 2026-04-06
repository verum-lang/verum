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
//! Pattern matching tests focused on complex patterns, guards, or-patterns,
//! nested patterns, and binding patterns.
//!
//! These complement the existing pattern_matching_tests.rs and
//! pattern_comprehensive_tests.rs with more complex scenarios.

use verum_ast::{FileId, Module};
use verum_lexer::Lexer;
use verum_fast_parser::VerumParser;

fn parse_module(source: &str) -> Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("Failed to parse module: {:?}", e))
}

fn assert_parses(source: &str) {
    parse_module(source);
}

fn assert_parse_fails(source: &str) {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    assert!(
        parser.parse_module(lexer, file_id).is_err(),
        "Expected parse failure for: {}",
        source
    );
}

// ============================================================================
// OR-PATTERNS
// ============================================================================

#[test]
fn test_or_pattern_in_match() {
    let source = r#"
        fn classify(x: Int) -> Text {
            match x {
                0 | 1 => "small",
                2 | 3 | 4 => "medium",
                _ => "large",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_or_pattern_with_variants() {
    let source = r#"
        fn is_terminal(state: State) -> Bool {
            match state {
                State.Success | State.Failure => true,
                State.Pending | State.Running => false,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_or_pattern_in_let() {
    let source = r#"
        fn main() {
            let (0 | 1) = get_value();
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// GUARD EXPRESSIONS
// ============================================================================

#[test]
fn test_match_with_guard() {
    let source = r#"
        fn classify(x: Int) -> Text {
            match x {
                n if n < 0 => "negative",
                0 => "zero",
                n if n > 100 => "large",
                n => "normal",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_guard_with_complex_expression() {
    let source = r#"
        fn process(item: Item) -> Bool {
            match item {
                Item.Value(x) if x > 0 && x < 100 => true,
                Item.Value(x) if x == 0 => false,
                _ => false,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_guard_with_function_call() {
    let source = r#"
        fn filter(event: Event) -> Bool {
            match event {
                Event.Message(msg) if is_valid(msg) => true,
                _ => false,
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// NESTED PATTERNS
// ============================================================================

#[test]
fn test_nested_tuple_patterns() {
    let source = r#"
        fn main() {
            match point {
                ((0, 0), (0, 0)) => "origin pair",
                ((x1, y1), (x2, y2)) => "two points",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nested_variant_patterns() {
    let source = r#"
        fn extract(opt: Maybe<Maybe<Int>>) -> Int {
            match opt {
                Some(Some(x)) => x,
                Some(None) => 0,
                None => -1,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nested_record_patterns() {
    let source = r#"
        fn main() {
            match config {
                Config { db: DbConfig { host, port }, .. } => connect(host, port),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_variant_with_record_pattern() {
    let source = r#"
        fn describe(shape: Shape) -> Text {
            match shape {
                Shape.Circle { radius } if radius > 0.0 => "valid circle",
                Shape.Rect { width, height } => f"rect {width}x{height}",
                _ => "unknown",
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// BINDING PATTERNS (@ patterns)
// ============================================================================

#[test]
fn test_binding_pattern_basic() {
    let source = r#"
        fn main() {
            match value {
                x @ 1 => use_one(x),
                x @ 2 => use_two(x),
                x => use_other(x),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_binding_pattern_with_variant() {
    let source = r#"
        fn main() {
            match opt {
                all @ Some(inner) => process(all, inner),
                None => default_value(),
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// REST PATTERNS (..)
// ============================================================================

#[test]
fn test_rest_pattern_in_tuple() {
    let source = r#"
        fn main() {
            let (first, ..) = get_tuple();
            let (.., last) = get_tuple();
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_rest_pattern_in_array() {
    let source = r#"
        fn main() {
            match arr {
                [first, ..rest] => process(first, rest),
                [] => handle_empty(),
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_rest_pattern_in_record() {
    let source = r#"
        fn main() {
            let Config { host, port, .. } = load_config();
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// WILDCARD AND CATCH-ALL PATTERNS
// ============================================================================

#[test]
fn test_wildcard_in_various_positions() {
    let source = r#"
        fn main() {
            match pair {
                (_, 0) => "y is zero",
                (0, _) => "x is zero",
                (_, _) => "neither zero",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_wildcard_in_variant() {
    let source = r#"
        fn has_value(opt: Maybe<Int>) -> Bool {
            match opt {
                Some(_) => true,
                None => false,
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// RANGE PATTERNS
// ============================================================================

#[test]
fn test_range_pattern_in_match() {
    let source = r#"
        fn classify_char(c: Char) -> Text {
            match c {
                'a'..'z' => "lowercase",
                'A'..'Z' => "uppercase",
                '0'..'9' => "digit",
                _ => "other",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_inclusive_range_pattern() {
    let source = r#"
        fn classify_score(score: Int) -> Text {
            match score {
                0..=59 => "F",
                60..=69 => "D",
                70..=79 => "C",
                80..=89 => "B",
                90..=100 => "A",
                _ => "invalid",
            }
        }
    "#;
    assert_parses(source);
}

// ============================================================================
// COMPLEX MATCH EXPRESSIONS
// ============================================================================

#[test]
fn test_match_with_multiple_patterns_and_guards() {
    let source = r#"
        fn process(event: Event) -> Action {
            match event {
                Event.Click(pos) if pos.x > 0 => Action.Handle(pos),
                Event.Key(key) if key == "Enter" => Action.Submit,
                Event.Timer(ms) if ms > 1000 => Action.Timeout,
                _ => Action.Ignore,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_match_returning_different_types_with_blocks() {
    let source = r#"
        fn dispatch(cmd: Command) -> Result<Text, Error> {
            match cmd {
                Command.Run { script, args } => {
                    let output = execute(script, args);
                    Ok(output)
                }
                Command.Help => {
                    Ok("Usage: run <script> [args...]")
                }
                Command.Version => {
                    Ok(f"v{VERSION}")
                }
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_match_on_tuple() {
    let source = r#"
        fn compare(a: Int, b: Int) -> Ordering {
            match (a, b) {
                (x, y) if x < y => Ordering.Less,
                (x, y) if x > y => Ordering.Greater,
                _ => Ordering.Equal,
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_match_on_bool() {
    let source = r#"
        fn to_text(b: Bool) -> Text {
            match b {
                true => "yes",
                false => "no",
            }
        }
    "#;
    assert_parses(source);
}

#[test]
fn test_nested_match_expressions() {
    let source = r#"
        fn deep_match(x: Maybe<Result<Int, Error>>) -> Int {
            match x {
                Some(Ok(n)) => n,
                Some(Err(_)) => -1,
                None => 0,
            }
        }
    "#;
    assert_parses(source);
}
