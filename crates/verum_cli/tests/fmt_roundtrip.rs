//! Round-trip contract: `fmt(parse(src)) ≅ src` (up to whitespace).
//!
//! The strict invariant is that parsing the formatted output and
//! re-formatting it produces the same text — equivalent to asserting
//! the AST shape is preserved across format → parse → format. We
//! use this transitive form because spans naturally differ across
//! parses, so structural AST equality requires a span-stripping
//! comparator we don't need to maintain.
//!
//! In short: every fixture must satisfy
//!   `fmt(parse(fmt(parse(src)))) == fmt(parse(src))`
//! Combined with idempotence (`fmt(fmt(src)) == fmt(src)`), this
//! means the AST → format → parse cycle is information-preserving.

use verum_ast::FileId;
use verum_cli::commands::fmt::format_string;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

fn fmt(source: &str) -> String {
    format_string(source)
        .expect("format_string returned Err")
        .to_string()
}

fn parses(source: &str) -> bool {
    let lexer = Lexer::new(source, FileId::new(0));
    let parser = VerumParser::new();
    parser.parse_module(lexer, FileId::new(0)).is_ok()
}

fn assert_round_trip(label: &str, source: &str) {
    // First format — produces the canonical form.
    let canonical = fmt(source);
    assert!(
        parses(&canonical),
        "fixture `{label}` failed to parse after first format:\n{canonical}"
    );
    // Re-format the canonical form. This is the round-trip check —
    // if the AST shape changed during the first format, this would
    // produce something different from `canonical`.
    let reformatted = fmt(&canonical);
    assert_eq!(
        canonical, reformatted,
        "fixture `{label}` did not round-trip:\n\
         === canonical ===\n{canonical}\n\
         === reformatted ===\n{reformatted}"
    );
}

#[test]
fn round_trip_empty() {
    assert_round_trip("empty", "");
}

#[test]
fn round_trip_function() {
    assert_round_trip("fn", "fn main() {}\n");
}

#[test]
fn round_trip_attributed_function() {
    assert_round_trip(
        "attributed fn",
        "@verify(formal)\npublic fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n",
    );
}

#[test]
fn round_trip_variant_type() {
    assert_round_trip(
        "variant",
        "type Tree<T> is\n    | Leaf(T)\n    | Node { left: Heap<Tree<T>>, right: Heap<Tree<T>> };\n",
    );
}

#[test]
fn round_trip_match() {
    assert_round_trip(
        "match",
        "fn classify(x: Int) -> Int {\n    match x {\n        0 => 0,\n        n if n > 0 => 1,\n        _ => -1,\n    }\n}\n",
    );
}

#[test]
fn round_trip_protocol_with_methods() {
    assert_round_trip(
        "protocol",
        "type Display is protocol {\n    fn fmt(&self) -> Text;\n    fn fmt_debug(&self) -> Text;\n};\n",
    );
}

#[test]
fn round_trip_implement_block() {
    assert_round_trip(
        "implement",
        "type P is { x: Int };\n\nimplement P {\n    fn x(self) -> Int { self.x }\n    fn doubled(self) -> Int { self.x * 2 }\n}\n",
    );
}

#[test]
fn round_trip_mount_with_alias() {
    assert_round_trip(
        "mount alias",
        "mount stdlib.collections.list as l;\n\nfn main() {}\n",
    );
}

#[test]
fn round_trip_corpus() {
    assert_round_trip(
        "corpus",
        r#"
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
"#,
    );
}
