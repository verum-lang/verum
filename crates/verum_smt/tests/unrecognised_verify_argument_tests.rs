//! `@verify(thorugh)` — a KNOWN attribute whose ARGUMENT names no strategy.
//!
//! `extract_from_attributes` returned `None` for it, which is exactly what
//! it returns when there is no `@verify` attribute at all. The caller could
//! not tell the two apart, so it fell back to the phase default and the
//! function reported everything proved. A one-character typo switched off
//! the verification the author asked for, in silence.
//!
//! The authority for what IS a strategy lives in `verify_strategy.rs`, so
//! the question of what is NOT one is answered there too — a second list in
//! another crate would drift the first time a strategy is added.
//!
//! Task: T1025.

use verum_ast::attr::Attribute;
use verum_ast::expr::Expr;
use verum_ast::{Ident, Span};
use verum_common::{List, Maybe, Text};
use verum_smt::verify_strategy::{extract_from_attributes, unrecognised_verify_arguments};

fn verify_attr(value: &str) -> List<Attribute> {
    let name = Ident::new(Text::from(value), Span::dummy());
    let mut args = List::new();
    args.push(Expr::ident(name));
    let mut attrs = List::new();
    attrs.push(Attribute::new(
        Text::from("verify"),
        Maybe::Some(args),
        Span::dummy(),
    ));
    attrs
}

#[test]
fn a_misspelled_strategy_is_named() {
    let attrs = verify_attr("thorugh");
    assert!(
        extract_from_attributes(&attrs).is_none(),
        "precondition: the typo yields no strategy — that is the silence"
    );
    let bad = unrecognised_verify_arguments(&attrs);
    assert_eq!(
        bad.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
        vec!["thorugh"],
        "the argument the author actually wrote must be named back to them"
    );
}

/// The differentiator: a CORRECT strategy must produce nothing, or the
/// warning would fire on every annotated function in the tree.
#[test]
fn a_correct_strategy_is_not_reported() {
    let attrs = verify_attr("thorough");
    assert!(extract_from_attributes(&attrs).is_some());
    assert!(
        unrecognised_verify_arguments(&attrs).is_empty(),
        "a known strategy is not an unrecognised argument"
    );
}

/// No `@verify` at all is not an unrecognised argument either — the two
/// were indistinguishable before, and the fix must not merge them the
/// other way round.
#[test]
fn an_absent_attribute_reports_nothing() {
    let attrs: List<Attribute> = List::new();
    assert!(unrecognised_verify_arguments(&attrs).is_empty());
}
