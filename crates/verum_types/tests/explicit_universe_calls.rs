//! Actual source admission for universe, value and type argument roles.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source)
        .parse_module()
        .expect("valid source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("explicit_levels");
    checker.set_universe_poly_enabled(true);
    let mut result: List<Text> = checker
        .prepare_module_declarations(&module)
        .into_iter()
        .map(|e| format!("{e:?}").into())
        .collect();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            result.push(format!("{e:?}").into());
        }
    }
    result.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    result.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}").into()),
    );
    result
}
fn accept(source: &str) {
    let errors = errors(source);
    assert!(errors.is_empty(), "{errors:?}");
}
fn reject_level(source: &str) {
    let errors = errors(source);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Universe") || e.contains("universe") || e.contains("level")),
        "expected universe refusal, not an unrelated error: {errors:?}"
    );
}
const RETAIN: &str = "fn retain<universe u>(value:Type(u))->Type(u){value}";
#[test]
fn explicit_concrete_levels_are_independent_per_call() {
    accept(&format!(
        "{RETAIN} fn one(value:Type(1))->Type(1){{retain<1>(value)}} fn two(value:Type(2))->Type(2){{retain<2>(value)}}"
    ));
}
#[test]
fn explicit_level_must_agree_with_actual_argument() {
    reject_level(&format!(
        "{RETAIN} fn bad(value:Type(1))->Type(1){{retain<2>(value)}}"
    ));
}
#[test]
fn explicit_level_must_agree_with_declared_result() {
    reject_level(&format!(
        "{RETAIN} fn bad(value:Type(2))->Type(1){{retain<2>(value)}}"
    ));
}
#[test]
fn level_hole_is_inferred_from_the_same_call() {
    accept(&format!(
        "{RETAIN} fn good(value:Type(2))->Type(2){{retain<_>(value)}}"
    ));
    reject_level(&format!(
        "{RETAIN} fn bad(value:Type(2))->Type(1){{retain<_>(value)}}"
    ));
}
#[test]
fn caller_owned_level_forwards_without_becoming_concrete() {
    accept(&format!(
        "{RETAIN} fn good<universe v>(value:Type(v))->Type(v){{retain<v>(value)}}"
    ));
    reject_level(&format!(
        "{RETAIN} fn bad<universe v>(value:Type(v))->Type(0){{retain<v>(value)}}"
    ));
}
#[test]
fn ordinary_type_and_meta_binders_cannot_supply_a_level() {
    for header in ["u", "u:meta Int"] {
        reject_level(&format!(
            "{RETAIN} fn bad<{header}>(value:Type(1))->Type(1){{retain<u>(value)}}"
        ));
    }
}
#[test]
fn nested_type_shadow_does_not_capture_an_outer_level() {
    reject_level(&format!(
        "{RETAIN} fn outer<universe u>(){{fn inner<u>(value:Type(1))->Type(1){{retain<u>(value)}}}}"
    ));
}
#[test]
fn type_and_value_roles_remain_separate_after_a_level() {
    let source = "type Cell<N:meta Int> is {value:()}; fn mixed<universe u,N:meta Int,T>(level:Type(u),cell:Cell<N>,value:T)->T{value}";
    accept(&format!(
        "{source} fn good(level:Type(1),cell:Cell<7>)->Bool{{mixed<1,7,Bool>(level,cell,true)}}"
    ));
    let bad = errors(&format!(
        "{source} fn bad(level:Type(1),cell:Cell<7>)->Bool{{mixed<1,8,Bool>(level,cell,true)}}"
    ));
    assert!(
        bad.iter()
            .any(|e| e.contains("Mismatch") || e.contains("exact value")),
        "{bad:?}"
    );
}
#[test]
fn inferred_level_does_not_consume_the_explicit_value_slot() {
    accept(
        "fn size<universe u,N:meta Int>(values:[Int;2])->Int{N} fn call(values:[Int;2])->Int{size<_,7>(values)}",
    );
}
#[test]
fn closed_arithmetic_is_checked_as_a_level() {
    accept(&format!(
        "{RETAIN} fn good(value:Type(2))->Type(2){{retain<1+1>(value)}}"
    ));
    for argument in ["-1", "4294967296", "4294967295+1", "1/0"] {
        reject_level(&format!(
            "{RETAIN} fn bad(value:Type(1))->Type(1){{retain<{argument}>(value)}}"
        ));
    }
}
#[test]
fn non_integer_and_type_arguments_cannot_be_reinterpreted_as_levels() {
    for argument in ["true", "1.0", "\"1\"", "Int", "Type(0)", "'static"] {
        reject_level(&format!(
            "{RETAIN} fn bad(value:Type(1))->Type(1){{retain<{argument}>(value)}}"
        ));
    }
}
#[test]
fn explicit_level_preserves_callable_universe_bounds() {
    let source = "fn apply<universe u,F:fn()->Type(u)>(f:F)->Type(u){f()}";
    accept(&format!(
        "{source} fn good(f:fn()->Type(1))->Type(1){{apply<1,fn()->Type(1)>(f)}}"
    ));
    reject_level(&format!(
        "{source} fn bad(f:fn()->Type(2))->Type(1){{apply<1,fn()->Type(2)>(f)}}"
    ));
}
