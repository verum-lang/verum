//! Alias cleanup follows declaration identity and its semantic address carrier.
use super::ParameterOwnership;
use crate::codegen::{CodegenConfig, VbcCodegen};
use crate::types::{CbgrTier, Mutability, TypeId, TypeRef};

fn address_declaration(mutable: bool) -> (VbcCodegen, TypeId) {
    let permission = if mutable { "mut" } else { "const" };
    let source = format!("module aliases; type Address is *{permission} Int;");
    let ast = verum_fast_parser::Parser::new(&source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("aliases"));
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    let id = compiler.nominal_type_id("aliases.Address").unwrap();
    (compiler, id)
}

#[test]
fn raw_alias_requires_matching_semantic_address_permission() {
    for mutable in [false, true] {
        let (mut compiler, id) = address_declaration(mutable);
        let selected = TypeRef::Concrete(id);
        assert!(matches!(compiler.parameter_ownership(Some(&selected), None),
            Ok(ParameterOwnership::RawAddress)));
        let permission = if mutable { Mutability::Mutable } else { Mutability::Immutable };
        let opposite = if mutable { Mutability::Immutable } else { Mutability::Mutable };
        for target in [
            None,
            Some(TypeRef::Concrete(TypeId::INT)),
            Some(TypeRef::Reference { inner: Box::new(TypeRef::Concrete(TypeId::INT)),
                mutability: permission, tier: CbgrTier::Tier0 }),
            Some(TypeRef::Reference { inner: Box::new(TypeRef::Concrete(TypeId::INT)),
                mutability: permission, tier: CbgrTier::Tier1 }),
            Some(TypeRef::Reference { inner: Box::new(TypeRef::Concrete(TypeId::INT)),
                mutability: opposite, tier: CbgrTier::Tier2 }),
        ] {
            compiler.types.iter_mut().find(|ty| ty.id == id).unwrap().alias_target = target;
            assert!(compiler.parameter_ownership(Some(&selected), None)
                .err().unwrap().to_string().contains("contradicts its semantic target"));
        }
    }
}

#[test]
fn value_and_reference_aliases_keep_their_own_parameter_discipline() {
    let source = "module aliases;
        type Cell is { value: Int };
        type ValueAlias is Cell;
        type RefAlias is &Cell;
        type RawAlias is *const Cell;
        type NestedRaw is RawAlias;";
    let ast = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("aliases"));
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    for (name, expected) in [
        ("ValueAlias", ParameterOwnership::Value),
        ("RefAlias", ParameterOwnership::Borrowed),
        ("RawAlias", ParameterOwnership::RawAddress),
        ("NestedRaw", ParameterOwnership::RawAddress),
    ] {
        let id = compiler.nominal_type_id(&format!("aliases.{name}")).unwrap();
        assert!(compiler.parameter_ownership(Some(&TypeRef::Concrete(id)), None)
            .is_ok_and(|actual| actual == expected), "{name}");
    }
}
