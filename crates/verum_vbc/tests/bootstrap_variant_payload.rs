#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{StringId, TypeDescriptor, TypeParamId, TypeRef, VariantKind};

fn ast(text: &str) -> verum_ast::Module {
    Parser::new(text).parse_module().expect("source parses")
}
fn producer(owner: &str, text: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast(&format!("module {owner}; {text}")))
        .expect("source producer")
}
fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).expect("serialize"),
    )
    .expect("deserialize")
}
fn imported(text: &str, available: &[&VbcModule]) -> VbcModule {
    let source = ast(text);
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..300 {
        cg.ctx_mut()
            .intern_string_raw(&format!("unrelated_consumer_string_{i}"));
    }
    cg.import_bootstrap_nominal_dependencies(&[&source], available)
        .expect("bootstrap dependency import");
    cg.collect_unit_declarations(&[&source]).unwrap();
    cg.compile_function_bodies(&source).unwrap()
}
fn ty<'a>(module: &'a VbcModule, owner: &str, leaf: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            ty.origin_module.and_then(|s| module.strings.get(s)) == Some(owner)
                && module
                    .strings
                    .get(ty.name)
                    .is_some_and(|s| s.rsplit('.').next() == Some(leaf))
        })
        .unwrap_or_else(|| panic!("missing {owner}.{leaf}"))
}
fn variant<'a>(
    module: &'a VbcModule,
    owner: &str,
    leaf: &str,
    name: &str,
) -> &'a verum_vbc::types::VariantDescriptor {
    ty(module, owner, leaf)
        .variants
        .iter()
        .find(|v| module.strings.get(v.name) == Some(name))
        .unwrap()
}

#[test]
fn declared_payload_slots_survive_source_export_bootstrap_and_wire() {
    let source = roundtrip(&producer(
        "alpha",
        "type Choice<T, E> is First(T) | Second(E) | Named { value: E } | Empty;",
    ));
    let result = imported(
        "module consumer; fn accept(x: alpha.Choice<Int, Text>) -> Int { 7 }",
        &[&source],
    );
    for module in [&result, &roundtrip(&result)] {
        for (name, kind, field_name, slot) in [
            ("First", VariantKind::Tuple, "_0", 0),
            ("Second", VariantKind::Tuple, "_0", 1),
            ("Named", VariantKind::Record, "value", 1),
        ] {
            let v = variant(module, "alpha", "Choice", name);
            assert_eq!(v.kind, kind);
            assert_eq!(v.fields.len(), 1, "{name} retains payload");
            assert_eq!(v.fields[0].type_ref, TypeRef::Generic(TypeParamId(slot)));
            assert_eq!(module.strings.get(v.fields[0].name), Some(field_name));
            assert_eq!(
                module.strings.get(v.fields[0].type_name),
                Some(if slot == 0 { "T" } else { "E" })
            );
        }
        let unit = variant(module, "alpha", "Choice", "Empty");
        assert_eq!(unit.kind, VariantKind::Unit);
        assert!(unit.fields.is_empty());
    }
}

fn nominal_source(owner: &str, leaf_field: &str) -> VbcModule {
    roundtrip(&producer(
        owner,
        &format!(
            "type Leaf is {{ {leaf_field}: Int }}; type Wrap<T> is {{ value: T }}; type Event<T> is Pair(Leaf, Wrap<Leaf>, T) | Named {{ item: Wrap<Leaf>, other: T }} | End;"
        ),
    ))
}
fn check_nominal_payloads(module: &VbcModule, owner: &str, leaf_field: &str) {
    let leaf = ty(module, owner, "Leaf");
    assert_eq!(module.strings.get(leaf.fields[0].name), Some(leaf_field));
    let nested = TypeRef::Instantiated {
        base: ty(module, owner, "Wrap").id,
        args: vec![TypeRef::Concrete(leaf.id)],
    };
    let tuple = variant(module, owner, "Event", "Pair");
    assert_eq!(tuple.fields.len(), 3, "tuple metadata survives import");
    assert_eq!(tuple.fields[0].type_ref, TypeRef::Concrete(leaf.id));
    assert_eq!(tuple.fields[1].type_ref, nested);
    assert_eq!(tuple.fields[2].type_ref, TypeRef::Generic(TypeParamId(0)));
    assert_eq!(module.strings.get(tuple.fields[1].name), Some("_1"));
    assert_eq!(
        module.strings.get(tuple.fields[1].type_name),
        Some("Wrap<Leaf>")
    );
    let named = variant(module, owner, "Event", "Named");
    assert_eq!(named.fields[0].type_ref, nested);
    assert_eq!(named.fields[1].type_ref, TypeRef::Generic(TypeParamId(0)));
    assert_eq!(module.strings.get(named.fields[0].name), Some("item"));
}

#[test]
fn qualified_payload_dependencies_remain_distinct_in_both_orders() {
    let alpha = nominal_source("alpha", "left");
    let beta = nominal_source("beta", "right");
    let text = "module consumer; fn accept(a: alpha.Event<Int>, b: beta.Event<Text>) -> Int { 7 }";
    for available in [&[&alpha, &beta][..], &[&beta, &alpha][..]] {
        let result = roundtrip(&imported(text, available));
        check_nominal_payloads(&result, "alpha", "left");
        check_nominal_payloads(&result, "beta", "right");
        assert_ne!(
            ty(&result, "alpha", "Leaf").id,
            ty(&result, "beta", "Leaf").id
        );
        // Allocation by exact qualified key permutes source IDs. Reapplying
        // the source map to an already-remapped payload would select Wrap.
        assert_eq!(
            ty(&alpha, "alpha", "Wrap").id,
            ty(&result, "alpha", "Leaf").id
        );
        assert_ne!(
            ty(&alpha, "alpha", "Leaf").id,
            ty(&result, "alpha", "Leaf").id
        );
        let again = imported(text, &[&result]);
        check_nominal_payloads(&again, "alpha", "left");
        check_nominal_payloads(&again, "beta", "right");
    }
}

#[test]
fn missing_qualified_sum_does_not_borrow_a_same_leaf_payload() {
    let alpha = nominal_source("alpha", "left");
    let beta = nominal_source("beta", "right");
    for available in [&[&alpha, &beta][..], &[&beta, &alpha][..]] {
        let source = ast("module consumer; fn accept(x: alpha.child.Event<Int>) -> Int { 7 }");
        let mut cg = VbcCodegen::new();
        assert_eq!(
            cg.import_bootstrap_nominal_dependencies(&[&source], available)
                .unwrap(),
            0
        );
    }
}

#[test]
fn importer_preserves_payload_refs_and_reinterns_all_field_strings() {
    let mut source = producer(
        "alpha",
        "type Choice<T, E> is First(T) | Named { value: E };",
    );
    // Optional refinement strings are independently carried descriptor
    // fields. Seed these wire facts without depending on refinement parsing.
    let predicate = source.strings.intern("payload > 0");
    let binding = source.strings.intern("payload");
    let index = source
        .types
        .iter()
        .position(|t| source.strings.get(t.name) == Some("Choice"))
        .unwrap();
    for v in &mut source.types[index].variants {
        v.fields[0].refinement_src = predicate;
        v.fields[0].refinement_binding = binding;
    }
    let source = roundtrip(&source);
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..300 {
        cg.ctx_mut().intern_string_raw(&format!("foreign_pool_{i}"));
    }
    // This is the ordinary loader entry, which does not pre-remap nominal
    // TypeRefs. The common helper must preserve its supplied reference space.
    cg.import_archive_module_types(&source);
    let consumer = ast("module consumer; fn main() {} ");
    cg.collect_unit_declarations(&[&consumer]).unwrap();
    let result = cg.compile_function_bodies(&consumer).unwrap();
    for name in ["Named", "First"] {
        let before = variant(&source, "alpha", "Choice", name);
        let after = variant(&result, "alpha", "Choice", name);
        assert_eq!(after.fields.len(), 1, "{name}");
        assert_eq!(after.fields[0].type_ref, before.fields[0].type_ref);
        assert_eq!(
            result.strings.get(after.fields[0].refinement_src),
            Some("payload > 0")
        );
        assert_eq!(
            result.strings.get(after.fields[0].refinement_binding),
            Some("payload")
        );
        assert_ne!(after.fields[0].refinement_src, StringId::EMPTY);
    }
}

#[test]
fn absent_field_metadata_does_not_read_the_source_string_at_zero() {
    let mut source = producer(
        "alpha",
        "type Record is { value: Int }; type Choice<T> is First(T) | Named { value: T };",
    );
    assert_eq!(source.strings.get(StringId::EMPTY), Some("alpha"));
    for ty in &mut source.types {
        for field in ty
            .fields
            .iter_mut()
            .chain(ty.variants.iter_mut().flat_map(|v| &mut v.fields))
        {
            field.type_name = StringId::EMPTY;
            field.refinement_src = StringId::EMPTY;
            field.refinement_binding = StringId::EMPTY;
        }
    }
    let source = roundtrip(&source);
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..300 {
        cg.ctx_mut()
            .intern_string_raw(&format!("foreign_optional_pool_{i}"));
    }
    cg.import_archive_module_types(&source);
    let consumer = ast("module consumer; fn main() {}");
    cg.collect_unit_declarations(&[&consumer]).unwrap();
    let result = roundtrip(&cg.compile_function_bodies(&consumer).unwrap());
    let tuple = &variant(&result, "alpha", "Choice", "First").fields[0];
    let named = &variant(&result, "alpha", "Choice", "Named").fields[0];
    let record = &ty(&result, "alpha", "Record").fields[0];
    for (field, expected_name) in [(tuple, "_0"), (named, "value"), (record, "value")] {
        assert_eq!(result.strings.get(field.name), Some(expected_name));
        assert_eq!(
            field.type_name,
            StringId::EMPTY,
            "{expected_name} type name"
        );
        assert_eq!(
            field.refinement_src,
            StringId::EMPTY,
            "{expected_name} refinement"
        );
        assert_eq!(
            field.refinement_binding,
            StringId::EMPTY,
            "{expected_name} binding"
        );
    }
}
