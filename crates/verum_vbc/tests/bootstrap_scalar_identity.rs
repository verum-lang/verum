//! T1546: primitive protocol carriers do not introduce nominal scalar identities.
#![cfg(feature = "codegen")]

use std::collections::HashMap;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{context::FunctionInfo, CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeKind, TypeRef};

fn producer(owner: &str, scalar: &str, offset: u32) -> (VbcModule, HashMap<String, FunctionInfo>) {
    let source = format!(
        "module {owner}; type Marker is protocol {{}}; implement Marker for {scalar} {{}} fn supply(value: {scalar}) -> {scalar} {{ value }}"
    );
    compile(&source, owner, offset)
}

fn compile(source: &str, owner: &str, offset: u32) -> (VbcModule, HashMap<String, FunctionInfo>) {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(owner));
    let module = codegen.compile_module(&ast).unwrap();
    let mut registry = codegen.export_functions();
    for info in registry.values_mut().filter(|info| info.id.0 < 100_000) {
        info.id.0 += offset;
    }
    // Import the same declared carrier that survives an archive roundtrip.
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    (module, registry)
}

fn consumer(
    source: &str,
    modules: &[&VbcModule],
    registries: &[&HashMap<String, FunctionInfo>],
) -> (VbcModule, HashMap<String, FunctionInfo>) {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for registry in registries {
        codegen.import_functions(registry);
    }
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], modules)
        .unwrap();
    let registry = codegen.export_functions();
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    let module = codegen.compile_function_bodies(&ast).unwrap();
    (module, registry)
}

#[test]
fn scalar_free_function_return_survives_bootstrap_and_mount_alias() {
    let (module, registry) = producer("scalar.flags", "Bool", 1000);
    let carrier = module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some("Bool"))
        .unwrap();
    assert_eq!(carrier.kind, TypeKind::Primitive);
    assert_eq!(carrier.id, TypeId::BOOL);
    assert_eq!(carrier.protocols.len(), 1);
    for invocation in [
        "mount scalar.flags.supply as choose; fn probe() -> Bool { choose(true) }",
        "fn probe() -> Bool { scalar.flags.supply(true) }",
    ] {
        let (_, carried) = consumer(
            &format!("module consumer; {invocation}"),
            &[&module],
            &[&registry],
        );
        assert_eq!(
            carried["scalar.flags.supply"].return_type,
            Some(TypeRef::Concrete(TypeId::BOOL))
        );
    }
}

#[test]
fn scalar_free_function_return_keeps_identity_after_an_earlier_carrier_import() {
    let (alpha, ar) = producer("scalar.alpha", "Bool", 1000);
    let (beta, br) = producer("scalar.beta", "Bool", 2000);
    let source = Parser::new("module consumer; mount scalar.alpha.supply as a; mount scalar.beta.supply as b; fn probe() -> Bool { b(a(true)) }").parse_module().unwrap();
    for modules in [[&alpha, &beta], [&beta, &alpha]] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        codegen.import_functions(&ar);
        codegen.import_functions(&br);
        for module in modules {
            codegen
                .import_bootstrap_nominal_dependencies(&[&source], &[module])
                .unwrap();
        }
        let carried = codegen.export_functions();
        for owner in ["scalar.alpha", "scalar.beta"] {
            assert_eq!(
                carried[&format!("{owner}.supply")].return_type,
                Some(TypeRef::Concrete(TypeId::BOOL)),
                "{owner}"
            );
        }
    }
}

#[test]
fn scalar_alias_carriers_preserve_names_protocols_and_id_in_both_orders() {
    let (bytes, byte_registry) = producer("scalar.bytes", "Byte", 1000);
    let (u8s, u8_registry) = producer("scalar.u8s", "UInt8", 2000);
    let expected = TypeId::from_well_known_scalar_name("Byte").unwrap();
    assert_eq!(Some(expected), TypeId::from_well_known_scalar_name("UInt8"));
    for reverse in [false, true] {
        let modules = if reverse {
            [&u8s, &bytes]
        } else {
            [&bytes, &u8s]
        };
        let (module, registry) = consumer(
            "module consumer; mount scalar.bytes.supply as byte_supply; mount scalar.u8s.supply as u8_supply; fn probe() -> Int { 7 }",
            &modules, &[&byte_registry, &u8_registry],
        );
        for owner in ["scalar.bytes", "scalar.u8s"] {
            assert_eq!(
                registry[&format!("{owner}.supply")].return_type,
                Some(TypeRef::Concrete(expected))
            );
        }
        for (name, owner) in [("Byte", "scalar.bytes"), ("UInt8", "scalar.u8s")] {
            let carrier = module
                .types
                .iter()
                .find(|ty| module.strings.get(ty.name) == Some(name))
                .expect(name);
            assert_eq!(carrier.id, expected);
            assert_eq!(carrier.kind, TypeKind::Primitive);
            assert_eq!(
                carrier.origin_module.and_then(|id| module.strings.get(id)),
                Some(owner)
            );
            assert_eq!(
                carrier.protocols.len(),
                1,
                "{name} protocol attachment survives"
            );
        }
    }
}

#[test]
fn unrelated_nominal_bool_keeps_a_distinct_bootstrap_identity() {
    let (foreign, registry) = compile(
        "module foreign.api; type Bool is { tag: Int }; fn supply() -> foreign.api.Bool { Bool(tag: 7) }",
        "foreign.api", 1000,
    );
    let source_nominal = foreign
        .types
        .iter()
        .find(|ty| foreign.strings.get(ty.name) == Some("Bool"))
        .unwrap();
    assert_eq!(source_nominal.kind, TypeKind::Record);
    assert_ne!(source_nominal.id, TypeId::BOOL, "producer nominal identity");
    let (module, carried) = consumer(
        "module consumer; mount foreign.api.supply as make; fn probe() -> Int { 7 }",
        &[&foreign],
        &[&registry],
    );
    let nominal = module
        .types
        .iter()
        .find(|ty| module.strings.get(ty.name) == Some("Bool"))
        .unwrap();
    assert_eq!(nominal.kind, TypeKind::Record);
    assert_ne!(nominal.id, TypeId::BOOL);
    assert_eq!(
        carried["foreign.api.supply"].return_type,
        Some(TypeRef::Concrete(nominal.id))
    );
}
