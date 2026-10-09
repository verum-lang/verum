//! T1706: sign meaning comes from canonical IDs and exact alias declarations.
use super::*;
use crate::codegen::CodegenConfig;
use crate::types::{TypeDescriptor, TypeParamId};
use verum_fast_parser::Parser;

#[test]
fn canonical_signed_ids_do_not_make_width_or_generic_ids_signed() {
    let codegen = VbcCodegen::new();
    for (id, bits) in [(TypeId::I8, 8), (TypeId::I16, 16), (TypeId::I32, 32)] {
        assert_eq!(
            codegen.array_element_signed_bits(&TypeRef::Concrete(id)),
            Some(bits)
        );
    }
    for id in [
        TypeId::U8,
        TypeId::U16,
        TypeId::U32,
        TypeId::INT,
        TypeId::F32,
        TypeId::F64,
    ] {
        assert_eq!(
            codegen.array_element_signed_bits(&TypeRef::Concrete(id)),
            None
        );
    }
    assert_eq!(
        codegen.array_element_signed_bits(&TypeRef::Generic(TypeParamId(0))),
        None
    );
}

#[test]
fn parsed_alias_chain_uses_the_exact_declaration_target() {
    let parsed = Parser::new(
        "type SignedShort is Int16; type Indirect is SignedShort; type UnsignedShort is UInt16;",
    )
    .parse_module()
    .expect("grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("element_owner"));
    codegen
        .compile_module(&parsed)
        .expect("source declarations");
    for (name, expected) in [
        ("element_owner.SignedShort", Some(16)),
        ("element_owner.Indirect", Some(16)),
        ("element_owner.UnsignedShort", None),
    ] {
        let id = codegen.nominal_type_id(name).expect("exact declared alias");
        assert_eq!(
            codegen.array_element_signed_bits(&TypeRef::Concrete(id)),
            expected,
            "{name}"
        );
    }
}

#[test]
fn same_spelled_record_is_not_a_signed_primitive() {
    let parsed = Parser::new("type Int16 is { value: Int };")
        .parse_module()
        .expect("grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("foreign"));
    codegen.compile_module(&parsed).expect("source record");
    let id = codegen
        .nominal_type_id("foreign.Int16")
        .expect("exact foreign owner");
    assert_ne!(id, TypeId::I16);
    assert_eq!(
        codegen.array_element_signed_bits(&TypeRef::Concrete(id)),
        None
    );
}

#[test]
fn missing_and_cyclic_alias_targets_have_no_signed_authority() {
    let mut codegen = VbcCodegen::new();
    let first = TypeId(0x10000);
    let second = TypeId(0x10001);
    codegen.types.push(TypeDescriptor {
        id: first,
        kind: TypeKind::Alias,
        alias_target: Some(TypeRef::Concrete(second)),
        ..TypeDescriptor::default()
    });
    assert_eq!(
        codegen.array_element_signed_bits(&TypeRef::Concrete(first)),
        None
    );
    codegen.types.push(TypeDescriptor {
        id: second,
        kind: TypeKind::Alias,
        alias_target: Some(TypeRef::Concrete(first)),
        ..TypeDescriptor::default()
    });
    assert_eq!(
        codegen.array_element_signed_bits(&TypeRef::Concrete(first)),
        None
    );
}
