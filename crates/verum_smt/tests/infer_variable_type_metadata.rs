//! Regression: `infer_variable_type` must drive its decision off the
//! `variant_map` registry + the goal's constructor occurrences, NOT
//! off hardcoded naming heuristics like "starts with `n` → Nat".
//!
//! Pre-fix:
//!
//!     match var.as_str() {
//!         name if name.starts_with('n') || name.ends_with("_nat") => Ok("Nat".into()),
//!         name if name.starts_with('l') || name.contains("list") => Ok("List".into()),
//!         name if name.starts_with('t') || name.contains("tree") => Ok("Tree".into()),
//!         name if name.contains("vec") => Ok("Vec".into()),
//!         _ => Ok("Nat".into()),
//!     }
//!
//! Two architectural violations: hardcoded type names + name-prefix
//! heuristics. Misclassified e.g. `nodes` as Nat, `tail` as Tree.
//!
//! Post-fix walks the goal/hypotheses for `var == Ctor(...)` patterns
//! and resolves Ctor through `variant_map`. Returns a real error when
//! no constructor is observable — no silent default.

use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::{Heap, List, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

#[test]
fn infers_type_from_registered_constructor_in_goal() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Color"),
        vec![
            Text::from("Red"),
            Text::from("Green"),
            Text::from("Blue"),
        ],
    );

    // Goal: c == Red
    let goal_expr = binary(BinOp::Eq, ident_expr("c"), ident_expr("Red"));
    let goal = ProofGoal::with_hypotheses(goal_expr, List::new());

    let result = engine.infer_variable_type_for_test(&Text::from("c"), &goal);
    let ty = result.expect("must resolve `c`");
    assert_eq!(
        ty.as_str(),
        "Color",
        "must resolve `c` to Color via the registered variant_map, not via name heuristics"
    );
}

#[test]
fn finds_constructor_in_hypothesis_when_goal_lacks_one_inner() {
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Maybe"),
        vec![Text::from("None"), Text::from("Some")],
    );

    let mut hyps = List::new();
    hyps.push(binary(BinOp::Eq, ident_expr("c"), ident_expr("None")));
    let goal_expr = ident_expr("any_goal");
    let goal = ProofGoal::with_hypotheses(goal_expr, hyps);

    let result = engine.infer_variable_type_for_test(&Text::from("c"), &goal);
    let ty = result.expect("must find type from hypothesis");
    assert_eq!(ty.as_str(), "Maybe");
}

#[test]
fn rejects_unregistered_variable_with_real_error() {
    let engine = ProofSearchEngine::new();
    // No variant types registered. Goal contains no constructor for x.
    let goal_expr = binary(
        BinOp::Eq,
        ident_expr("x"),
        ident_expr("y"),
    );
    let goal = ProofGoal::with_hypotheses(goal_expr, List::new());

    let result = engine.infer_variable_type_for_test(&Text::from("x"), &goal);
    assert!(
        result.is_err(),
        "must error when no constructor for x is observable — pre-fix this defaulted to `Nat`"
    );
    let msg = format!("{}", result.err().unwrap());
    assert!(
        msg.contains("cannot infer type") && msg.contains("'x'"),
        "error must explain the failure mode and name the variable. got: {}",
        msg
    );
}

#[test]
fn does_not_default_to_nat_for_non_numeric_names() {
    // The exact case the heuristic mishandled: a variable named `n`
    // should NOT be classified as Nat by name. With no goal-context
    // constructor, the function must error rather than defaulting.
    let engine = ProofSearchEngine::new();
    let goal_expr = ident_expr("Q");
    let goal = ProofGoal::with_hypotheses(goal_expr, List::new());

    let result = engine.infer_variable_type_for_test(&Text::from("n"), &goal);
    assert!(
        result.is_err(),
        "variable `n` with no constructor context must NOT silently become Nat"
    );
}

#[test]
fn finds_constructor_in_hypothesis_when_goal_lacks_one() {
    // Goal doesn't constrain `c`, but a hypothesis does. Both paths
    // are walked.
    let mut engine = ProofSearchEngine::new();
    engine.register_variant_type(
        Text::from("Maybe"),
        vec![Text::from("None"), Text::from("Some")],
    );

    let mut hyps = List::new();
    hyps.push(binary(BinOp::Eq, ident_expr("c"), ident_expr("None")));
    let goal_expr = ident_expr("any_goal");
    let goal = ProofGoal::with_hypotheses(goal_expr, hyps);

    let result = engine.infer_variable_type_for_test(&Text::from("c"), &goal);
    let ty = result.expect("must find type from hypothesis");
    assert_eq!(
        ty.as_str(),
        "Maybe",
        "type must be discoverable from a hypothesis when the goal itself doesn't mention the variable"
    );
}
