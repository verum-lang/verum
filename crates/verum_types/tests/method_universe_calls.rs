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
const INSTANCE: &str = "type Holder<T> is {value:T}; implement<T> Holder<T>{fn retain<universe u>(&self,value:Type(u))->Type(u){value}}";
const STATIC: &str = "type Holder<T> is {value:T}; implement<T> Holder<T>{fn retain<universe u>(value:Type(u))->Type(u){value}}";
#[test]
fn instance_levels_instantiate_independently() {
    accept(&format!(
        "{INSTANCE} fn one(h:&Holder<Int>,x:Type(1))->Type(1){{h.retain<1>(x)}} fn two(h:&Holder<Int>,x:Type(2))->Type(2){{h.retain<2>(x)}}"
    ));
}
#[test]
fn static_levels_instantiate_independently() {
    accept(&format!(
        "{STATIC} fn one(x:Type(1))->Type(1){{Holder<Int>.retain<1>(x)}} fn two(x:Type(2))->Type(2){{Holder<Int>.retain<2>(x)}}"
    ));
}
#[test]
fn method_level_cannot_disagree_with_argument_or_result() {
    for tail in [
        "fn bad(h:&Holder<Int>,x:Type(2))->Type(2){h.retain<1>(x)}",
        "fn bad(h:&Holder<Int>,x:Type(2))->Type(1){h.retain<2>(x)}",
    ] {
        reject_level(&format!("{INSTANCE} {tail}"));
    }
}
#[test]
fn explicit_hole_and_inferred_method_levels_keep_their_obligation() {
    for call in ["h.retain<_>(x)", "h.retain(x)"] {
        accept(&format!(
            "{INSTANCE} fn good(h:&Holder<Int>,x:Type(2))->Type(2){{{call}}}"
        ));
        reject_level(&format!(
            "{INSTANCE} fn bad(h:&Holder<Int>,x:Type(2))->Type(1){{{call}}}"
        ));
    }
}
#[test]
fn caller_owned_method_level_stays_rigid() {
    accept(&format!(
        "{INSTANCE} fn good<universe v>(h:&Holder<Int>,x:Type(v))->Type(v){{h.retain<v>(x)}}"
    ));
    reject_level(&format!(
        "{INSTANCE} fn bad<universe v>(h:&Holder<Int>,x:Type(v))->Type(0){{h.retain<v>(x)}}"
    ));
}
#[test]
fn type_or_value_binder_cannot_supply_method_level() {
    for kind in ["u", "u:meta Int"] {
        reject_level(&format!(
            "{INSTANCE} fn bad<{kind}>(h:&Holder<Int>,x:Type(1))->Type(1){{h.retain<u>(x)}}"
        ));
    }
}
#[test]
fn local_level_shadows_owner_type_or_value_without_changing_receiver() {
    for (owner, argument) in [("N", "Int"), ("N:meta Int", "7")] {
        accept(&format!(
            "type Holder<{owner}> is {{value:()}}; implement<{owner}> Holder<N>{{fn retain<universe N>(&self,x:Type(N))->Type(N){{x}}}} fn good(h:&Holder<{argument}>,x:Type(2))->Type(2){{h.retain<2>(x)}}"
        ));
    }
}
#[test]
fn method_bound_keeps_the_owned_level() {
    let prefix = "type Runner is {value:()}; implement Runner{fn apply<universe u,F:fn()->Type(u)>(&self,f:F)->Type(u){f()}}";
    accept(&format!(
        "{prefix} fn good(r:&Runner,f:fn()->Type(1))->Type(1){{r.apply<1,fn()->Type(1)>(f)}}"
    ));
    reject_level(&format!(
        "{prefix} fn bad(r:&Runner,f:fn()->Type(2))->Type(1){{r.apply<1,fn()->Type(2)>(f)}}"
    ));
}
