//! Checked array-count expressions use the producer's scalar domain.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        match &item.kind {
            ItemKind::Function(function) => checker
                .register_function_signature(function)
                .expect("signature"),
            ItemKind::Const(decl) => checker.pre_register_const(decl),
            _ => (),
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

#[test]
fn pure_conditional_counts_and_lazy_branches_typecheck() {
    for count in [
        "if 2 > 1 {3} else {4}",
        "if false {1 / 0} else {4}",
        "if false && 1 / 0 > 0 {1} else {4}",
        "if true || 1 / 0 > 0 {3} else {4}",
        "{let n = 2; if n == 2 {n + 1} else {4}}",
        "if (true) {if false {9} else {3}} else {4}",
    ] {
        let source =
            format!("fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}");
        let result = errors(&source);
        assert!(result.is_empty(), "{count}: {result:?}");
    }
}

#[test]
fn checked_counts_refuse_executed_invalid_or_unsupported_expressions() {
    for count in [
        "if true {1 / 0} else {4}",
        "if true {-1} else {4}",
        "if true {9223372036854775807 + 1 - 1} else {4}",
        "if true {1 << 63 >> 63} else {4}",
        "if 1 {3} else {4}",
        "if false {3}",
        "{let mut n = 2; n = 3; n}",
    ] {
        let source =
            format!("fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}");
        assert!(
            !errors(&source).is_empty(),
            "invalid count accepted: {count}"
        );
    }
}

#[test]
fn named_checked_counts_keep_invalid_intermediates() {
    for initializer in [
        "9223372036854775807 + 1 - 1",
        "if true {1 << 63 >> 63} else {4}",
    ] {
        let source = format!(
            "const N: Int = {initializer}; fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; N]>()}}"
        );
        assert!(
            !errors(&source).is_empty(),
            "invalid named count accepted: {initializer}"
        );
    }
}

#[test]
fn checked_count_domain_does_not_narrow_rich_meta_arithmetic() {
    let ast = Parser::new("9223372036854775807 + 1 - 1")
        .parse_expr()
        .expect("expression");
    let mut evaluator = verum_types::const_eval::ConstEvaluator::new();
    assert_eq!(
        evaluator.eval(&ast).unwrap().as_i128(),
        Some(i64::MAX as i128)
    );
    assert!(evaluator.eval_array_count(&ast, "cog").is_err());
}

#[test]
fn conditional_constants_keep_their_source_owner() {
    let source = "module origin { const BASE: Int = 3; public const SIZE: Int = if BASE > 0 {{let extra = 1; BASE + extra}} else {8}; } const BASE: Int = 7; fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; origin.SIZE]>()}";
    let result = errors(source);
    assert!(result.is_empty(), "{result:?}");
}

#[test]
fn checked_named_conditions_are_lazy() {
    for prefix in [
        "const BASE: Int = 2; const N: Int = if BASE > 0 {BASE + 1} else {4};",
        "const FLAG: Bool = true; const N: Int = if FLAG {3} else {1 / 0};",
        "const N: Int = if false && 1 / 0 > 0 {9} else {3};",
    ] {
        let source =
            format!("{prefix} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; N]>()}}");
        let result = errors(&source);
        assert!(result.is_empty(), "{source}: {result:?}");
    }
}

#[test]
fn shared_scalar_evaluation_distinguishes_refusal_from_invalid_and_unknown() {
    use verum_ast::checked_const::{EvalError, Scalar, evaluate};
    let eval = |source: &str| {
        evaluate(
            &Parser::new(source).parse_expr().unwrap(),
            0,
            &mut |_, _| Ok::<_, ()>(None),
        )
    };
    assert!(matches!(eval("loop {}"), Err(EvalError::Unsupported(_))));
    assert!(matches!(
        eval("missing_value"),
        Err(EvalError::Unresolved(_))
    ));
    assert!(matches!(
        eval("1 / 0"),
        Err(EvalError::InvalidArithmetic(_))
    ));
    assert!(matches!(
        eval("if 1 {3} else {4}"),
        Err(EvalError::InvalidType(_))
    ));
    assert!(matches!(
        eval("-9223372036854775808 / -1"),
        Err(EvalError::InvalidArithmetic(_))
    ));
    assert_eq!(eval("-9223372036854775808").unwrap(), Scalar::Int(i64::MIN));
    let literal = Parser::new("1").parse_expr().unwrap();
    assert!(matches!(
        evaluate(&literal, 128, &mut |_, _| Ok::<_, ()>(None)),
        Err(EvalError::Limit(_))
    ));
    let source = format!("{{{} 3}}", "1;".repeat(4097));
    assert!(matches!(eval(&source), Err(EvalError::Limit(_))));
}

#[test]
fn unselected_branches_and_short_circuits_never_resolve_calls() {
    use verum_ast::checked_const::{Scalar, evaluate};
    for source in [
        "if false {side_effect()} else {3}",
        "if false && side_effect() {9} else {3}",
        "if true || side_effect() {3} else {9}",
    ] {
        let expr = Parser::new(source).parse_expr().unwrap();
        let value = evaluate(&expr, 0, &mut |_, _| -> Result<Option<Scalar>, ()> {
            panic!("dead branch reached resolver")
        })
        .unwrap();
        assert_eq!(value, Scalar::Int(3));
    }
}

#[test]
fn unknown_owners_and_arbitrary_calls_do_not_supply_counts() {
    for count in ["missing.N", "if true {ordinary(3)} else {4}"] {
        let source = format!(
            "fn ordinary(value: Int)->Int {{value}} fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}"
        );
        assert!(!errors(&source).is_empty(), "{count}");
    }
    let source = "fn ordinary(value: Int)->Int {value} const N: Int = ordinary(3); fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; N]>()}";
    assert!(!errors(source).is_empty());
}

#[test]
fn block_local_projections_never_reach_global_declaration_lookup() {
    use verum_ast::checked_const::{EvalError, Scalar, evaluate};
    for source in [
        "{let Foo = 3; Foo.size}",
        "{let Foo = 3; offset_of(Foo, field)}",
        "{let Foo = 3; ([Foo; 2]).size}",
    ] {
        let expr = Parser::new(source).parse_expr().unwrap();
        let result = evaluate(&expr, 0, &mut |_, _| -> Result<Option<Scalar>, ()> {
            panic!("local projection reached global resolver: {source}")
        });
        assert!(
            matches!(result, Err(EvalError::Unsupported(_))),
            "{source}: {result:?}"
        );
    }
}
