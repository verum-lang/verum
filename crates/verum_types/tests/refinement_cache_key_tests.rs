//! The verification cache must not answer one obligation with another's verdict.
//!
//! `RefinementChecker` keys its cache on a structural hash of the verification
//! condition and looks the verdict up by that number alone — there is no
//! equality check on the condition itself.  So the hash owes its caller
//! injectivity over structure: two obligations that differ anywhere must differ
//! in the key, or the first one's answer is handed to the second.
//!
//! `hash_expr` used to cover 11 of the language's 73 expression forms and hash
//! a constant `255u8` for the other 62 — WITHOUT their children.  Every
//! instance of an absorbed form therefore hashed identically no matter what it
//! contained: `(a, 0).0` and `(b, 7).1` were the same number, as were
//! `x as Int` / `y as Float`, `f"{a}"` / `f"{b}"`, and `a ?? b` / `c ?? d`.
//!
//! The collision is MEASURED here, not inferred.  On the old hash the second
//! obligation below came back as a cache hit — `smt_checks` stayed at 1 while
//! `cache_hits` went to 1; with the children hashed it is `smt_checks` 2 and
//! `cache_hits` 0, so both obligations are now decided on their own.
//!
//! What is NOT claimed is a source program whose verdict changes.  Getting
//! there needs two obligations that both reach the cache AND disagree, and the
//! cache sits behind `try_syntactic_eval`, which decides anything closed —
//! including tuple projection, measured.  So an obligation that reaches the
//! cache still holds a free variable, and for a projection the verdict then
//! depends on that variable rather than on the part of the tree the hash
//! dropped.  The wider channel is `assumptions`, which the key also covers and
//! where the dropped subtree is the one naming the variable — `(n, 0).0 >= 20`
//! and `(m, 0).0 >= 20` are one key — but no such program was built, so no such
//! claim is made.  A structural hash owes its caller injectivity over
//! structure either way; that is what this file pins.
//!
//! NOTE: this file lives in `tests/`, which CI does not currently run — its
//! only test invocation is `cargo test --workspace --lib --bins`, and
//! `--lib --bins` excludes integration tests.  The gate is therefore inert
//! until that changes.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::{FileId, Span};
use verum_ast::ty::Ident;
use verum_common::{Heap, Text};
use verum_types::context::TypeContext;
use verum_types::refinement::{
    RefinementConfig,
    PredicateProvenance, RefinementBinding, RefinementChecker, RefinementPredicate, RefinementType,
};

fn span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

fn int_lit(value: i128) -> Expr {
    Expr::literal(Literal::int(value, span()))
}

fn var(name: &str) -> Expr {
    Expr::ident(Ident::new(Text::from(name), span()))
}

fn tuple(items: Vec<Expr>) -> Expr {
    Expr::new(ExprKind::Tuple(items.into()), span())
}

fn projection(inner: Expr, index: u32) -> Expr {
    Expr::new(
        ExprKind::TupleIndex {
            expr: Heap::new(inner),
            index,
        },
        span(),
    )
}

fn at_least(left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Ge,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span(),
    )
}

fn refined_int(predicate: Expr) -> RefinementType {
    RefinementType {
        base_type: verum_types::ty::Type::int(),
        predicate: RefinementPredicate {
            predicate,
            binding: RefinementBinding::Lambda(Text::from("it")),
            provenance: PredicateProvenance::Declared,
            span: span(),
        },
        span: span(),
    }
}

/// Two predicates differing only INSIDE a projection must not collide.
///
/// Both are `Binary(Ge, TupleIndex, 10)` at the top and differ only in the
/// tuple's second element, which the old hash never reached — so they were one
/// number, and the second obligation was answered by the first.
///
/// Two details are load-bearing, and each was learned by watching a version of
/// this test pass for the wrong reason:
///
///   * the subject is a FREE VARIABLE.  With a literal the predicate is closed,
///     `try_syntactic_eval` decides it, and the check returns before the cache
///     is consulted at all — measured: `syntactic_checks` 1, `cache_hits` 0.
///   * the projected slot holds `it` in BOTH.  Project a literal slot instead
///     and the syntactic evaluator decides that one too, so the two obligations
///     take different paths and never meet in the cache — which is exactly how
///     the first version of this test came to pass on the broken hash.
#[test]
fn predicates_differing_inside_a_projection_do_not_share_a_cache_entry() {
    let constrains_subject = refined_int(at_least(
        projection(tuple(vec![var("it"), int_lit(0)]), 0),
        int_lit(10),
    ));
    let different_tuple = refined_int(at_least(
        projection(tuple(vec![var("it"), int_lit(7)]), 0),
        int_lit(10),
    ));

    let subject = var("n");
    let ctx = TypeContext::new();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    let _ = checker.check(&subject, &constrains_subject, &ctx);
    let hits_after_first = checker.stats().cache_hits;
    let _ = checker.check(&subject, &different_tuple, &ctx);
    let hits_after_second = checker.stats().cache_hits;

    assert_eq!(
        hits_after_first, 0,
        "the first obligation cannot hit an empty cache; if it does, the \
         checker is answering from somewhere this test does not model"
    );
    assert_eq!(
        hits_after_second, 0,
        "the second obligation was answered from the first one's cache entry: \
         `(it, 0).0 >= 10` and `(it, 7).0 >= 10` hashed to the same key, so the \
         structural hash is not injective over structure"
    );
}

/// The control: an obligation repeated VERBATIM should hit the cache.
///
/// Without this, the assertion above is satisfied by a hash so fine-grained
/// that nothing ever hits — or by a cache that silently never stores.  A test
/// that only checks "no false hit" passes just as well when the feature it
/// guards is dead.
#[test]
fn the_identical_obligation_does_hit_the_cache() {
    let refinement = refined_int(at_least(
        projection(tuple(vec![var("it"), int_lit(0)]), 0),
        int_lit(10),
    ));

    let subject = var("n");
    let ctx = TypeContext::new();
    let mut checker = RefinementChecker::new(RefinementConfig::default());

    let _ = checker.check(&subject, &refinement, &ctx);
    let _ = checker.check(&subject, &refinement, &ctx);

    assert_eq!(
        checker.stats().cache_hits, 1,
        "the same obligation twice must be answered from the cache the second \
         time — if this is 0 the cache is not reached at all, and the \
         no-collision assertion in this file proves nothing"
    );
}
