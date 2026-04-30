//! Locks the quantifier-aware lemma instantiation in
//! `HintsDatabase::instantiate_lemma`.
//!
//! Pre-fix `extract_quantifiers` was a stub:
//!
//! ```ignore
//! fn extract_quantifiers(expr: &Expr) -> (List<Text>, Expr) {
//!     let mut vars = List::new();
//!     let mut current = expr;
//!     // For now, we don't have explicit forall syntax in ExprKind
//!     (vars, current.clone())
//! }
//! ```
//!
//! The comment was stale: `verum_ast::ExprKind` HAS Forall and Exists
//! variants. The stub returned an empty variable list, so
//! `instantiate_lemma(forall_expr, [t])` errored with "expects 0
//! arguments" — quantified lemmas were unusable.
//!
//! Post-fix the function walks Forall/Exists chains and collects
//! every binding-pattern's identifier, producing a real variable list
//! that matches the lemma's arity.

use verum_ast::expr::{Expr, ExprKind, QuantifierBinding};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{Heap, List, Maybe};

use verum_smt::proof_search::HintsDatabase;

fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        kind: PatternKind::Ident {
            by_ref: false,
            mutable: false,
            name: Ident::new(name, Span::dummy()),
            subpattern: Maybe::None,
        },
        span: Span::dummy(),
    }
}

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn quantifier_binding(name: &str) -> QuantifierBinding {
    QuantifierBinding {
        pattern: ident_pattern(name),
        ty: Maybe::None,
        domain: Maybe::None,
        guard: Maybe::None,
        span: Span::dummy(),
    }
}

fn forall(bindings: List<QuantifierBinding>, body: Expr) -> Expr {
    Expr::new(
        ExprKind::Forall {
            bindings,
            body: Heap::new(body),
        },
        Span::dummy(),
    )
}

fn exists(bindings: List<QuantifierBinding>, body: Expr) -> Expr {
    Expr::new(
        ExprKind::Exists {
            bindings,
            body: Heap::new(body),
        },
        Span::dummy(),
    )
}

#[test]
fn forall_with_one_binding_instantiates_with_one_term() {
    let db = HintsDatabase::new();

    // ∀x. P(x) — body uses x, which we approximate with `ident_expr("x")`
    let mut bindings: List<QuantifierBinding> = List::new();
    bindings.push(quantifier_binding("x"));
    let lemma = forall(bindings, ident_expr("P_of_x"));

    let mut terms: List<Expr> = List::new();
    terms.push(ident_expr("term_a"));

    let result = db.instantiate_lemma(&lemma, &terms);
    assert!(
        result.is_ok(),
        "1 binding + 1 term must succeed (was failing pre-fix because the \
         extractor returned 0 vars): {:?}",
        result.err()
    );
}

#[test]
fn nested_forall_extracts_all_binders() {
    let db = HintsDatabase::new();

    // ∀x. ∀y. P(x, y)
    let mut inner_bindings: List<QuantifierBinding> = List::new();
    inner_bindings.push(quantifier_binding("y"));
    let inner = forall(inner_bindings, ident_expr("body"));

    let mut outer_bindings: List<QuantifierBinding> = List::new();
    outer_bindings.push(quantifier_binding("x"));
    let lemma = forall(outer_bindings, inner);

    let mut terms: List<Expr> = List::new();
    terms.push(ident_expr("a"));
    terms.push(ident_expr("b"));

    let result = db.instantiate_lemma(&lemma, &terms);
    assert!(
        result.is_ok(),
        "nested ∀x. ∀y. body must accept 2 instantiation terms: {:?}",
        result.err()
    );
}

#[test]
fn mixed_forall_exists_chain_extracts_all_binders() {
    let db = HintsDatabase::new();

    // ∀x. ∃y. body
    let mut inner_bindings: List<QuantifierBinding> = List::new();
    inner_bindings.push(quantifier_binding("y"));
    let inner = exists(inner_bindings, ident_expr("body"));

    let mut outer_bindings: List<QuantifierBinding> = List::new();
    outer_bindings.push(quantifier_binding("x"));
    let lemma = forall(outer_bindings, inner);

    // 2 binders → must accept 2 terms.
    let mut terms: List<Expr> = List::new();
    terms.push(ident_expr("a"));
    terms.push(ident_expr("b"));
    assert!(db.instantiate_lemma(&lemma, &terms).is_ok());

    // 1 term must fail with arity mismatch — proves the extractor
    // saw both binders, not just the outer one.
    let mut one_term: List<Expr> = List::new();
    one_term.push(ident_expr("a"));
    let result = db.instantiate_lemma(&lemma, &one_term);
    assert!(
        result.is_err(),
        "instantiating ∀x. ∃y. body with 1 term must fail — extractor \
         must collect both binders"
    );
}

#[test]
fn arity_mismatch_is_reported_for_too_few_terms() {
    let db = HintsDatabase::new();

    let mut bindings: List<QuantifierBinding> = List::new();
    bindings.push(quantifier_binding("x"));
    bindings.push(quantifier_binding("y"));
    let lemma = forall(bindings, ident_expr("body"));

    let mut terms: List<Expr> = List::new();
    terms.push(ident_expr("only_one"));
    let result = db.instantiate_lemma(&lemma, &terms);
    assert!(result.is_err(), "2 binders + 1 term must fail");
    let msg = format!("{}", result.err().unwrap());
    assert!(
        msg.contains("expects 2") && msg.contains("got 1"),
        "error must report arities. got: {}",
        msg
    );
}
