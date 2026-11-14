//! Protocol conformance compares declared refinement domains, not source offsets.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source).parse_module().expect("valid grammar");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_builtins();
    checker.set_current_module_path("sample");
    let mut found = checker.prepare_module_declarations(&module);
    found.extend(
        module
            .items
            .iter()
            .filter_map(|item| checker.check_item(item).err()),
    );
    found.extend(checker.take_deferred_errors());
    found
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| format!("{error:?}").into())
        .collect()
}
fn source(expected: &str, actual: &str) -> Text {
    format!("module sample; type Contract is protocol {{fn accept(value:{expected})->Unit;}}; type Worker is (); implement Contract for Worker {{fn accept(value:{actual})->Unit {{}}}}").into()
}
fn accept(source: &str) {
    let found = errors(source);
    assert!(found.is_empty(), "{source}\n{found:#?}");
}
fn reject(source: &str) {
    let found = errors(source);
    assert!(
        found.iter().any(|e| e.contains("E427")),
        "{source}\n{found:#?}"
    );
}

#[test]
fn identical_refinement_domains_ignore_method_source_positions() {
    for ty in [
        "Int{>=0}",
        "UInt8{>=0}",
        "(Int{>=0},Int)",
        "&(Int{>=0})",
        "fn(Int{>=0})->Int",
    ] {
        accept(&source(ty, ty));
    }
}
#[test]
fn implicit_subject_spellings_denote_the_same_declared_domain() {
    accept(&source("Int{it>=0}", "Int{self>=0}"));
}
#[test]
fn differing_bounds_operators_and_literal_domains_still_refuse() {
    for (expected, actual) in [
        ("Int{>=0}", "Int{>0}"),
        ("Int{>0}", "Int{>=0}"),
        ("Int{>=0}", "Int{>=1}"),
        ("Int{>=0}", "UInt8{>=0}"),
        ("Int{>=0}", "Int{>=0u8}"),
        ("Int{>=0}", "Int{>=0.0}"),
    ] {
        reject(&source(expected, actual));
    }
}
#[test]
fn a_free_parameter_is_not_the_refinement_subject() {
    reject(
        "module sample; type Contract is protocol {fn accept(limit:Int,value:Int{it>=0})->Unit;}; type Worker is (); implement Contract for Worker {fn accept(limit:Int,value:Int{limit>=0})->Unit {}}",
    );
}
#[test]
fn same_named_predicate_functions_keep_their_declaring_owners() {
    let prefix = "module sample; module first {public fn valid(value:Int)->Bool {value>=0}} module second {public fn valid(value:Int)->Bool {value>0}}";
    let body = source("Int{first.valid(it)}", "Int{second.valid(it)}");
    reject(&format!(
        "{prefix} {}",
        body.trim_start_matches("module sample;")
    ));
}
#[test]
fn one_shared_refinement_alias_remains_accepted() {
    let body = source("Count", "Count");
    accept(&format!(
        "module sample; type Count is Int{{>=0}}; {}",
        body.trim_start_matches("module sample;")
    ));
}

#[test]
fn reference_domains_are_not_reinterpreted_as_integer_domains() {
    use verum_ast::{
        decl::{FunctionParamKind, ItemKind},
        ty::TypeKind,
    };
    let module = Parser::new("fn probe(value:&Int{>=0}) {}")
        .parse_module()
        .unwrap();
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("function")
    };
    let FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
        panic!("parameter")
    };
    let TypeKind::Refined { base, .. } = &ty.kind else {
        panic!("refinement on reference carrier: {ty:?}")
    };
    assert!(matches!(base.kind, TypeKind::Reference { .. }));
    // This predicate is over a reference, not an intrinsic integer sort.
    // Identical text alone supplies no proof in that unsupported domain.
    reject(&source("&Int{>=0}", "&Int{>=0}"));
}
