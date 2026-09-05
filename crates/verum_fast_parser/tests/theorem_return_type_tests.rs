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
//! Regression tests for A85 — a theorem with `-> T` and no statement was
//! refused; without the return type it parsed.
//!
//! `theorem_tail` (`grammar/verum.ebnf`) makes `[ '->' , type_expr ]` optional
//! and INDEPENDENT of the statement, so a theorem may carry a return type, no
//! contract clause, no `:` expression, and a proof body. Measured 2026-09-05:
//!
//! ```text
//!   theorem t() { proof by simp }             parses — no statement at all
//!   theorem t(): true { … }                   parses
//!   theorem t() ensures true { … }            parses
//!   theorem t() -> Int { proof by simp }      unexpected keyword 'by'
//!   theorem t() -> Int: true { … }            parses — a statement rescues it
//! ```
//!
//! So it was the return type WITHOUT a statement, and the grammar is the side
//! that is right.
//!
//! Cause: the theorem parser read its return type with `parse_type_no_sigma`,
//! which is not a SIGNATURE context — so the proof body's `{ … }` was taken
//! for a refinement predicate on the return type, and the `by` inside it was
//! the reported error. `brace_group_is_refinement` already decides this
//! correctly for functions: a group is a refinement exactly when the
//! signature CONTINUES after it, and after a proof body nothing does.
//!
//! Documentation consequence, already fixed: `website:docs/reference/tactics.md`
//! wrote `theorem refl<T>(x: T) -> Path<T>(x, x)` — the Curry-Howard reading
//! where the return type IS the claim.
//!
//! WHAT THESE TESTS PIN: that the return type and the statement are
//! independent, in all four combinations, and that a real refinement on a
//! theorem's return type still parses.

use verum_fast_parser::Parser;

fn parses(src: &str) -> Result<(), String> {
    Parser::new(src)
        .parse_module()
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}

/// The four combinations of (return type present?) × (statement present?).
/// Three parsed before; the fourth is the subject.
#[test]
fn a_return_type_and_a_statement_are_independent() {
    let cases = [
        ("neither", "theorem t() { proof by simp }\n"),
        ("statement only", "theorem t(): true { proof by simp }\n"),
        ("return type only", "theorem t() -> Int { proof by simp }\n"),
        ("both", "theorem t() -> Int: true { proof by simp }\n"),
    ];
    for (what, src) in cases {
        assert!(
            parses(src).is_ok(),
            "{what}: the grammar makes the return type and the statement \
             independent: {:?}",
            parses(src)
        );
    }
}

/// The `ensures` form, which supplies the statement a different way.
#[test]
fn a_return_type_beside_an_ensures_clause_parses() {
    let src = "theorem t() -> Int ensures true { proof by simp }\n";
    assert!(parses(src).is_ok(), "with `ensures`: {:?}", parses(src));
}

/// NEGATIVE CONTROL. A genuine refinement on a theorem's return type must
/// still parse as a refinement — the fix decides brace-vs-body, it does not
/// stop refinements being written.
#[test]
fn a_refinement_on_a_theorem_return_type_still_parses() {
    let src = "theorem t() -> Int{> 0}: true { proof by simp }\n";
    assert!(
        parses(src).is_ok(),
        "a refinement followed by a statement and a body: {:?}",
        parses(src)
    );
}

/// A theorem with a return type and NO body keeps parsing — the declaration
/// form.
#[test]
fn a_bodiless_theorem_with_a_return_type_parses() {
    let src = "theorem t() -> Int;\n";
    assert!(parses(src).is_ok(), "bodiless: {:?}", parses(src));
}
