//! T1547: spelling-specific archive carriers converge only at runtime finalization.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind};

fn codegen() -> VbcCodegen {
    let mut config = CodegenConfig::new("consumer");
    config.strict_codegen = true;
    VbcCodegen::with_config(config)
}

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn source(first: &str, second: &str) -> String {
    format!(
        "module carrier_source; type FirstOnly is protocol {{}}; type SecondOnly is protocol {{}}; implement FirstOnly for {first} {{}} implement SecondOnly for {second} {{}}"
    )
}

fn assert_runtime_carrier(module: &VbcModule, id: TypeId, protocols: &[&str]) {
    let carriers: Vec<_> = module.types.iter().filter(|ty| ty.id == id).collect();
    assert_eq!(
        carriers.len(),
        1,
        "runtime identity must have one descriptor"
    );
    assert_eq!(carriers[0].kind, TypeKind::Primitive);
    let mut actual: Vec<_> = carriers[0]
        .protocols
        .iter()
        .map(|implementation| {
            let protocol = module.get_type(TypeId(implementation.protocol.0)).unwrap();
            module.strings.get(protocol.name).unwrap()
        })
        .collect();
    actual.sort_unstable();
    let mut expected = protocols.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn source_aliases_finalize_to_one_runtime_identity_in_both_orders() {
    for (first, second) in [
        ("Byte", "UInt8"),
        ("UInt8", "Byte"),
        ("USize", "ISize"),
        ("ISize", "USize"),
    ] {
        let ast = Parser::new(&source(first, second)).parse_module().unwrap();
        let mut codegen = codegen();
        let archive_module = wire(&codegen.compile_module(&ast).unwrap());
        let id = TypeId::from_well_known_scalar_name(first).unwrap();
        // Metadata production keeps declared target spellings before runtime assembly.
        let names: Vec<_> = archive_module
            .types
            .iter()
            .filter(|ty| ty.id == id)
            .map(|ty| archive_module.strings.get(ty.name).unwrap())
            .collect();
        assert_eq!(names, [first, second]);
        let runtime = wire(&codegen.finalize_module().unwrap());
        assert!(codegen.verify_global_type_table_consistency().is_clean());
        assert_runtime_carrier(&runtime, id, &["FirstOnly", "SecondOnly"]);
        let again = codegen.finalize_module().unwrap();
        assert_runtime_carrier(&again, id, &["FirstOnly", "SecondOnly"]);
    }
}

#[test]
fn imported_alias_carriers_finalize_without_losing_protocols() {
    for (first, second) in [
        ("Byte", "UInt8"),
        ("UInt8", "Byte"),
        ("USize", "ISize"),
        ("ISize", "USize"),
    ] {
        let ast = Parser::new(&source(first, second)).parse_module().unwrap();
        let producer = wire(&codegen().compile_module(&ast).unwrap());
        let mut consumer = codegen();
        consumer.import_archive_module_types(&producer);
        consumer.import_archive_module_types(&producer);
        let runtime = wire(&consumer.finalize_module().unwrap());
        assert!(consumer.verify_global_type_table_consistency().is_clean());
        assert_runtime_carrier(
            &runtime,
            TypeId::from_well_known_scalar_name(first).unwrap(),
            &["FirstOnly", "SecondOnly"],
        );
    }
}

#[test]
fn distinct_scalar_ids_remain_distinct_at_finalization() {
    let ast = Parser::new(&source("Bool", "Int")).parse_module().unwrap();
    let mut codegen = codegen();
    codegen.compile_module(&ast).unwrap();
    let runtime = codegen.finalize_module().unwrap();
    assert_runtime_carrier(&runtime, TypeId::BOOL, &["FirstOnly"]);
    assert_runtime_carrier(&runtime, TypeId::INT, &["SecondOnly"]);
}

fn descriptor(codegen: &mut VbcCodegen, name: &str, id: TypeId, kind: TypeKind) {
    let name = verum_vbc::types::StringId(codegen.ctx_mut().intern_string_raw(name));
    codegen.push_type_for_test(TypeDescriptor {
        id,
        name,
        kind,
        ..Default::default()
    });
}

#[test]
fn unrelated_nominal_duplicate_ids_remain_rejected() {
    let mut codegen = codegen();
    descriptor(&mut codegen, "First", TypeId(1500), TypeKind::Record);
    descriptor(&mut codegen, "Second", TypeId(1500), TypeKind::Record);
    assert!(codegen.finalize_module().is_err());
    assert!(!codegen.verify_global_type_table_consistency().is_clean());
}

#[test]
fn scalar_spelling_on_nominal_descriptor_is_not_a_merge_proof() {
    let mut codegen = codegen();
    descriptor(&mut codegen, "Byte", TypeId::U8, TypeKind::Primitive);
    descriptor(&mut codegen, "UInt8", TypeId::U8, TypeKind::Record);
    assert!(codegen.finalize_module().is_err());
}

#[test]
fn scalar_spelling_with_wrong_id_is_not_a_merge_proof() {
    let mut codegen = codegen();
    descriptor(&mut codegen, "Byte", TypeId(1500), TypeKind::Primitive);
    descriptor(&mut codegen, "UInt8", TypeId(1500), TypeKind::Primitive);
    assert!(codegen.finalize_module().is_err());
}

#[test]
fn incompatible_scalar_glue_is_not_silently_discarded() {
    let mut codegen = codegen();
    descriptor(&mut codegen, "Byte", TypeId::U8, TypeKind::Primitive);
    let name = verum_vbc::types::StringId(codegen.ctx_mut().intern_string_raw("UInt8"));
    codegen.push_type_for_test(TypeDescriptor {
        id: TypeId::U8,
        name,
        kind: TypeKind::Primitive,
        drop_fn: Some(9),
        ..Default::default()
    });
    assert!(codegen.finalize_module().is_err());
}

#[test]
fn scalar_alias_methods_and_associated_metadata_survive_runtime_assembly() {
    let source = r#"
        module callable_carriers;
        type FirstOnly<T> is protocol { type Output; fn first_marker(&self) -> Int; };
        type SecondOnly<T> is protocol { type Output; fn second_marker(&self) -> Int; };
        implement FirstOnly<Int> for Byte { type Output = Int; fn first_marker(&self) -> Int { 17 } }
        implement SecondOnly<Text> for UInt8 { type Output = Text; fn second_marker(&self) -> Int { 23 } }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = codegen();
    let archive = wire(&codegen.compile_module(&ast).unwrap());
    let source_impls: Vec<_> = archive
        .types
        .iter()
        .filter(|ty| ty.id == TypeId::U8)
        .flat_map(|ty| ty.protocols.iter().cloned())
        .collect();
    assert_eq!(source_impls.len(), 2);
    assert!(
        source_impls
            .iter()
            .all(|implementation| !implementation.associated_types.is_empty()
                && !implementation.protocol_args_text.is_empty())
    );
    let mut runtime = wire(&codegen.finalize_module().unwrap());
    assert_eq!(
        runtime.get_type(TypeId::U8).unwrap().protocols.as_slice(),
        source_impls.as_slice()
    );
    runtime.resolve_protocol_dispatch();
    for method in ["first_marker", "second_marker"] {
        let function = runtime
            .resolve_protocol_method_by_name(TypeId::U8.0, method)
            .unwrap_or_else(|| panic!("lost scalar alias method {method}"));
        let descriptor = runtime.get_function(function).unwrap();
        assert!(
            runtime
                .get_string(descriptor.name)
                .unwrap()
                .ends_with(method)
        );
        assert!(descriptor.bytecode_length > 0);
    }
    // A spelling without the declared parent identity is not an alias proof.
    let second = runtime
        .resolve_protocol_method_by_name(TypeId::U8.0, "second_marker")
        .unwrap();
    runtime
        .functions
        .iter_mut()
        .find(|function| function.id == second)
        .unwrap()
        .parent_type = None;
    runtime.resolve_protocol_dispatch();
    assert!(
        runtime
            .resolve_protocol_method_by_name(TypeId::U8.0, "first_marker")
            .is_some()
    );
    assert!(
        runtime
            .resolve_protocol_method_by_name(TypeId::U8.0, "second_marker")
            .is_none()
    );
}

#[test]
fn same_named_nominal_method_cannot_attach_to_the_scalar_identity() {
    let source = r#"
        module nominal_sibling;
        type UInt8 is { value: Int };
        type FirstOnly is protocol { fn first_marker(&self) -> Int; };
        type SecondOnly is protocol { fn second_marker(&self) -> Int; };
        implement FirstOnly for Byte { fn first_marker(&self) -> Int { 17 } }
        implement SecondOnly for UInt8 { fn second_marker(&self) -> Int { 23 } }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = codegen();
    codegen.compile_module(&ast).unwrap();
    let mut runtime = wire(&codegen.finalize_module().unwrap());
    let nominal = runtime
        .types
        .iter()
        .find(|ty| runtime.get_string(ty.name) == Some("UInt8") && ty.kind == TypeKind::Record)
        .unwrap()
        .id;
    assert_ne!(nominal, TypeId::U8);
    runtime.resolve_protocol_dispatch();
    assert!(
        runtime
            .resolve_protocol_method_by_name(TypeId::U8.0, "first_marker")
            .is_some()
    );
    assert!(
        runtime
            .resolve_protocol_method_by_name(TypeId::U8.0, "second_marker")
            .is_none()
    );
    assert!(
        runtime
            .resolve_protocol_method_by_name(nominal.0, "second_marker")
            .is_some()
    );
}
