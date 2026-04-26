//! Idempotence contract: `fmt(fmt(x)) == fmt(x)` for every fixture.
//!
//! A formatter that doesn't reach a fixed point in one pass is
//! broken — running `verum fmt` after another `verum fmt` would
//! produce a diff, breaking format-on-save and CI's `fmt --check`
//! invariant. Each fixture in the lint test corpus + a few targeted
//! convergence-hazard shapes is exercised here.

use verum_cli::commands::fmt::format_string;

fn fmt(source: &str) -> String {
    format_string(source)
        .expect("format_string returned Err")
        .to_string()
}

fn assert_idempotent(label: &str, source: &str) {
    let once = fmt(source);
    let twice = fmt(&once);
    assert_eq!(
        once, twice,
        "fixture `{label}` is not idempotent: \n\
         === first ===\n{once}\n\
         === second ===\n{twice}"
    );
}

#[test]
fn idempotent_on_empty() {
    assert_idempotent("empty", "");
}

#[test]
fn idempotent_on_function() {
    assert_idempotent("fn", "fn main() {}\n");
}

#[test]
fn idempotent_on_type_def() {
    assert_idempotent("type", "type Point is { x: Int, y: Int };\n");
}

#[test]
fn idempotent_on_variant_type() {
    assert_idempotent("variant", "type Maybe<T> is None | Some(T);\n");
}

#[test]
fn idempotent_on_attributed_function() {
    assert_idempotent(
        "attributed fn",
        "@verify(formal)\npublic fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n",
    );
}

#[test]
fn idempotent_on_match_expression() {
    assert_idempotent(
        "match",
        "fn classify(x: Int) -> Int {\n    match x {\n        0 => 0,\n        _ => 1,\n    }\n}\n",
    );
}

#[test]
fn idempotent_on_protocol() {
    assert_idempotent(
        "protocol",
        "type Display is protocol {\n    fn fmt(&self) -> Text;\n};\n",
    );
}

#[test]
fn idempotent_on_implement_block() {
    assert_idempotent(
        "implement",
        "type P is { x: Int };\n\nimplement P {\n    fn x(self) -> Int { self.x }\n}\n",
    );
}

#[test]
fn idempotent_on_mount_with_alias() {
    assert_idempotent("mount alias", "mount stdlib.collections.list as l;\n\nfn main() {}\n");
}

#[test]
fn idempotent_on_nested_mount() {
    assert_idempotent(
        "mount nested",
        "mount stdlib.{collections.list, io};\n\nfn main() {}\n",
    );
}

#[test]
fn idempotent_on_excessive_blank_lines() {
    // Convergence hazard: blank-line normalisation must reach a
    // fixed point. The input has many blanks; the formatter
    // collapses to at most one consecutive blank, and the second
    // pass sees the canonical form.
    assert_idempotent(
        "blanks",
        "fn a() {}\n\n\n\n\nfn b() {}\n\n\n\nfn c() {}\n",
    );
}

#[test]
fn idempotent_on_corpus() {
    assert_idempotent(
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
