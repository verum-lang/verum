//! Hidden callable computations contribute to the checked contract (T2738).
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

fn check(source: &str) -> List<Text> {
    let module = FastParser::new().parse_module_str(source, FileId::new(0)).expect("production grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    let mut errors: List<Text> = checker.prepare_module_declarations(&module)
        .iter().map(|error| format!("{error:?}").into()).collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) { errors.push(format!("{error:?}").into()); }
    }
    errors.extend(checker.take_deferred_errors().iter().map(|error| format!("{error:?}").into()));
    errors.extend(checker.diagnostic_sources().iter().map(|error| format!("{error:?}").into()));
    errors
}

#[test]
fn pure_body_cannot_hide_an_unknown_default() {
    let errors = check(r#"
        fn leaf(value:Int=@intrinsic("syscall0",73))->Int {value}
        pure fn caller()->Int {leaf()}
    "#);
    assert!(errors.iter().any(|error| error.contains("ImpurePureFunction")), "{errors:#?}");
}

#[test]
fn a_local_wrapper_retains_the_default_computation() {
    let errors = check(r#"
        fn leaf(value:Int=@intrinsic("syscall0",73))->Int {value}
        fn wrapper()->Int {leaf()}
        pure fn caller()->Int {wrapper()}
    "#);
    assert!(errors.iter().any(|error| error.contains("ImpurePureFunction")), "{errors:#?}");
}

#[test]
fn synchronous_callback_cannot_hide_an_unknown_default() {
    let errors = check(r#"
        fn leaf(value:Int=@intrinsic("syscall0",73))->Int {value}
        fn wrapper()->Int {leaf()}
        fn invoke<F:fn()->Int>(operation:F)->Int {operation()}
        fn caller()->Int {invoke(wrapper)}
    "#);
    assert!(errors.iter().any(|error| error.contains("Mismatch")), "{errors:#?}");
}

#[test]
fn a_scalar_default_preserves_the_finite_checked_bound() {
    let errors = check("fn leaf(value:Int=73)->Int {value} pure fn caller()->Int {leaf()}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn an_explicit_regular_operand_does_not_change_the_leaf_body() {
    let errors = check("fn leaf(value:Int)->Int {value} pure fn caller()->Int {leaf(73)}");
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn precondition_computation_is_not_replaced_by_the_body_footprint() {
    let errors = check(r#"
        fn leaf()->Int requires @intrinsic("syscall0",73) > 0 {73}
        pure fn caller()->Int {leaf()}
    "#);
    assert!(errors.iter().any(|error| error.contains("ImpurePureFunction")), "{errors:#?}");
}

#[test]
fn postcondition_computation_is_not_replaced_by_the_body_footprint() {
    let errors = check(r#"
        fn leaf()->Int ensures @intrinsic("syscall0",73) > 0 {73}
        pure fn caller()->Int {leaf()}
    "#);
    assert!(errors.iter().any(|error| error.contains("ImpurePureFunction")), "{errors:#?}");
}
