//! Actual meta evaluation must preserve values and report attempted failures.
use super::*;
use crate::meta::{ConstValue, MetaContext, MetaError};
use verum_ast::decl::{FunctionBody, ItemKind};
use verum_ast::expr::{Expr, ExprKind};
use verum_common::{Maybe, Text};
use verum_fast_parser::Parser;

fn evaluate_body(source: &str) -> Result<ConstValue, MetaError> {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("fixture must parse as a function");
    };
    assert!(function.is_meta);
    let Maybe::Some(FunctionBody::Block(block)) = &function.body else {
        panic!("fixture must have a block body");
    };
    let mut context = MetaContext::new();
    let expression = Expr::new(ExprKind::Block(block.clone()), block.span);
    let lowered = context.ast_expr_to_meta_expr(&expression)?;
    context.eval_meta_expr(&lowered)
}

#[test]
fn parenthesized_meta_expressions_preserve_value_and_exact_error() {
    assert_eq!(
        evaluate_body("meta fn probe() -> Int { ((2 + 5) * 6) }").unwrap(),
        ConstValue::Int(42)
    );
    assert!(matches!(
        evaluate_body("meta fn probe() -> Int { 1 / (1 - 1) }"),
        Err(MetaError::DivisionByZero)
    ));
}

#[test]
fn unsupported_evaluation_is_not_an_implicit_quote() {
    for source in [
        "meta fn probe() { async { 42 } }",
        "meta fn probe() { [42; 2] }",
        "meta fn probe() { let mut a = 1; let mut b = 2; (a, b) = (2, 3); a }",
    ] {
        assert!(
            matches!(
                evaluate_body(source),
                Err(MetaError::MetaEvaluationFailed { .. })
            ),
            "unsupported evaluation must refuse: {source}"
        );
    }
    assert!(matches!(
        evaluate_body("meta fn probe() { quote { 1 / 0 } }"),
        Ok(ConstValue::Expr(Expr {
            kind: ExprKind::Quote { .. },
            ..
        }))
    ));
    assert!(matches!(
        evaluate_body("meta fn probe() -> Int { true / 2 }"),
        Err(MetaError::Other(_))
    ));
    assert!(matches!(
        evaluate_body("meta fn probe() { let mut value = 1; value += 1; value }"),
        Err(MetaError::Other(_))
    ));
}

#[test]
fn zero_argument_phase_reports_all_errors_and_preserves_explicit_quote() {
    const CHILD: &str = "VERUM_TEST_META_EVALUATION_FAILURES_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let thread = std::thread::current();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                thread.name().unwrap(),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env_remove("VERUM_META_ENGINE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("META_FAILURE_PHASE_CHECKED"));
        return;
    }
    for (source, fails) in [
        ("meta fn probe() -> Int { ((2 + 5) * 6) }", false),
        ("meta fn probe() { quote { 1 / 0 } }", false),
        ("meta fn probe() -> Int { 1 / (1 - 1) }", true),
        ("meta fn probe() -> Int { true / 2 }", true),
        ("meta fn probe() { async { 42 } }", true),
        ("meta fn probe() { [42; 2] }", true),
        (
            "meta fn probe() { let mut value = 1; value += 1; value }",
            true,
        ),
        (
            "meta fn probe() { let mut a = 1; let mut b = 2; (a, b) = (2, 3); a }",
            true,
        ),
    ] {
        let module = Parser::new(source).parse_module().unwrap();
        let mut session = Session::new(Default::default());
        let mut pipeline = CompilationPipeline::new(&mut session);
        let owner = Text::from("fixture.meta_failures");
        pipeline
            .register_meta_declarations(&owner, &module)
            .unwrap();
        let result = pipeline.phase_meta_evaluation(&module, &owner);
        assert_eq!(result.is_err(), fails, "{source}: {result:?}");
        assert_eq!(
            pipeline.session.error_count(),
            usize::from(fails),
            "{source}"
        );
    }
    println!("META_FAILURE_PHASE_CHECKED");
}
