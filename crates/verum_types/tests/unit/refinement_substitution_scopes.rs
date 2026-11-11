use super::{RefinementChecker, RefinementConfig};
use crate::const_eval::ConstEvaluator;
use verum_ast::{Expr, ExprKind, PatternKind};
use verum_common::ConstValue;
use verum_fast_parser::Parser;

fn rewrite(source: &str, value: &str) -> Expr {
    let source = Parser::new(source).parse_expr().expect("source grammar");
    let value = Parser::new(value).parse_expr().expect("argument grammar");
    RefinementChecker::new(RefinementConfig::default())
        .substitute_all_in_expr(&source, &[("it".into(), &value)])
        .expect("checked substitution")
}

fn name(expr: &Expr) -> &str {
    let ExprKind::Path(path) = &expr.kind else {
        panic!("{expr:?}")
    };
    path.as_ident().expect("unqualified variable").name.as_str()
}

fn evaluate(source: &str, value: &str) -> ConstValue {
    let source = Parser::new(source).parse_expr().expect("source grammar");
    let value = Parser::new(value).parse_expr().expect("argument grammar");
    let checker = RefinementChecker::new(RefinementConfig::default());
    let rewritten = checker
        .substitute_all_in_expr(&source, &[("it".into(), &value)])
        .expect("checked substitution");
    let mut evaluator = ConstEvaluator::new();
    evaluator.bind("outside", ConstValue::Int(7));
    evaluator
        .eval(&rewritten)
        .unwrap_or_else(|error| panic!("{error:?}: {rewritten:?}"))
}

#[test]
fn a_let_initializer_reads_the_outer_subject_before_shadowing_it() {
    assert_eq!(evaluate("{ let it = it; it }", "7"), ConstValue::Int(7));
}

#[test]
fn a_local_cannot_capture_a_free_variable_of_the_argument() {
    assert_eq!(
        evaluate("{ let outside = 0; it }", "outside"),
        ConstValue::Int(7)
    );
}

#[test]
fn nested_locals_keep_their_own_uses_and_do_not_capture_the_argument() {
    assert_eq!(
        evaluate(
            "{ let outside = 0; { let outside = 1; it + outside } + outside }",
            "outside"
        ),
        ConstValue::Int(8)
    );
}

#[test]
fn a_local_subject_is_not_replaced_by_the_outer_argument() {
    assert_eq!(
        evaluate("{ let it = 3; it }", "outside"),
        ConstValue::Int(3)
    );
}

#[test]
fn record_shorthand_in_an_argument_is_a_free_variable() {
    let rewritten = rewrite("{let outside = 0; it}", "Pair {outside}");
    let ExprKind::Block(block) = &rewritten.kind else {
        panic!()
    };
    let verum_ast::StmtKind::Let { pattern, .. } = &block.stmts[0].kind else {
        panic!()
    };
    let PatternKind::Ident { name: bound, .. } = &pattern.kind else {
        panic!()
    };
    assert_ne!(bound.name.as_str(), "outside");
    let ExprKind::Record { fields, .. } = &block.expr.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(fields[0].name.name.as_str(), "outside");
    assert!(fields[0].value.is_none());
}

#[test]
fn generated_names_do_not_capture_record_shorthand_in_the_predicate() {
    let rewritten = rewrite(
        "{let outside = 0; (it, Pair {__refinement_local_0})}",
        "outside",
    );
    let ExprKind::Block(block) = &rewritten.kind else {
        panic!()
    };
    let verum_ast::StmtKind::Let { pattern, .. } = &block.stmts[0].kind else {
        panic!()
    };
    let PatternKind::Ident { name: bound, .. } = &pattern.kind else {
        panic!()
    };
    assert_ne!(bound.name.as_str(), "__refinement_local_0");
}

#[test]
fn match_bindings_preserve_outer_arguments() {
    let rewritten = rewrite("match 3 { outside => it + outside }", "outside");
    let ExprKind::Match {
        expr: scrutinee,
        arms,
    } = &rewritten.kind
    else {
        panic!()
    };
    let PatternKind::Ident { name: bound, .. } = &arms[0].pattern.kind else {
        panic!()
    };
    let ExprKind::Binary { left, right, .. } = &arms[0].body.kind else {
        panic!()
    };
    assert_eq!(
        ConstEvaluator::new().eval(scrutinee).unwrap(),
        ConstValue::Int(3)
    );
    assert_ne!(bound.name.as_str(), "outside");
    assert_eq!(name(left), "outside");
    assert_eq!(name(right), bound.name.as_str());
}

#[test]
fn a_closure_renames_its_parameter_without_changing_the_inserted_value() {
    let rewritten = rewrite("|outside: Int| it + outside", "outside");
    let ExprKind::Closure { params, body, .. } = &rewritten.kind else {
        panic!("{rewritten:?}")
    };
    let PatternKind::Ident { name: bound, .. } = &params[0].pattern.kind else {
        panic!()
    };
    let ExprKind::Binary { left, right, .. } = &body.kind else {
        panic!()
    };
    assert_ne!(bound.name.as_str(), "outside");
    assert_eq!(name(left), "outside");
    assert_eq!(name(right), bound.name.as_str());
}

#[test]
fn a_quantifier_keeps_its_domain_outside_and_its_guard_inside_the_binding() {
    let rewritten = rewrite(
        "forall outside: Int in it where outside > 0 . it == outside",
        "outside",
    );
    let ExprKind::Forall { bindings, body } = &rewritten.kind else {
        panic!("{rewritten:?}")
    };
    let binding = &bindings[0];
    let PatternKind::Ident { name: bound, .. } = &binding.pattern.kind else {
        panic!()
    };
    assert_ne!(bound.name.as_str(), "outside");
    assert_eq!(name(binding.domain.as_ref().unwrap()), "outside");
    let ExprKind::Binary { left, .. } = &binding.guard.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(name(left), bound.name.as_str());
    let ExprKind::Binary { left, right, .. } = &body.kind else {
        panic!()
    };
    assert_eq!(name(left), "outside");
    assert_eq!(name(right), bound.name.as_str());
}

#[test]
fn explicit_value_arguments_are_substituted_and_type_arguments_are_retained() {
    use verum_ast::{GenericArg, Ident, Span, Type};
    use verum_common::List;
    let span = Span::dummy();
    let source = Expr::new(
        ExprKind::Call {
            func: Box::new(Expr::ident(Ident::new("callee", span))),
            type_args: [
                GenericArg::Type(Type::int(span)),
                GenericArg::Const(Parser::new("it + 1").parse_expr().unwrap().into()),
            ]
            .into_iter()
            .collect(),
            args: List::new(),
        },
        span,
    );
    let value = Parser::new("7").parse_expr().unwrap();
    let checker = RefinementChecker::new(RefinementConfig::default());
    let rewritten = checker
        .substitute_all_in_expr(&source, &[("it".into(), &value)])
        .unwrap();
    let ExprKind::Call { type_args, .. } = &rewritten.kind else {
        panic!()
    };
    assert_eq!(type_args[0], GenericArg::Type(Type::int(span)));
    let GenericArg::Const(expression) = &type_args[1] else {
        panic!()
    };
    assert_eq!(
        ConstEvaluator::new().eval(expression).unwrap(),
        ConstValue::Int(8)
    );
}

#[test]
fn a_type_refinements_own_subject_is_not_an_outer_dependency() {
    let rewritten = rewrite("{let local: Int {it > 0} = it; local}", "7");
    let ExprKind::Block(block) = &rewritten.kind else {
        panic!()
    };
    let verum_ast::StmtKind::Let {
        ty: Some(ty),
        value: Some(value),
        ..
    } = &block.stmts[0].kind
    else {
        panic!()
    };
    let verum_ast::TypeKind::Refined { predicate, .. } = &ty.kind else {
        panic!()
    };
    let ExprKind::Binary { left, .. } = &predicate.expr.kind else {
        panic!()
    };
    assert_eq!(name(left), "it");
    assert_eq!(
        ConstEvaluator::new().eval(value).unwrap(),
        ConstValue::Int(7)
    );
}

#[test]
fn an_explicit_type_binder_does_not_hide_a_dependency_on_outer_self() {
    use verum_ast::{Ident, StmtKind, TypeKind};
    let mut source = Parser::new("{let local: Int {self > 0} = 1; local}")
        .parse_expr()
        .unwrap();
    let ExprKind::Block(block) = &mut source.kind else {
        panic!()
    };
    let StmtKind::Let { ty: Some(ty), .. } = &mut block.stmts[0].kind else {
        panic!()
    };
    let TypeKind::Refined { predicate, .. } = &mut ty.kind else {
        panic!()
    };
    predicate.binding = Some(Ident::new("local_subject", ty.span));
    let value = Parser::new("7").parse_expr().unwrap();
    let checker = RefinementChecker::new(RefinementConfig::default());
    let error = checker
        .substitute_all_in_expr(&source, &[("self".into(), &value)])
        .unwrap_err();
    assert!(
        error.message.contains("dependent type annotation"),
        "{error:?}"
    );
}

#[test]
fn an_unsupported_scope_returns_an_error_instead_of_an_unchanged_expression() {
    let source = Parser::new("{fn nested() -> Int {it} nested()}")
        .parse_expr()
        .unwrap();
    let value = Parser::new("7").parse_expr().unwrap();
    let checker = RefinementChecker::new(RefinementConfig::default());
    let error = checker
        .substitute_all_in_expr(&source, &[("it".into(), &value)])
        .unwrap_err();
    assert!(error.message.contains("local declaration"), "{error:?}");
}

#[test]
fn named_predicates_apply_to_the_actual_value() {
    use super::RefinementPredicate;
    use verum_ast::{Ident, Path, Span};
    let span = Span::dummy();
    let predicate = RefinementPredicate::named(Path::single(Ident::new("positive", span)), span);
    let value = Parser::new("7").parse_expr().unwrap();
    let checker = RefinementChecker::new(RefinementConfig::default());
    let condition = checker.generate_vc(&value, &predicate).unwrap().condition;
    let ExprKind::Call { func, args, .. } = condition.kind else {
        panic!()
    };
    assert_eq!(name(&func), "positive");
    assert_eq!(
        ConstEvaluator::new().eval(&args[0]).unwrap(),
        ConstValue::Int(7)
    );
}
