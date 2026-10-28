//! Source correspondence controls: ordered writes, scoped locals and every path.
use verum_ast::expr::{ConditionKind, IfCondition};
use verum_ast::{
    BinOp, Block, Expr, ExprKind, Ident, Literal, Path, Pattern, PatternKind, Span, Stmt, Type,
    TypeKind,
};
use verum_common::{Heap, List, Text};
use verum_smt::{
    Context, ContractError, verify_loop_invariant, verify_loop_invariant_with_precondition,
    verify_termination, verify_termination_in_domain,
};

fn int(n: i64) -> Expr {
    Expr::literal(Literal::int(n as i128, Span::dummy()))
}
fn boolean(b: bool) -> Expr {
    Expr::literal(Literal::bool(b, Span::dummy()))
}
fn var(name: &str) -> Expr {
    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}
fn bin(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        Span::dummy(),
    )
}
fn ncmp(op: BinOp, n: i64) -> Expr {
    bin(op, var("n"), int(n))
}
fn write(op: BinOp, n: i64) -> Expr {
    ncmp(op, n)
}
fn block(body: impl IntoIterator<Item = Expr>) -> Block {
    Block::new(
        body.into_iter().map(|e| Stmt::expr(e, true)).collect(),
        None,
        Span::dummy(),
    )
}
fn seq(body: impl IntoIterator<Item = Expr>) -> Expr {
    Expr::new(ExprKind::Block(block(body)), Span::dummy())
}
fn branch(test: Expr, yes: Expr, no: Option<Expr>) -> Expr {
    Expr::new(
        ExprKind::If {
            condition: Heap::new(IfCondition {
                conditions: [ConditionKind::Expr(test)].into_iter().collect(),
                span: Span::dummy(),
            }),
            then_branch: block([yes]),
            else_branch: no.map(Heap::new),
        },
        Span::dummy(),
    )
}
fn variables(names: &[&str]) -> List<(Text, Type)> {
    names
        .iter()
        .map(|n| (Text::from(*n), Type::new(TypeKind::Int, Span::dummy())))
        .collect()
}
fn terminates(body: &Expr, domain: &Expr, exit: &Expr) -> Result<(), ContractError> {
    verify_termination_in_domain(
        &Context::new(),
        &var("n"),
        &variables(&["n"]),
        body,
        domain,
        exit,
    )
    .map(|_| ())
}
fn refuted(result: Result<(), ContractError>, message: &str) {
    match result {
        Err(ContractError::VerificationFailed {
            message: actual, ..
        }) => assert!(actual.contains(message), "{actual}"),
        other => panic!("expected counterexample for {message}, got {other:?}"),
    }
}

#[test]
fn sequential_assignments_read_the_updated_state() {
    let domain = ncmp(BinOp::Ge, 0);
    assert!(
        terminates(
            &seq([write(BinOp::SubAssign, 1), write(BinOp::SubAssign, 1)]),
            &domain,
            &ncmp(BinOp::Lt, 2)
        )
        .is_ok()
    );
    refuted(
        terminates(
            &seq([write(BinOp::SubAssign, 1), write(BinOp::AddAssign, 1)]),
            &domain,
            &ncmp(BinOp::Eq, 0),
        ),
        "rank",
    );
    // The old flat encoding asserted n'=0 AND n'=1 and accepted a constant rank.
    refuted(
        verify_termination(
            &Context::new(),
            &int(100),
            &variables(&["n"]),
            &seq([write(BinOp::Assign, 0), write(BinOp::Assign, 1)]),
        )
        .map(|_| ()),
        "rank",
    );
}

#[test]
fn branches_preserve_both_paths_and_current_guards() {
    let domain = ncmp(BinOp::Ge, 0);
    let exit = ncmp(BinOp::Eq, 0);
    let good = branch(
        ncmp(BinOp::Ge, 2),
        write(BinOp::SubAssign, 2),
        Some(write(BinOp::SubAssign, 1)),
    );
    assert!(terminates(&good, &domain, &exit).is_ok());
    let bad = branch(ncmp(BinOp::Ge, 2), write(BinOp::SubAssign, 2), Some(int(0)));
    refuted(terminates(&bad, &domain, &exit), "rank");
    // The second guard sees n-1. Reading the original n would produce -1 at n=1.
    assert!(
        terminates(
            &seq([
                write(BinOp::SubAssign, 1),
                branch(ncmp(BinOp::Gt, 0), write(BinOp::SubAssign, 1), None)
            ]),
            &domain,
            &exit
        )
        .is_ok()
    );
}

#[test]
fn invariant_checks_real_post_state_and_entry_assumptions() {
    let context = Context::new();
    let vars = variables(&["n"]);
    let entry = ncmp(BinOp::Eq, 0);
    let exit = ncmp(BinOp::Ge, 10);
    let invariant = bin(BinOp::And, ncmp(BinOp::Ge, 0), ncmp(BinOp::Le, 10));
    let body = write(BinOp::AddAssign, 1);
    assert!(
        verify_loop_invariant_with_precondition(
            &context,
            &entry,
            &invariant,
            &vars,
            &body,
            &exit,
            &ncmp(BinOp::Eq, 10)
        )
        .is_ok()
    );
    refuted(
        verify_loop_invariant_with_precondition(
            &context,
            &entry,
            &invariant,
            &vars,
            &write(BinOp::AddAssign, 2),
            &exit,
            &boolean(true),
        )
        .map(|_| ()),
        "violate the invariant",
    );
    refuted(
        verify_loop_invariant(&context, &invariant, &vars, &body, &exit, &boolean(true))
            .map(|_| ()),
        "initially",
    );
    refuted(
        verify_loop_invariant_with_precondition(
            &context,
            &entry,
            &invariant,
            &vars,
            &body,
            &exit,
            &ncmp(BinOp::Eq, 11),
        )
        .map(|_| ()),
        "postcondition",
    );
}

#[test]
fn successor_domain_and_nonnegativity_are_not_assumed() {
    refuted(
        terminates(
            &write(BinOp::SubAssign, 2),
            &ncmp(BinOp::Ge, 0),
            &ncmp(BinOp::Eq, 0),
        ),
        "rank",
    );
    refuted(
        terminates(
            &write(BinOp::SubAssign, 1),
            &ncmp(BinOp::Ge, 10),
            &ncmp(BinOp::Lt, 10),
        ),
        "rank",
    );
    refuted(terminates(&int(0), &boolean(true), &boolean(true)), "rank");
}

fn local(name: &str, value: Expr) -> Stmt {
    Stmt::let_stmt(
        Pattern::new(
            PatternKind::Ident {
                name: Ident::new(name, Span::dummy()),
                mutable: false,
                by_ref: false,
                subpattern: None,
            },
            Span::dummy(),
        ),
        None,
        Some(value),
        Span::dummy(),
    )
}

#[test]
fn local_slots_preserve_lexical_shadowing_and_value_capture() {
    let make = |stmts: List<Stmt>| {
        Expr::new(
            ExprKind::Block(Block::new(stmts, None, Span::dummy())),
            Span::dummy(),
        )
    };
    let capture = make(List::from(vec![
        local("tmp", bin(BinOp::Sub, var("n"), int(1))),
        Stmt::expr(bin(BinOp::Assign, var("n"), var("tmp")), true),
    ]));
    assert!(terminates(&capture, &ncmp(BinOp::Ge, 0), &ncmp(BinOp::Eq, 0)).is_ok());
    let shadow = make(List::from(vec![local("n", int(0))]));
    refuted(
        terminates(&shadow, &ncmp(BinOp::Ge, 0), &ncmp(BinOp::Eq, 0)),
        "rank",
    );
    let mutation = make(List::from(vec![
        local("tmp", int(0)),
        Stmt::expr(bin(BinOp::Assign, var("tmp"), int(1)), true),
    ]));
    assert!(matches!(
        terminates(&mutation, &ncmp(BinOp::Ge, 0), &ncmp(BinOp::Eq, 0)),
        Err(ContractError::InvalidContract(_))
    ));
}

#[test]
fn ordered_updates_preserve_relational_invariants() {
    let invariant = bin(
        BinOp::And,
        ncmp(BinOp::Ge, 0),
        bin(BinOp::Eq, var("x"), int(1)),
    );
    let body = seq([
        bin(BinOp::Assign, var("x"), var("n")),
        write(BinOp::SubAssign, 1),
        bin(BinOp::SubAssign, var("x"), var("n")),
    ]);
    assert!(
        verify_loop_invariant_with_precondition(
            &Context::new(),
            &invariant,
            &invariant,
            &variables(&["n", "x"]),
            &body,
            &ncmp(BinOp::Eq, 0),
            &invariant
        )
        .is_ok()
    );
}

#[test]
fn boolean_assignments_and_block_tails_are_part_of_the_transition() {
    let mut vars = variables(&["n"]);
    vars.push((Text::from("p"), Type::new(TypeKind::Bool, Span::dummy())));
    let mut body = block([bin(BinOp::Assign, var("p"), ncmp(BinOp::Gt, 1))]);
    body.expr = Some(Heap::new(branch(
        var("p"),
        write(BinOp::SubAssign, 2),
        Some(write(BinOp::SubAssign, 1)),
    )));
    assert!(
        verify_termination_in_domain(
            &Context::new(),
            &var("n"),
            &vars,
            &Expr::new(ExprKind::Block(body), Span::dummy()),
            &ncmp(BinOp::Ge, 0),
            &ncmp(BinOp::Eq, 0),
        )
        .is_ok()
    );
    let inner = Expr::new(
        ExprKind::Block(Block::new(
            List::from(vec![local("n", int(0))]),
            None,
            Span::dummy(),
        )),
        Span::dummy(),
    );
    assert!(
        terminates(
            &seq([inner, write(BinOp::SubAssign, 1)]),
            &ncmp(BinOp::Ge, 0),
            &ncmp(BinOp::Eq, 0)
        )
        .is_ok()
    );
}

#[test]
fn intermediate_overflow_and_short_circuit_paths_are_checked() {
    let context = Context::new();
    let verify = |body: &Expr, exit: &Expr| {
        verify_loop_invariant(
            &context,
            &boolean(true),
            &variables(&["n"]),
            body,
            exit,
            &boolean(true),
        )
        .map(|_| ())
    };
    assert!(
        verify(
            &branch(ncmp(BinOp::Lt, i64::MAX), write(BinOp::AddAssign, 1), None),
            &boolean(false)
        )
        .is_ok()
    );
    refuted(
        verify(
            &seq([write(BinOp::AddAssign, 1), write(BinOp::SubAssign, 1)]),
            &boolean(false),
        ),
        "overflow",
    );
    let addition = bin(BinOp::Gt, bin(BinOp::Add, var("n"), int(1)), var("n"));
    assert!(
        verify(
            &int(0),
            &bin(BinOp::And, ncmp(BinOp::Lt, i64::MAX), addition.clone())
        )
        .is_ok()
    );
    refuted(verify(&int(0), &addition), "exit condition can overflow");
}

#[test]
fn unsupported_effects_and_ill_typed_terms_fail_before_solver_success() {
    let call = Expr::new(
        ExprKind::Call {
            func: Heap::new(var("opaque")),
            args: List::new(),
            type_args: List::new(),
        },
        Span::dummy(),
    );
    let nested = Expr::new(
        ExprKind::Loop {
            label: None,
            body: block([]),
            invariants: List::new(),
        },
        Span::dummy(),
    );
    for body in [
        call.clone(),
        nested,
        branch(boolean(false), call, None),
        bin(BinOp::Assign, var("absent"), int(1)),
        bin(BinOp::Assign, var("n"), boolean(true)),
        bin(BinOp::DivAssign, var("n"), int(0)),
        bin(BinOp::And, var("n"), boolean(true)),
    ] {
        let result = verify_loop_invariant(
            &Context::new(),
            &boolean(true),
            &variables(&["n"]),
            &body,
            &boolean(true),
            &boolean(true),
        );
        assert!(
            matches!(result, Err(ContractError::InvalidContract(_))),
            "{result:?}"
        );
    }
    let duplicate = variables(&["n", "n"]);
    assert!(matches!(
        verify_termination(&Context::new(), &int(1), &duplicate, &int(0)),
        Err(ContractError::InvalidContract(_))
    ));
}
