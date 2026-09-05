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
//! Regression tests for A75 — a `calc` chain written with trailing `by`
//! accepted exactly ONE step.
//!
//! Measured 2026-09-05 by step count, after five narrower hypotheses were each
//! refuted in isolation:
//!
//! ```text
//!   calc { a = a by x; }                        parses
//!   calc { a = a by x; = a by y; }              E018 expected tactic expression
//!   three steps, four steps                     likewise
//!   calc { a == { by x } a == { by y } a }      parses — the OTHER spelling
//! ```
//!
//! `grammar/verum.ebnf` gives `calc_chain = 'calc' '{' expression calc_step
//! { calc_step } '}'` — one or more, with no distinction between the two
//! justification spellings. So the grammar is the side that is right.
//!
//! Cause: `parse_tactic_seq` treats `;` as a TACTIC-SEQUENCE separator unless
//! the token after it is on a stop list of proof-step and item keywords. A
//! calc relation (`=`, `==`, `<`, …) is on no such list, so the tactic parser
//! consumed the step terminator and then demanded a tactic where the next
//! relation stood. The chain now marks the semicolon as belonging to the
//! construct (`semicolon_ends_construct`), scoped to the step loop and
//! restored on every exit including the error paths.
//!
//! `website:docs/verification/proofs.md` uses four-step trailing-`by` chains,
//! which is why that page kept failing after three rounds of narrower repairs.
//!
//! WHAT THESE TESTS PIN: that step COUNT does not change the verdict, and
//! that both documented spellings chain. Not the shape of the AST.

use verum_fast_parser::Parser;

fn parses(src: &str) -> Result<(), String> {
    Parser::new(src)
        .parse_module()
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}

fn in_theorem(body: &str) -> String {
    format!("theorem t: 1 == 1 {{\n    {body}\n}}\n")
}

/// The control: one step has always parsed.
#[test]
fn a_one_step_trailing_by_chain_parses() {
    let src = in_theorem("calc { a = a by refl; }");
    assert!(parses(&src).is_ok(), "one step: {:?}", parses(&src));
}

/// The subject. Two identical steps — the only change is the count.
#[test]
fn a_trailing_by_chain_takes_more_than_one_step() {
    for n in 2..=4 {
        let steps: String = (0..n).map(|_| " = a by refl;").collect();
        let src = in_theorem(&format!("calc {{ a{steps} }}"));
        assert!(
            parses(&src).is_ok(),
            "{n} trailing-`by` step(s) must parse — one does, and the grammar's \
             `calc_step {{ calc_step }}` says nothing about a limit: {:?}",
            parses(&src)
        );
    }
}

/// The other documented spelling must keep chaining — a fix that moved the
/// acceptance rather than widening it would pass the test above.
#[test]
fn the_brace_justified_spelling_still_chains() {
    let src = in_theorem("calc { a == { by refl } a == { by refl } a }");
    assert!(parses(&src).is_ok(), "brace-justified: {:?}", parses(&src));
}

/// NEGATIVE CONTROL. Outside a calc chain a tactic sequence still spans `;` —
/// the flag is scoped, not global.
#[test]
fn a_tactic_sequence_outside_a_calc_chain_still_spans_semicolons() {
    let src = "theorem t: 1 == 1 {\n    have h: 1 == 1 by simp; refl;\n}\n";
    assert!(
        parses(src).is_ok(),
        "a `;`-separated tactic sequence outside a calc chain must still parse: {:?}",
        parses(src)
    );
}
