//! Value-indexed associated families preserve declaration roles and domains.
//! These gates qualify source checking and metadata replay, not VM execution.
use std::sync::Arc;
use verum_ast::{ItemKind, TypeDecl};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker, context::GenericBinder, core_metadata::*};

fn errors(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    let module = Parser::new(source)
        .parse_module()
        .expect("ordinary source grammar");
    let mut errors: Vec<_> = checker
        .prepare_module_declarations(&module)
        .iter()
        .map(|error| format!("{error:?}"))
        .collect();
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(|error| format!("{error:?}")),
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
fn checked(source: &str) -> (TypeChecker, Vec<String>) {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("families");
    let errors = errors(&mut checker, source);
    (checker, errors)
}
fn accept(source: &str) {
    let (_, errors) = checked(source);
    assert!(errors.is_empty(), "{errors:#?}");
}
fn reject(source: &str, diagnostic: &str) {
    let (_, errors) = checked(source);
    assert!(
        errors.iter().any(|error| error.contains(diagnostic)),
        "expected {diagnostic}: {errors:#?}"
    );
}
fn source(domain: &str, implementation: &str) -> String {
    format!(
        "type Indexed<N: meta Int> is {{value: ()}}; type Family is protocol {{ type Cell<N: meta {domain}>; }}; type Owner is {{}}; implement Family for Owner {{ type Cell<M: meta {implementation}> = Indexed<M>; }}"
    )
}
#[test]
fn value_family_declaration_has_a_rigid_value_binder() {
    let (checker, errors) = checked("type Family is protocol {type Cell<N: meta Int>;};");
    assert!(errors.is_empty(), "{errors:?}");
    let registry = checker.protocol_checker.read();
    let family = &registry
        .get_protocol(&"families.Family".into())
        .unwrap()
        .associated_types[&Text::from("Cell")];
    assert!(
        matches!(&family.binders.local[0], GenericBinder::Value(Type::Meta { ty, value: verum_types::ty::ValueTerm::Parameter(_), .. }) if **ty == Type::Int)
    );
}
#[test]
fn ordinary_application_preserves_the_value_index() {
    accept(&(source("Int", "Int") + " fn use_cell(cell: Owner.Cell<2>) -> Indexed<2> {cell}"));
    reject(
        &(source("Int", "Int") + " fn wrong(cell: Owner.Cell<2>) -> Indexed<3> {cell}"),
        "exact value index",
    );
}
#[test]
fn symbolic_local_identity_is_preserved_under_renaming() {
    accept(
        &(source("Int", "Int") + " fn keep<N: meta Int>(cell: Owner.Cell<N>) -> Indexed<N> {cell}"),
    );
    reject(
        &(source("Int", "Int")
            + " fn wrong<N: meta Int, M: meta Int>(cell: Owner.Cell<N>) -> Indexed<M> {cell}"),
        "exact value index",
    );
}
#[test]
fn non_integer_value_sort_is_not_rewritten_to_int() {
    accept(
        "type Indexed<B: meta Bool> is {value: ()}; type Family is protocol {type Cell<B: meta Bool>;}; type Owner is {}; implement Family for Owner {type Cell<C: meta Bool> = Indexed<C>;} fn keep(cell: Owner.Cell<true>) -> Indexed<true> {cell}",
    );
    reject(
        "type Family is protocol {type Cell<B: meta Bool>;}; type Owner is {}; implement Family for Owner {type Cell<N: meta Int> = Int;}",
        "sort",
    );
}
#[test]
fn value_argument_kind_arity_and_domain_are_checked() {
    let prefix = source("Int{>= 0}", "Int{>= 0}");
    accept(&(prefix.clone() + " fn keep(cell: Owner.Cell<0>) -> Indexed<0> {cell}"));
    for suffix in [
        "fn bad(cell: Owner.Cell<Bool>) {}",
        "fn bad(cell: Owner.Cell<true>) {}",
        "fn bad(cell: Owner.Cell<-1>) {}",
        "fn bad(cell: Owner.Cell<1, 2>) {}",
        "fn bad(cell: Owner.Cell) {}",
    ] {
        reject(&(prefix.clone() + suffix), "E405");
    }
}
#[test]
fn implementation_cannot_narrow_its_value_domain() {
    reject(&source("Int", "Int{>= 0}"), "domain");
    reject(&source("Int{>= 0}", "Int{> 0}"), "domain");
    accept(&source("Int{> 0}", "Int{>= 0}"));
    accept(&source("Int{>= 0}", "Int"));
}
#[test]
fn type_and_value_roles_cannot_be_interchanged() {
    reject(
        "type Family is protocol {type Cell<N: meta Int>;}; type Owner is {}; implement Family for Owner {type Cell<N> = Int;}",
        "role",
    );
    reject(
        "type Family is protocol {type Cell<T>;}; type Owner is {}; implement Family for Owner {type Cell<T: meta Int> = Int;}",
        "role",
    );
}
#[test]
fn owner_type_and_shadowing_local_value_remain_independent() {
    accept(
        "type Indexed<T,N: meta Int> is {value:T}; type Family<N> is protocol {type Cell<N: meta Int>;}; type Owner<T> is {value:T}; implement<T> Family<T> for Owner<T> {type Cell<M: meta Int> = Indexed<T,M>;} type BooleanOwner is Owner<Bool>; fn keep(cell: BooleanOwner.Cell<2>) -> Indexed<Bool,2> {cell}",
    );
}
#[test]
fn dependent_value_sort_uses_the_preceding_type_binder() {
    accept(
        "type Family is protocol {type Cell<T,V: meta T>;}; type Owner is {}; implement Family for Owner {type Cell<U,W: meta U> = U;} fn keep(cell: Owner.Cell<Bool,true>) -> Bool {cell}",
    );
}
fn declaration(source: &str) -> TypeDecl {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Type(decl) = &module.items[0].kind else {
        panic!("declaration")
    };
    decl.clone()
}
fn descriptor(name: &str, source: &str, protocol: bool) -> TypeDescriptor {
    TypeDescriptor {
        source_declaration: Some(declaration(source)),
        name: name.into(),
        module_path: "provider".into(),
        origin_module_path: None,
        generic_params: List::new(),
        kind: if protocol {
            TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: List::new(),
                default_methods: List::new(),
            }
        } else {
            TypeDescriptorKind::Record {
                fields: List::new(),
            }
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
#[test]
fn value_domains_survive_sidecar_and_actual_declaration_replay() {
    let saved = declaration("public type Family<A> is protocol {type Cell<N: meta Int{>= 0}>;};");
    let mut desc = descriptor(
        "Family",
        "public type Family<A> is protocol {type Cell<N: meta Int{>= 0}>;};",
        true,
    );
    desc.generic_params.push(GenericParam {
        name: "A".into(),
        bounds: List::new(),
        default: None,
        type_bounds: List::new(),
        pid: Some(0),
    });
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    metadata
        .type_declaration_order
        .push("provider.Family".into());
    metadata.types.insert("provider.Family".into(), desc);
    let restored = decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap();
    assert_eq!(
        restored
            .types
            .get(&Text::from("provider.Family"))
            .unwrap()
            .source_declaration
            .as_ref(),
        Some(&saved)
    );
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(restored));
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostics().is_empty());
    let registry = checker.protocol_checker.read();
    let family = &registry
        .get_protocol(&"provider.Family".into())
        .unwrap()
        .associated_types[&Text::from("Cell")];
    let receiver = Type::bool();
    assert!(
        registry
            .complete_associated_family_arguments(
                family,
                &receiver,
                &[Type::bool()],
                &[Type::meta_value(
                    verum_common::ConstValue::Int(0),
                    Type::Int
                )],
                &[]
            )
            .is_ok()
    );
    assert!(
        registry
            .complete_associated_family_arguments(
                family,
                &receiver,
                &[Type::bool()],
                &[Type::meta_value(
                    verum_common::ConstValue::Int(-1),
                    Type::Int
                )],
                &[]
            )
            .is_err()
    );
}
fn family_metadata(implementation: &str) -> CoreMetadata {
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    for (name, source, protocol) in [
        (
            "Indexed",
            "public type Indexed<N: meta Int> is {value:()};",
            false,
        ),
        ("Owner", "public type Owner is {};", false),
        (
            "Family",
            "public type Family is protocol {type Cell<N: meta Int{>= 0}>;};",
            true,
        ),
    ] {
        let mut desc = descriptor(name, source, protocol);
        if name == "Indexed" {
            desc.generic_params.push(GenericParam {
                name: "N".into(),
                bounds: List::new(),
                default: None,
                type_bounds: List::new(),
                pid: None,
            });
        }
        let key: Text = format!("provider.{name}").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, desc);
    }
    let module = Parser::new(implementation).parse_module().unwrap();
    let ItemKind::Impl(decl) = &module.items[0].kind else {
        panic!("implementation")
    };
    metadata.implementations.push(ImplementationDescriptor {
        source_implementation: Some(
            verum_ast::source_implementation::AssociatedImplementationSource::new(
                "provider".into(),
                decl,
            )
            .unwrap(),
        ),
        protocol: "provider.Family".into(),
        target_type: "provider.Owner".into(),
        generic_params: List::new(),
        where_clause: List::new(),
        associated_types: Default::default(),
        methods: List::new(),
        protocol_args: List::new(),
    });
    decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap()
}
#[test]
fn metadata_implementation_uses_the_same_value_domain_admission() {
    let valid = "implement provider.Family for provider.Owner {type Cell<M: meta Int{>= 0}> = provider.Indexed<M>;}";
    let metadata = family_metadata(valid);
    TypeChecker::validate_source_implementation_metadata(Arc::new(metadata.clone())).unwrap();
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(metadata));
    checker.set_current_module_path("consumer");
    let found = errors(
        &mut checker,
        "fn keep(cell: provider.Owner.Cell<3>) -> provider.Indexed<3> {cell}",
    );
    assert!(found.is_empty(), "{found:#?}");
    let invalid = family_metadata(&valid.replace("Int{>= 0}", "Int{> 0}"));
    let error =
        TypeChecker::validate_source_implementation_metadata(Arc::new(invalid)).unwrap_err();
    assert!(format!("{error:?}").contains("domain"), "{error:?}");
}
#[test]
fn metadata_never_attaches_a_valid_family_to_a_foreign_receiver() {
    let mut metadata = family_metadata(
        "implement provider.Family for provider.Owner {type Cell<M: meta Int{>= 0}> = provider.Indexed<M>;}",
    );
    let mut foreign = metadata
        .types
        .get(&Text::from("provider.Owner"))
        .unwrap()
        .clone();
    foreign.module_path = "foreign".into();
    metadata.types.insert("foreign.Owner".into(), foreign);
    metadata.implementations[0].target_type = "foreign.Owner".into();
    let error =
        TypeChecker::validate_source_implementation_metadata(Arc::new(metadata)).unwrap_err();
    assert!(
        format!("{error:?}").contains("receiver") || format!("{error:?}").contains("owner"),
        "{error:?}"
    );
}

#[test]
fn value_local_can_shadow_the_owner_that_supplies_its_sort() {
    accept(
        "type Family<T> is protocol {type Cell<N: meta T>;}; type Owner<T> is {value:T}; implement<T> Family<T> for Owner<T> {type Cell<T: meta T> = Int;}",
    );
    reject(
        "type Family<T> is protocol {type Cell<N: meta T>;}; type Owner<T> is {value:T}; implement<T> Family<T> for Owner<T> {type Cell<T: meta Bool> = Int;}",
        "sort",
    );
}
