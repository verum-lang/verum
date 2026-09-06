//! A86 — an active pattern's FIRST argument list is an `expression_list`.
//!
//! `active_pattern_tail = '(' , expression_list , ')' , '(' , [ pattern_list ]
//! , ')'` (`grammar/verum.ebnf`).  Parsing that list as PATTERNS refuses every
//! expression form that has no pattern spelling — a tagged literal among them —
//! before the pattern-to-expression conversion is ever reached, which is why
//! `M(rx#"^a$")(g)` reported `expected pattern` while `M("^a$")(g)` parsed.
//!
//! The four cases are one control set: the same shape with a literal the
//! pattern grammar admits, the shape that used to fail, and the two other
//! spellings of a first argument list, which must keep working.

fn parses(src: &str) -> bool {
    verum_fast_parser::Parser::new(src).parse_module().is_ok()
}

const CONTROL: &str = r#"
fn probe(e: Text) -> Int {
    match e {
        M("^a$")(g) => 1,
        _ => 0,
    }
}
"#;

const TAGGED: &str = r#"
fn probe(e: Text) -> Int {
    match e {
        M(rx#"^a$")(g) => 1,
        _ => 0,
    }
}
"#;

const EMPTY_FIRST_LIST: &str = r#"
fn probe(e: Int) -> Int {
    match e {
        Even()(n) => n,
        _ => 0,
    }
}
"#;

const PLAIN_VARIANT: &str = r#"
type T is A(Int) | B;

fn probe(t: T) -> Int {
    match t {
        A(x) => x,
        B => 0,
    }
}
"#;

#[test]
fn a_plain_literal_in_an_active_pattern_argument_parses() {
    assert!(
        parses(CONTROL),
        "the control must parse — without it a change that broke every active \
         pattern would look like a fix"
    );
}

#[test]
fn a_tagged_literal_in_an_active_pattern_argument_parses() {
    assert!(
        parses(TAGGED),
        "a tagged literal is an expression, and the grammar says this list is \
         an expression list"
    );
}

#[test]
fn an_empty_first_argument_list_still_parses() {
    assert!(parses(EMPTY_FIRST_LIST), "`Even()(n)` is the partial form");
}

#[test]
fn a_plain_variant_pattern_still_parses() {
    assert!(
        parses(PLAIN_VARIANT),
        "`A(x)` has no second paren group, so its list stays PATTERNS"
    );
}
