#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Regression tests for A84 — `self[i]` inside a quantifier body did not
//! parse, while `(self)[i]` did.
//!
//! Measured 2026-09-05 with three controls that each isolate one variable:
//!
//! ```text
//!   type S is List<Int> where forall i in 0..10. self[i] == 1;   unexpected operator '=='
//!   type S is List<Int> { self[0] == 1 };                        parses (no quantifier)
//!   fn f(xs: List<Int>) … requires forall i in 0..10. xs[i] == 1 parses (a PARAMETER)
//!   … forall i in 0..10. (self)[i] == 1 …                        parses (parenthesised)
//! ```
//!
//! So it was neither `self`, nor indexing, nor quantifiers, but `self`
//! indexed inside a quantifier body.
//!
//! Cause: after a quantifier's domain, `is_quantifier_body_separator` decides
//! whether the `.` starts the BODY or is member access, by looking at the
//! token after it. Its list covered identifiers, `(`, `[`, `!`, nested
//! quantifiers, booleans and numbers — and everything else fell to "not a
//! separator". `self` is its own token, so `. self[i] == 1` was read as field
//! access `.self`, then `[i]` as postfix, and the domain parser then met `==`
//! below its minimum precedence and stopped — leaving the caller to expect a
//! `.` or `=>` and report `unexpected operator '=='`. `(self)[i]` worked
//! because `.(` was already on the list.
//!
//! The list now also admits the tokens that CANNOT be a field name: `self`,
//! `Self`, a prefix operator (`-`, `*`, `&`, `~`), and a string or char
//! literal.
//!
//! WHAT THESE TESTS PIN: that the natural spelling parses and that the
//! parenthesised workaround keeps parsing. Not the AST shape.

use verum_fast_parser::Parser;

fn parses(src: &str) -> Result<(), String> {
    Parser::new(src)
        .parse_module()
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}

/// The subject: the natural spelling, in the `where` form the row measured.
#[test]
fn self_indexed_inside_a_quantifier_body_parses() {
    let src = "type S is List<Int> where forall i in 0..10. self[i] == 1;\n";
    assert!(parses(src).is_ok(), "`self[i]` in a quantifier body: {:?}", parses(src));
}

/// The braced refinement form failed identically and must be fixed too.
#[test]
fn self_indexed_inside_a_braced_refinement_quantifier_parses() {
    let src = "type S is List<Int> { forall i in 0..10. self[i] == 1 };\n";
    assert!(parses(src).is_ok(), "braced form: {:?}", parses(src));
}

/// The workaround must keep working — a fix that moved the acceptance rather
/// than widening it would pass the tests above.
#[test]
fn the_parenthesised_workaround_still_parses() {
    let src = "type S is List<Int> where forall i in 0..10. (self)[i] == 1;\n";
    assert!(parses(src).is_ok(), "`(self)[i]`: {:?}", parses(src));
}

/// The control that made the row precise: a PARAMETER indexed in the same
/// position always parsed.
#[test]
fn a_parameter_indexed_inside_a_quantifier_body_still_parses() {
    let src = "fn f(xs: List<Int>) -> Bool requires forall i in 0..10. xs[i] == 1 { true }\n";
    assert!(parses(src).is_ok(), "parameter control: {:?}", parses(src));
}

/// NEGATIVE CONTROL. `.` followed by an identifier that continues a method
/// chain must STILL be member access, not a body start — the list decides a
/// genuine ambiguity and widening it must not swallow chains.
#[test]
fn a_method_chain_after_the_domain_is_still_member_access() {
    let src = "fn f(xs: List<Int>) -> Bool requires forall i in xs.filter(pos).len() . i > 0 { true }\n";
    assert!(
        parses(src).is_ok(),
        "a method chain in the domain must still parse: {:?}",
        parses(src)
    );
}
