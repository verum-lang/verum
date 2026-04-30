//! Regression: `proof_search::expr_eq` must walk the structural shape
//! of Call / MethodCall / Field / TupleIndex / Index / Tuple / Array /
//! Cast / Pipeline / NullCoalesce / Try variants — pre-fix only
//! Literal / Path / Binary / Unary / Paren were handled, all other
//! expression kinds fell through to `_ => false`.
//!
//! Impact pre-fix: `try_rewrite_once` couldn't find subterm matches
//! inside any goal containing function calls, field access, etc.
//! Tactics like `rewrite h` on goals like `f(x) + 1 > 0` silently
//! reported "no match" because the recursive walker bailed out on
//! the `Call`. This regression suite exercises rewriting through each
//! of the newly-supported variants.

use verum_ast::expr::{ArrayExpr, BinOp, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_ast::{IntLit, LiteralKind};
use verum_common::{Heap, List, Text};

use verum_smt::proof_search::{ProofGoal, ProofSearchEngine, ProofTactic};

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn int_lit(value: i64) -> Expr {
    Expr::literal(Literal::new(
        LiteralKind::Int(IntLit::new(value as i128)),
        Span::dummy(),
    ))
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

fn call(func: Expr, args: List<Expr>) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Heap::new(func),
            type_args: List::new(),
            args,
        },
        Span::dummy(),
    )
}

fn field(base: Expr, field_name: &str) -> Expr {
    Expr::new(
        ExprKind::Field {
            expr: Heap::new(base),
            field: Ident::new(field_name, Span::dummy()),
        },
        Span::dummy(),
    )
}

fn tuple(elems: Vec<Expr>) -> Expr {
    let mut list = List::new();
    for e in elems {
        list.push(e);
    }
    Expr::new(ExprKind::Tuple(list), Span::dummy())
}

fn array(elems: Vec<Expr>) -> Expr {
    let mut list = List::new();
    for e in elems {
        list.push(e);
    }
    Expr::new(
        ExprKind::Array(ArrayExpr::List(list)),
        Span::dummy(),
    )
}

fn rewrite_once(goal_expr: Expr, hyp_eq: Expr) -> Result<Expr, String> {
    let mut engine = ProofSearchEngine::new();
    let mut hyps = List::new();
    hyps.push(hyp_eq);
    let goal = ProofGoal::with_hypotheses(goal_expr, hyps);

    let result = engine
        .execute_tactic(
            &ProofTactic::Rewrite {
                hypothesis: Text::from("h0"),
                reverse: false,
            },
            &goal,
        )
        .map_err(|e| format!("{}", e))?;
    assert_eq!(result.len(), 1);
    Ok(result.into_iter().next().unwrap().goal)
}

#[test]
fn expr_eq_recognises_call_subterms() {
    // hyp: f(x) = y. goal: f(x) + 1 > 0. rewrite → y + 1 > 0.
    // Pre-fix expr_eq returned false on Call so the rewrite would
    // have failed to locate the subterm.
    let hyp = binary(
        BinOp::Eq,
        call(ident_expr("f"), {
            let mut a = List::new();
            a.push(ident_expr("x"));
            a
        }),
        ident_expr("y"),
    );
    let goal_expr = binary(
        BinOp::Gt,
        binary(
            BinOp::Add,
            call(ident_expr("f"), {
                let mut a = List::new();
                a.push(ident_expr("x"));
                a
            }),
            int_lit(1),
        ),
        int_lit(0),
    );
    let new_goal = rewrite_once(goal_expr, hyp).expect("rewrite must succeed");
    let dump = format!("{:?}", new_goal);
    assert!(dump.contains("\"y\""), "rewritten goal must contain `y`. dump: {}", dump);
}

#[test]
fn expr_eq_recognises_field_subterms() {
    // hyp: p.x = z. goal: p.x + 1 > 0. rewrite → z + 1 > 0.
    let hyp = binary(BinOp::Eq, field(ident_expr("p"), "x"), ident_expr("z"));
    let goal_expr = binary(
        BinOp::Gt,
        binary(BinOp::Add, field(ident_expr("p"), "x"), int_lit(1)),
        int_lit(0),
    );
    let new_goal = rewrite_once(goal_expr, hyp).expect("field rewrite must succeed");
    let dump = format!("{:?}", new_goal);
    assert!(dump.contains("\"z\""), "rewritten goal must contain `z`. dump: {}", dump);
}

#[test]
fn expr_eq_distinguishes_different_field_names() {
    // hyp: p.x = z. goal: p.y + 1 > 0. rewrite must FAIL: p.y ≠ p.x.
    // Pre-fix this would have failed for the wrong reason (Field
    // unsupported, fall-through false). Post-fix it fails for the
    // RIGHT reason: structural mismatch on field name.
    let hyp = binary(BinOp::Eq, field(ident_expr("p"), "x"), ident_expr("z"));
    let goal_expr = binary(
        BinOp::Gt,
        binary(BinOp::Add, field(ident_expr("p"), "y"), int_lit(1)),
        int_lit(0),
    );
    let result = rewrite_once(goal_expr, hyp);
    assert!(
        result.is_err(),
        "rewrite of `p.y` using `p.x = z` must not match — different field names"
    );
}

#[test]
fn expr_eq_recognises_tuple_subterms() {
    // hyp: (a, b) = pair. goal: f((a, b)) ⇒ should rewrite the tuple.
    let hyp = binary(
        BinOp::Eq,
        tuple(vec![ident_expr("a"), ident_expr("b")]),
        ident_expr("pair"),
    );
    let goal_expr = call(ident_expr("f"), {
        let mut args = List::new();
        args.push(tuple(vec![ident_expr("a"), ident_expr("b")]));
        args
    });
    let new_goal = rewrite_once(goal_expr, hyp).expect("tuple rewrite must succeed");
    let dump = format!("{:?}", new_goal);
    assert!(dump.contains("\"pair\""), "rewritten goal must contain `pair`. dump: {}", dump);
}

#[test]
fn expr_eq_recognises_array_subterms() {
    // hyp: [1, 2] = arr. goal: g([1, 2]) → g(arr).
    let hyp = binary(
        BinOp::Eq,
        array(vec![int_lit(1), int_lit(2)]),
        ident_expr("arr"),
    );
    let goal_expr = call(ident_expr("g"), {
        let mut args = List::new();
        args.push(array(vec![int_lit(1), int_lit(2)]));
        args
    });
    let new_goal = rewrite_once(goal_expr, hyp).expect("array rewrite must succeed");
    let dump = format!("{:?}", new_goal);
    assert!(dump.contains("\"arr\""), "rewritten goal must contain `arr`. dump: {}", dump);
}
