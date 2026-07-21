//! T0457 — pins for the two refinement judgments.
//!

//! The false positive these pin: a declaration-site refinement check
//! (`fn f(max_level: Int{>= 0 && <= 5})`, no value in hand) used to ask
//! "does *every* Int satisfy the predicate?" and report the inevitable
//! `Sat` as "unsatisfiable refinement constraint". Every non-tautological
//! refinement was rejected.
//!

//! Both directions are pinned deliberately. Fixing a false positive by
//! weakening the check into unsoundness would pass the "valid programs
//! accepted" half and fail the "invalid programs rejected" half, so the
//! genuinely-uninhabited cases below are as load-bearing as the
//! satisfiable ones.

use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, RefinementPredicate, Span, Type, TypeKind};
use verum_common::Heap;
use verum_smt::{Context, RefinementJudgment, VerifyMode, verify_refinement};

// ── AST helpers ────────────────────────────────────────────────────

fn dummy_span() -> Span {
    Span::dummy()
}

fn int_lit(value: i64) -> Expr {
    Expr::literal(Literal::int(value as i128, dummy_span()))
}

fn ident_expr(name: &str) -> Expr {
    Expr::ident(Ident::new(name, dummy_span()))
}

fn binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        dummy_span(),
    )
}

fn refined_type(base: Type, predicate: Expr) -> Type {
    Type::new(
        TypeKind::Refined {
            base: Box::new(base),
            predicate: Box::new(RefinementPredicate::new(predicate, dummy_span())),
        },
        dummy_span(),
    )
}

/// `Int{it <op1> a && it <op2> b}`
fn bounded_int(op1: BinOp, a: i64, op2: BinOp, b: i64) -> Type {
    let lower = binary_expr(op1, ident_expr("it"), int_lit(a));
    let upper = binary_expr(op2, ident_expr("it"), int_lit(b));
    refined_type(
        Type::int(dummy_span()),
        binary_expr(BinOp::And, lower, upper),
    )
}

/// Assert that a declared refinement type is inhabited, i.e. accepted at
/// a declaration site.
fn assert_inhabited(ty: &Type, what: &str) {
    let ctx = Context::new();
    let result = verify_refinement(&ctx, ty, None, VerifyMode::Proof);
    assert!(
        result.is_ok(),
        "{what} is inhabited and must be accepted at a declaration site; got {:?}",
        result.err()
    );
}

/// Assert that a declared refinement type has no inhabitants, i.e. is
/// still rejected.
fn assert_uninhabited(ty: &Type, what: &str) {
    let ctx = Context::new();
    let result = verify_refinement(&ctx, ty, None, VerifyMode::Proof);
    assert!(
        result.is_err(),
        "{what} has no inhabitants and must stay rejected"
    );
}

// ── Accepted: satisfiable declarations ─────────────────────────────

/// The T0457 repro, verbatim: `max_level: Int{>= 0 && <= 5}`.
#[test]
fn t0457_non_strict_conjunction_is_inhabited() {
    assert_inhabited(
        &bounded_int(BinOp::Ge, 0, BinOp::Le, 5),
        "Int{>= 0 && <= 5}",
    );
}

#[test]
fn t0457_strict_conjunction_is_inhabited() {
    assert_inhabited(&bounded_int(BinOp::Gt, 0, BinOp::Lt, 5), "Int{> 0 && < 5}");
}

#[test]
fn t0457_mixed_strict_non_strict_is_inhabited() {
    assert_inhabited(&bounded_int(BinOp::Gt, 0, BinOp::Le, 5), "Int{> 0 && <= 5}");
    assert_inhabited(&bounded_int(BinOp::Ge, 0, BinOp::Lt, 5), "Int{>= 0 && < 5}");
}

#[test]
fn t0457_negative_bounds_are_inhabited() {
    assert_inhabited(
        &bounded_int(BinOp::Ge, -10, BinOp::Le, -1),
        "Int{>= -10 && <= -1}",
    );
    assert_inhabited(
        &bounded_int(BinOp::Gt, -1, BinOp::Lt, 1),
        "Int{> -1 && < 1} (the single point 0)",
    );
}

/// A one-element interval is legal, not empty: `it >= 5 && it <= 5`
/// admits exactly 5.
#[test]
fn t0457_singleton_range_is_inhabited() {
    assert_inhabited(&bounded_int(BinOp::Ge, 5, BinOp::Le, 5), "Int{>= 5 && <= 5}");
}

/// Half-open, unbounded above — the shape most stdlib refinements use.
#[test]
fn t0457_single_sided_bounds_are_inhabited() {
    let ctx = Context::new();
    for (op, bound, label) in [
        (BinOp::Gt, 0, "Int{> 0}"),
        (BinOp::Ge, 0, "Int{>= 0}"),
        (BinOp::Lt, 0, "Int{< 0}"),
        (BinOp::Le, 0, "Int{<= 0}"),
        (BinOp::Ne, 0, "Int{!= 0}"),
    ] {
        let ty = refined_type(
            Type::int(dummy_span()),
            binary_expr(op, ident_expr("it"), int_lit(bound)),
        );
        let result = verify_refinement(&ctx, &ty, None, VerifyMode::Proof);
        assert!(
            result.is_ok(),
            "{label} is inhabited and must be accepted; got {:?}",
            result.err()
        );
    }
}

#[test]
fn t0457_disjunction_is_inhabited() {
    let lower = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(0));
    let upper = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(100));
    let ty = refined_type(
        Type::int(dummy_span()),
        binary_expr(BinOp::Or, lower, upper),
    );
    assert_inhabited(&ty, "Int{< 0 || > 100}");
}

/// Three-way conjunction with a hole: `> 10 && < 20 && != 15`.
#[test]
fn t0457_conjunction_with_exclusion_is_inhabited() {
    let gt10 = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(10));
    let lt20 = binary_expr(BinOp::Lt, ident_expr("it"), int_lit(20));
    let ne15 = binary_expr(BinOp::Ne, ident_expr("it"), int_lit(15));
    let ty = refined_type(
        Type::int(dummy_span()),
        binary_expr(BinOp::And, binary_expr(BinOp::And, gt10, lt20), ne15),
    );
    assert_inhabited(&ty, "Int{> 10 && < 20 && != 15}");
}

// ── Rejected: the check must keep its teeth ────────────────────────

#[test]
fn t0457_contradictory_bounds_stay_rejected() {
    assert_uninhabited(&bounded_int(BinOp::Gt, 10, BinOp::Lt, 5), "Int{> 10 && < 5}");
}

/// Off-by-one emptiness — the case a range check is most likely to get
/// wrong, and the one a weakened checker would wave through.
#[test]
fn t0457_empty_integer_interval_stays_rejected() {
    assert_uninhabited(&bounded_int(BinOp::Ge, 1, BinOp::Le, 0), "Int{>= 1 && <= 0}");
    assert_uninhabited(&bounded_int(BinOp::Gt, 0, BinOp::Lt, 1), "Int{> 0 && < 1}");
}

#[test]
fn t0457_self_contradiction_stays_rejected() {
    let gt = binary_expr(BinOp::Gt, ident_expr("it"), int_lit(0));
    let le = binary_expr(BinOp::Le, ident_expr("it"), int_lit(0));
    let ty = refined_type(Type::int(dummy_span()), binary_expr(BinOp::And, gt, le));
    assert_uninhabited(&ty, "Int{> 0 && <= 0}");
}

// ── Membership: the value-carrying judgment ────────────────────────
//
// `value_expr` used to be ignored outright (the parameter was spelled
// `_value_expr`), so the same verdict came back whatever value was
// supplied. These pin that the value now decides the outcome.

#[test]
fn t0457_membership_accepts_a_conforming_value() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Ge, 0, BinOp::Le, 5);
    let result = verify_refinement(&ctx, &ty, Some(&int_lit(3)), VerifyMode::Proof);
    assert!(
        result.is_ok(),
        "3 satisfies Int{{>= 0 && <= 5}}; got {:?}",
        result.err()
    );
}

#[test]
fn t0457_membership_rejects_a_violating_value() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Ge, 0, BinOp::Le, 5);
    let result = verify_refinement(&ctx, &ty, Some(&int_lit(9)), VerifyMode::Proof);
    assert!(result.is_err(), "9 violates Int{{>= 0 && <= 5}}");
}

/// Both boundary values are members of a closed interval.
#[test]
fn t0457_membership_accepts_both_endpoints() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Ge, 0, BinOp::Le, 5);
    for v in [0, 5] {
        let result = verify_refinement(&ctx, &ty, Some(&int_lit(v)), VerifyMode::Proof);
        assert!(
            result.is_ok(),
            "{v} is an endpoint of Int{{>= 0 && <= 5}} and must be accepted; got {:?}",
            result.err()
        );
    }
}

/// Just outside each endpoint must fail — the pin that stops the
/// membership check from degenerating into "always Ok".
#[test]
fn t0457_membership_rejects_values_just_outside() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Ge, 0, BinOp::Le, 5);
    for v in [-1, 6] {
        let result = verify_refinement(&ctx, &ty, Some(&int_lit(v)), VerifyMode::Proof);
        assert!(
            result.is_err(),
            "{v} is outside Int{{>= 0 && <= 5}} and must be rejected"
        );
    }
}

/// Distinct values against the same (predicate, base type) must get
/// distinct verdicts: the verification cache keys on the predicate and
/// the base type only, so a cached membership verdict would hand the
/// second value the first one's answer.
#[test]
fn t0457_membership_is_not_served_from_the_inhabitation_cache() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Ge, 0, BinOp::Le, 5);

    let inside = verify_refinement(&ctx, &ty, Some(&int_lit(2)), VerifyMode::Proof);
    let outside = verify_refinement(&ctx, &ty, Some(&int_lit(42)), VerifyMode::Proof);

    assert!(inside.is_ok(), "2 is inside the range; got {:?}", inside.err());
    assert!(
        outside.is_err(),
        "42 is outside the range and must not inherit 2's verdict"
    );
}

/// Runtime mode short-circuits before any judgment is asked, so even an
/// uninhabited type comes back Ok — deferred to a runtime check.
#[test]
fn t0457_runtime_mode_defers_both_judgments() {
    let ctx = Context::new();
    let ty = bounded_int(BinOp::Gt, 10, BinOp::Lt, 5);
    assert!(verify_refinement(&ctx, &ty, None, VerifyMode::Runtime).is_ok());
    assert!(verify_refinement(&ctx, &ty, Some(&int_lit(3)), VerifyMode::Runtime).is_ok());
}

// ── Guard: an undecided verdict splits by judgment ──────────────────
//
// When the solver returns `SatResult::Unknown`, the two judgments must
// diverge, and the divergence is a property of the judgment itself —
// read once through `RefinementJudgment::rejects_on_unknown` and applied
// verbatim at both call sites (refinement.rs, verify.rs) — not a policy
// each caller re-decides. Membership is a *universal* obligation the
// user's code created (`∀. P[it := e]`), so an undecided verdict leaves a
// real proof obligation open and conservative rejection is sound.
// Inhabitation is *existential* (`∃x. P(x)`) and only ever volunteered at
// a declaration site, so failing to find a witness within the solver's
// budget is no evidence the type is empty; rejecting there would reinstate
// T0457 for every predicate the solver cannot decide and make the verdict
// depend on machine load.
//
// This is pinned at the policy level rather than end-to-end on a real
// `Unknown` deliberately: an `Unknown` cannot be forced deterministically.
// z3 decides these predicates in microseconds, and the only levers that
// provoke `Unknown` — a genuinely hard predicate, or a wall-clock/memory
// budget (there is no deterministic step-limit knob on `ContextConfig`) —
// are precisely the load-flaky constructs a regression pin must not carry.
#[test]
fn t0457_unknown_verdict_splits_by_judgment() {
    assert!(
        !RefinementJudgment::Inhabited.rejects_on_unknown(),
        "an undecided inhabitation check must NOT reject: a declaration \
         site volunteered it, and no witness within budget is not proof \
         the type is uninhabited (T0457 guard)"
    );

    let value = int_lit(0);
    assert!(
        RefinementJudgment::Satisfies(&value).rejects_on_unknown(),
        "an undecided membership check must reject: it is a universal \
         proof obligation the program itself created"
    );
}
