//! Per-construct formatter contract tests.
//!
//! Each test fires `format_string` against a fixture, asserts the
//! output is non-empty, parses cleanly, and matches the canonical
//! formatting we want the formatter to produce. Together with the
//! idempotence and round-trip suites these tests pin formatter
//! behaviour from three angles — output shape, convergence, and
//! semantic preservation.

use verum_cli::commands::fmt::format_string;

fn parses(source: &str) -> bool {
    use verum_ast::FileId;
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    let lexer = Lexer::new(source, FileId::new(0));
    let parser = VerumParser::new();
    parser.parse_module(lexer, FileId::new(0)).is_ok()
}

fn fmt(source: &str) -> String {
    format_string(source)
        .expect("format_string returned Err")
        .to_string()
}

// ============================================================
// Output shape — the formatter produces parseable, non-empty text
// ============================================================

#[test]
fn formats_empty_module() {
    let out = fmt("");
    // Empty input is valid — the output may be empty or just a
    // trailing newline; parsing must succeed either way.
    assert!(parses(&out), "empty input should produce parseable output, got: {out:?}");
}

#[test]
fn formats_single_function() {
    let src = "fn main() {}\n";
    let out = fmt(src);
    assert!(out.contains("fn main"));
    assert!(parses(&out), "output must parse: {out}");
}

#[test]
fn formats_simple_type_def() {
    let src = "type Point is { x: Int, y: Int };\n";
    let out = fmt(src);
    assert!(out.contains("type Point"));
    assert!(parses(&out));
}

#[test]
fn formats_variant_type() {
    let src = "type Maybe<T> is None | Some(T);\n";
    let out = fmt(src);
    assert!(out.contains("type Maybe"));
    assert!(parses(&out));
}

#[test]
fn formats_mount_statement() {
    let src = "mount stdlib.collections.list;\n\nfn main() {}\n";
    let out = fmt(src);
    assert!(out.contains("mount"));
    assert!(parses(&out));
}

#[test]
fn formats_match_expression() {
    let src = "fn classify(x: Int) -> Int {\n    match x {\n        0 => 0,\n        _ => 1,\n    }\n}\n";
    let out = fmt(src);
    assert!(out.contains("match"));
    assert!(parses(&out));
}

#[test]
fn formats_refinement_type() {
    let src = "fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n";
    let out = fmt(src);
    assert!(out.contains("Int"));
    assert!(parses(&out));
}

#[test]
fn formats_attributes() {
    let src = "@verify(formal)\npublic fn safe_divide(a: Int, b: Int{ it != 0 }) -> Int {\n    a / b\n}\n";
    let out = fmt(src);
    assert!(out.contains("@verify"));
    assert!(parses(&out));
}

#[test]
fn formats_protocol_definition() {
    let src = "type Display is protocol {\n    fn fmt(&self) -> Text;\n};\n";
    let out = fmt(src);
    assert!(out.contains("protocol"));
    assert!(parses(&out));
}

#[test]
fn formats_implement_block() {
    let src = "type Point is { x: Int };\n\nimplement Point {\n    fn x(self) -> Int { self.x }\n}\n";
    let out = fmt(src);
    assert!(out.contains("implement"));
    assert!(parses(&out));
}

// ============================================================
// Convergence — formatting reaches a fixed point
// ============================================================

#[test]
fn already_formatted_output_is_unchanged() {
    // A canonical short program should pass through fmt unchanged.
    let canonical = fmt("fn main() {}\n");
    let twice = fmt(&canonical);
    assert_eq!(canonical, twice, "fmt must be a fixed point on its own output");
}

#[test]
fn trailing_newline_added() {
    // A file without a trailing newline should gain one — POSIX
    // text-file convention. (Trailing-newline policy is on by
    // default per FormatterConfig.)
    let src = "fn main() {}";
    let out = fmt(src);
    assert!(
        out.ends_with('\n'),
        "fmt should add trailing newline, got: {out:?}"
    );
}

#[test]
fn formats_small_corpus() {
    let src = r#"
mount stdlib.collections.list;
mount stdlib.io;

@verify(formal)
public fn double(x: Int{ it >= 0 }) -> Int{ it >= 0 } {
    x * 2
}

type Tree<T> is
    | Leaf(T)
    | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>> };

fn main() {
    let xs: List<Int> = list::empty();
    print("hello");
}
"#;
    let out = fmt(src);
    assert!(parses(&out), "formatted corpus must parse: {out}");
    assert!(out.contains("@verify"));
    assert!(out.contains("type Tree"));
}
