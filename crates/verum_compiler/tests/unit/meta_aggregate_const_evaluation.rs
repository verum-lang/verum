//! Aggregate construction and @const validation preserve attempted evaluation.
use super::*;
use crate::meta::{ConstValue, MetaContext, MetaError};
use verum_ast::decl::{FunctionBody, ItemKind};
use verum_ast::expr::{Expr, ExprKind};
use verum_common::{List, Maybe, Text};
use verum_fast_parser::Parser;

fn evaluate_body(source: &str) -> Result<ConstValue, MetaError> {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!()
    };
    let Maybe::Some(FunctionBody::Block(block)) = &function.body else {
        panic!()
    };
    let expression = Expr::new(ExprKind::Block(block.clone()), block.span);
    let mut context = MetaContext::new();
    let lowered = context.ast_expr_to_meta_expr(&expression)?;
    context.eval_meta_expr(&lowered)
}

#[test]
fn aggregate_elements_preserve_values_and_lexical_bindings() {
    for source in [
        "meta fn probe() -> Int { let value = 41; [value + 1][0] }",
        "meta fn probe() -> Int { let value = 41; (value + 1, true).0 }",
    ] {
        assert_eq!(
            evaluate_body(source).unwrap(),
            ConstValue::Int(42),
            "{source}"
        );
    }
    assert_eq!(
        evaluate_body("meta fn probe() { (1 + 1, [3 + 4]) }").unwrap(),
        ConstValue::Tuple(List::from(vec![
            ConstValue::Int(2),
            ConstValue::Array(List::from(vec![ConstValue::Int(7)]))
        ]))
    );
    assert_eq!(
        evaluate_body("meta fn probe() { [()] }").unwrap(),
        ConstValue::Array(List::from(vec![ConstValue::Unit]))
    );
}

#[test]
fn aggregate_failure_is_not_replaced_by_unit() {
    for source in [
        "meta fn probe() { [1 / (1 - 1)] }",
        "meta fn probe() { (1 / (1 - 1),) }",
        "meta fn probe() { ([1 / (1 - 1)],) }",
    ] {
        assert!(
            matches!(evaluate_body(source), Err(MetaError::DivisionByZero)),
            "{source}"
        );
    }
}

#[test]
fn aggregate_errors_follow_source_order() {
    for source in [
        "meta fn probe() { [true / 2, 1 / 0] }",
        "meta fn probe() { (true / 2, 1 / 0) }",
    ] {
        assert!(
            matches!(evaluate_body(source), Err(MetaError::Other(_))),
            "{source}"
        );
    }
    for source in [
        "meta fn probe() { [1 / 0, true / 2] }",
        "meta fn probe() { (1 / 0, true / 2) }",
    ] {
        assert!(
            matches!(evaluate_body(source), Err(MetaError::DivisionByZero)),
            "{source}"
        );
    }
}

fn isolated_const_child() -> bool {
    const CHILD: &str = "VERUM_TEST_META_AGGREGATE_CONST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("CONST_PHASE_CHECKED"));
    false
}

fn phase_check(source: &str, failures: usize) -> String {
    let module = Parser::new(source).parse_module().unwrap();
    let mut session = Session::new(Default::default());
    let mut pipeline = CompilationPipeline::new(&mut session);
    let owner = Text::from("fixture.const_evaluation");
    pipeline
        .register_meta_declarations(&owner, &module)
        .unwrap();
    let result = pipeline.phase_meta_evaluation(&module, &owner);
    assert_eq!(result.is_err(), failures != 0, "{source}: {result:?}");
    assert_eq!(pipeline.session.error_count(), failures, "{source}");
    pipeline.session.format_diagnostics()
}

#[test]
fn const_phase_reports_other_and_unsupported_errors() {
    if !isolated_const_child() {
        return;
    }
    for source in [
        "meta fn probe() { [1 / (1 - 1)] }",
        "meta fn probe() { (1 / (1 - 1),) }",
        "fn probe() { @const { true / 2 } }",
        "fn probe() { @const { async { 42 } } }",
        "fn probe() { @const { 1 / (1 - 1) } }",
        "fn probe() { @const { [1 / (1 - 1)] } }",
        "fn probe() { @const { (1 / (1 - 1),) } }",
    ] {
        phase_check(source, 1);
    }
    for source in [
        "fn probe() { @const { ((2 + 5) * 6) } }",
        "fn probe() { @const { let value = 41; [value + 1][0] } }",
    ] {
        phase_check(source, 0);
    }
    let quote = phase_check("fn probe() { @const { quote { 1 / 0 } } }", 1);
    assert!(quote.contains("does not splice syntax"), "{quote}");
    assert!(!quote.contains("Division by zero"), "{quote}");
    println!("CONST_PHASE_CHECKED");
}

#[test]
fn const_search_traverses_nested_expressions_without_evaluating_quotes() {
    if !isolated_const_child() {
        return;
    }
    for source in [
        "fn probe() { (@const { 1 / 0 }) }",
        "fn probe() { [@const { 1 / 0 }] }",
        "fn probe() { (@const { 1 / 0 },) }",
        "fn probe() { 1 + @const { 1 / 0 } }",
        "fn probe() { if true { @const { 1 / 0 } } else { 42 } }",
        "fn probe() { return (@const { 1 / 0 }); }",
    ] {
        phase_check(source, 1);
    }
    phase_check("fn probe() { quote { @const { 1 / 0 } } }", 0);
    phase_check("fn probe() { (@const { 1 / 0 }, @const { true / 2 }) }", 2);
    println!("CONST_PHASE_CHECKED");
}

#[test]
fn evaluated_aggregate_construction_obeys_memory_budget() {
    for source in [
        "meta fn probe() { [1 + 1] }",
        "meta fn probe() { (1 + 1,) }",
    ] {
        let module = Parser::new(source).parse_module().unwrap();
        let ItemKind::Function(function) = &module.items[0].kind else {
            panic!()
        };
        let Maybe::Some(FunctionBody::Block(block)) = &function.body else {
            panic!()
        };
        let expression = Expr::new(ExprKind::Block(block.clone()), block.span);
        let mut context = MetaContext::new();
        context.memory_limit = 1;
        let lowered = context.ast_expr_to_meta_expr(&expression).unwrap();
        assert!(
            matches!(
                context.eval_meta_expr(&lowered),
                Err(MetaError::MemoryLimitExceeded { .. })
            ),
            "{source}"
        );
    }
}

fn quote_scalar(source: &str, expected: &str) {
    use verum_ast::expr::TokenTree;
    let ConstValue::Expr(expression) = evaluate_body(source).unwrap() else {
        panic!()
    };
    let ExprKind::Quote { tokens, .. } = expression.kind else {
        panic!()
    };
    assert_eq!(tokens.len(), 1, "{source}: {tokens:?}");
    let TokenTree::Token(token) = &tokens[0] else {
        panic!()
    };
    assert_eq!(token.text.as_str(), expected, "{source}");
}

#[test]
fn quote_splice_aggregate_values_and_bindings_are_preserved() {
    quote_scalar("meta fn probe() { quote { ${[1, 2][0]} } }", "1");
    quote_scalar("meta fn probe() { quote { ${(1, 2).0} } }", "1");
    quote_scalar("meta fn probe() { quote { ${[40 + 2][0]} } }", "42");
    quote_scalar(
        "meta fn probe() { let value = 41; quote { ${[value + 1][0]} } }",
        "42",
    );
}

#[test]
fn ordinary_evaluation_and_splices_short_circuit_inside_aggregates() {
    let condition = "false && (1 / 0 == 0)";
    let source = format!("meta fn probe() {{ [{condition}][0] }}");
    assert_eq!(evaluate_body(&source).unwrap(), ConstValue::Bool(false));
    quote_scalar(
        &format!("meta fn probe() {{ quote {{ ${{[{condition}][0]}} }} }}"),
        "false",
    );
}

#[test]
fn arithmetic_spines_lower_and_evaluate_on_an_ordinary_stack() {
    let sum = std::iter::repeat_n("1", 96).collect::<List<_>>().join(" + ");
    let source = format!("meta fn probe() {{ [{sum}][0] }}");
    assert_eq!(evaluate_body(&source).unwrap(), ConstValue::Int(96));
    let source = format!("meta fn probe() {{ [({sum}) / 0][0] }}");
    assert!(matches!(evaluate_body(&source), Err(MetaError::DivisionByZero)));
}

#[test]
fn quote_splice_aggregate_errors_follow_source_order() {
    for source in [
        "meta fn probe() { quote { ${[1 / (1 - 1)][0]} } }",
        "meta fn probe() { quote { ${(1 / (1 - 1),).0} } }",
        "meta fn probe() { quote { ${[1 / 0, true / 2][0]} } }",
    ] {
        assert!(
            matches!(evaluate_body(source), Err(MetaError::DivisionByZero)),
            "{source}"
        );
    }
    assert!(matches!(
        evaluate_body("meta fn probe() { quote { ${[true / 2, 1 / 0][0]} } }"),
        Err(MetaError::Other(_))
    ));
}

#[test]
fn quote_splice_aggregate_construction_obeys_memory_budget() {
    let module = Parser::new("meta fn probe() { quote { ${[1, 2][0]} } }")
        .parse_module()
        .unwrap();
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!()
    };
    let Maybe::Some(FunctionBody::Block(block)) = &function.body else {
        panic!()
    };
    let expression = Expr::new(ExprKind::Block(block.clone()), block.span);
    let mut context = MetaContext::new();
    context.memory_limit = 1;
    let lowered = context.ast_expr_to_meta_expr(&expression).unwrap();
    assert!(matches!(
        context.eval_meta_expr(&lowered),
        Err(MetaError::MemoryLimitExceeded { .. })
    ));
}

#[test]
fn quote_splice_aggregate_actual_phase_controls() {
    if !isolated_const_child() {
        return;
    }
    phase_check("meta fn probe() { quote { ${[1, 2][0]} } }", 0);
    let runtime = phase_check("fn probe() { @const { quote { ${[40 + 2][0]} } } }", 1);
    assert!(runtime.contains("does not splice syntax"), "{runtime}");
    for source in [
        "meta fn probe() { quote { ${[1 / (1 - 1)][0]} } }",
        "fn probe() { @const { quote { ${(1 / (1 - 1),).0} } } }",
    ] {
        phase_check(source, 1);
    }
    println!("CONST_PHASE_CHECKED");
}
