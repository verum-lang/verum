//! Nominal value carriers are instantiated through the owner's exact telescope.
use std::sync::Arc;
use verum_ast::{ItemKind, TypeDecl};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{TypeChecker, core_metadata::*};

const INDEXED: &str = "type Indexed<T,V: meta T> is {value:()};";
fn errors(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let module = Parser::new(source)
        .parse_module()
        .expect("actual source grammar");
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|e| format!("{e:?}"))
        .collect();
    for item in &module.items {
        if let Err(e) = checker.check_item(item) {
            errors.push(format!("{e:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|e| format!("{e:?}")),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("{d:?}")),
    );
    errors
}
fn check(source: &str) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("nominal_roles");
    errors(&mut checker, source)
}
fn accept(source: &str) {
    let found = check(source);
    assert!(found.is_empty(), "{found:#?}");
}
fn reject(source: &str, expected: &str) {
    let found = check(source);
    assert!(
        found.iter().any(|e| e.contains(expected)),
        "expected {expected}: {found:#?}"
    );
}
#[test]
fn dependent_nominal_carrier_is_instantiated_before_admission() {
    accept(
        &(INDEXED.to_owned() + "fn keep(value: Indexed<Bool,true>) -> Indexed<Bool,true> {value}"),
    );
}
#[test]
fn mixed_family_preserves_the_dependent_nominal_result_carrier() {
    let source = "type Indexed<T,V: meta T> is {value:()}; type Family is protocol {type Cell<T,V: meta T>;}; type Owner is {}; implement Family for Owner {type Cell<U,W: meta U> = Indexed<U,W>;} fn keep(cell: Owner.Cell<Bool,true>) -> Indexed<Bool,true> {cell}";
    reject(source, "generic_associated_types = false");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("nominal_roles");
    checker.set_generic_associated_types_enabled(true);
    let found = errors(&mut checker, source);
    assert!(found.is_empty(), "{found:#?}");
}
#[test]
fn alpha_renamed_symbolic_carriers_and_aliases_preserve_their_domains() {
    accept(
        &(INDEXED.to_owned()
            + "type Renamed<U,W: meta U> is Indexed<U,W>; fn keep<T,V: meta T>(value: Renamed<T,V>) -> Indexed<T,V> {value} fn concrete(value:Renamed<Bool,true>)->Indexed<Bool,true>{value}"),
    );
}
#[test]
fn caller_type_parameters_cannot_be_identified_to_fit_a_value() {
    reject(
        &(INDEXED.to_owned()
            + "fn wrong<T,U,V: meta T>(value: Indexed<T,V>) -> Indexed<U,V> {value}"),
        "E407",
    );
}
#[test]
fn wrong_sort_role_arity_and_value_identity_are_refused() {
    for suffix in [
        "fn wrong(value:Indexed<Bool,7>) {}",
        "fn wrong(value:Indexed<Int,true>) {}",
        "fn wrong(value:Indexed<true,7>) {}",
        "fn wrong(value:Indexed<Bool,Bool>) {}",
        "fn wrong(value:Indexed<Bool>) {}",
        "fn wrong(value:Indexed<Bool,true,Int>) {}",
    ] {
        reject(&(INDEXED.to_owned() + suffix), "E407");
    }
    reject(
        &(INDEXED.to_owned() + "fn wrong(value:Indexed<Bool,true>)->Indexed<Bool,false>{value}"),
        "exact value index",
    );
}
#[test]
fn each_dependent_sort_uses_its_own_preceding_type_binder() {
    let declaration = "type Pair<A,X: meta A,B,Y: meta B> is {value:()};";
    accept(
        &(declaration.to_owned()
            + "fn keep<A,X: meta A,B,Y: meta B>(value:Pair<A,X,B,Y>)->Pair<A,X,B,Y>{value} fn concrete(value:Pair<Bool,true,Int,7>)->Pair<Bool,true,Int,7>{value}"),
    );
    reject(
        &(declaration.to_owned()
            + "fn wrong<A,X: meta A,B,Y: meta B>(value:Pair<A,X,B,Y>)->Pair<A,Y,B,X>{value}"),
        "E407",
    );
}
#[test]
fn refinement_on_the_applied_carrier_is_not_erased() {
    accept(
        &(INDEXED.to_owned()
            + "fn keep(value:Indexed<Int{>= 0},3>)->Indexed<Int{>= 0},3>{value} fn symbolic<N:meta Int{>= 0}>(value:Indexed<Int{>= 0},N>)->Indexed<Int{>= 0},N>{value}"),
    );
    reject(
        &(INDEXED.to_owned() + "fn wrong(value:Indexed<Int{>= 0},-1>) {}"),
        "declared value domain",
    );
    reject(
        &(INDEXED.to_owned() + "fn wrong<N:meta Int>(value:Indexed<Int{>= 0},N>) {}"),
        "declared value domain",
    );
}
fn declaration(source: &str) -> TypeDecl {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("type")
    };
    declaration.clone()
}
fn descriptor(owner: &str) -> TypeDescriptor {
    TypeDescriptor {
        source_declaration: Some(declaration(
            "public type Indexed<T,V: meta T> is {value:()};",
        )),
        name: "Indexed".into(),
        module_path: owner.into(),
        origin_module_path: None,
        generic_params: List::from_iter([
            GenericParam {
                name: "T".into(),
                bounds: List::new(),
                default: None,
                type_bounds: List::new(),
                pid: Some(0),
            },
            GenericParam {
                name: "V".into(),
                bounds: List::new(),
                default: None,
                type_bounds: List::new(),
                pid: None,
            },
        ]),
        kind: TypeDescriptorKind::Record {
            fields: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    }
}
fn imported() -> TypeChecker {
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    for owner in ["provider", "foreign"] {
        let key: Text = format!("{owner}.Indexed").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, descriptor(owner));
    }
    let restored = decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap();
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(restored));
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker
}
#[test]
fn sidecar_replay_preserves_the_dependent_carrier() {
    let mut checker = imported();
    let found = errors(
        &mut checker,
        "fn keep<T,V:meta T>(value:provider.Indexed<T,V>)->provider.Indexed<T,V>{value} fn concrete(value:provider.Indexed<Bool,true>)->provider.Indexed<Bool,true>{value}",
    );
    assert!(found.is_empty(), "{found:#?}");
    let mut checker = imported();
    let found = errors(&mut checker, "fn wrong(value:provider.Indexed<Bool,7>) {}");
    assert!(found.iter().any(|e| e.contains("E407")), "{found:#?}");
}
#[test]
fn same_leaf_declaration_under_a_different_owner_is_not_the_same_nominal_type() {
    let mut checker = imported();
    let found = errors(
        &mut checker,
        "fn wrong(value:provider.Indexed<Bool,true>)->foreign.Indexed<Bool,true>{value}",
    );
    assert!(
        found.iter().any(|e| e.contains("Mismatch")
            && e.contains("provider.Indexed")
            && e.contains("foreign.Indexed")),
        "{found:#?}"
    );
}

#[test]
fn recursive_application_of_own_binders_is_an_identity_not_a_cycle() {
    accept("type Indexed<T,V: meta T> is {next: &unsafe Indexed<T,V>}; fn keep<T,V:meta T>(value: &Indexed<T,V>) -> &Indexed<T,V> {value}");
}

#[test]
fn source_refinement_keeps_its_dependent_carrier() {
    let declaration = "type Positive<T,V:meta T{>=0}> is {value:()};";
    accept(&(declaration.to_owned() + "fn keep(value:Positive<Int,3>)->Positive<Int,3>{value}"));
    reject(&(declaration.to_owned() + "fn wrong(value:Positive<Int,-1>){}"), "MetaRefinementViolation");
}

#[test]
fn nested_carrier_uses_only_the_selected_declaration_prefix() {
    accept("type Carrier<A> is {value:()}; type Indexed<T,V:meta Carrier<T>> is {value:()}; fn keep<N:meta Carrier<Bool>>(value:Indexed<Bool,N>)->Indexed<Bool,N>{value}");
    reject("type Carrier<A> is {value:()}; type Indexed<T,V:meta Carrier<T>> is {value:()}; fn wrong<N:meta Carrier<Int>>(value:Indexed<Bool,N>){}", "E407");
}

#[test]
fn ordinary_function_carrier_keeps_its_instantiated_signature() {
    let declaration = "type Callable<T,F:meta fn()->T> is {value:()};";
    accept(&(declaration.to_owned() + "fn keep<G:meta fn()->Bool>(value:Callable<Bool,G>)->Callable<Bool,G>{value}"));
    reject(&(declaration.to_owned() + "fn wrong<G:meta fn()->Int>(value:Callable<Bool,G>){}"), "E407");
}
