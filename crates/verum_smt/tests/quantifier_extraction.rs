//! Schematic universal instantiation preserves actual arguments and refuses
//! existential witness substitution and unchecked binder metadata.

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
    let lemma = forall(bindings, ident_expr("x"));

    let mut terms: List<Expr> = List::new();
    terms.push(ident_expr("term_a"));

    let result = db.instantiate_lemma(&lemma, &terms);
    let result = result.expect("universal instantiation");
    assert!(matches!(result.kind, ExprKind::Path(ref path) if path.as_ident().unwrap().as_str() == "term_a"));
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
fn an_existential_is_not_a_universal_instantiation_parameter() {
    let db = HintsDatabase::new();

    // ∀x. ∃y. body
    let mut inner_bindings: List<QuantifierBinding> = List::new();
    inner_bindings.push(quantifier_binding("y"));
    let inner = exists(inner_bindings, ident_expr("body"));

    let mut outer_bindings: List<QuantifierBinding> = List::new();
    outer_bindings.push(quantifier_binding("x"));
    let lemma = forall(outer_bindings, inner);

    for terms in [List::from_iter([ident_expr("a"), ident_expr("b")]), List::from_iter([ident_expr("a")])] {
        assert!(db.instantiate_lemma(&lemma, &terms).is_err(),
            "existential witnesses cannot be chosen by a caller's universal instantiation");
    }
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
