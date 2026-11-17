use super::VbcCodegen;
use crate::codegen::CodegenErrorKind;

#[test]
fn cyclic_alias_through_a_heap_cannot_supply_a_field_owner() {
    use crate::types::{StringId, TypeDescriptor, TypeId, TypeKind, TypeRef};

    let mut codegen = VbcCodegen::new();
    codegen.initialize();
    let heap = TypeDescriptor {
        id: TypeId::HEAP, name: StringId(codegen.intern_string("Heap")), kind: TypeKind::Record,
        origin_module: Some(StringId(codegen.intern_string("core.base.memory"))),
        ..Default::default()
    };
    codegen.register_archive_type_qualified(heap, "Heap".into(), Some("core.base.memory"), None);
    let id = TypeId(40000);
    let alias = TypeDescriptor {
        id, name: StringId(codegen.intern_string("Loop")), kind: TypeKind::Alias,
        origin_module: Some(StringId(codegen.intern_string("fixture"))),
        alias_target: Some(TypeRef::Instantiated { base: TypeId::HEAP, args: vec![TypeRef::Concrete(id)] }),
        ..Default::default()
    };
    codegen.register_archive_type_qualified(alias, "Loop".into(), Some("fixture"), None);
    assert_eq!(codegen.nominal_type_id("fixture.Loop"), Some(id));
    assert!(codegen.is_allocating_wrapper("core.base.memory.Heap"));
    let error = codegen.field_layout_owner("fixture.Loop", "value").unwrap_err();
    assert!(matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref message)
        if message.contains("cyclic field owner")), "{error}");
}

#[test]
fn source_definition_rejects_real_lexical_cycles() {
    for edges in [
        &[("loop", "loop")][..],
        &[("left", "right"), ("right", "left")][..],
    ] {
        let mut codegen = VbcCodegen::new();
        for &(child, parent) in edges {
            codegen.record_inline_type_parent(child, parent);
        }
        let error = codegen.definition_lexical_scopes(edges[0].0).unwrap_err();
        assert!(matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref message)
            if message == "cyclic lexical scope in source definition identity"));
    }
}

#[test]
fn inline_scope_retains_the_anonymous_root_as_its_parent() {
    let source = verum_fast_parser::Parser::new(
        "module main { public fn choose() -> Int { 7 } } fn choose() -> Int { 7 }",
    ).parse_module().unwrap();
    let mut codegen = VbcCodegen::new();
    codegen.collect_unit_declarations(&[&source]).unwrap();
    let scopes = codegen.definition_lexical_scopes("main").unwrap();
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0].owner, "main");
    assert_eq!(scopes[1].owner, "");
    assert!(scopes.iter().all(|scope| scope.known));
}

#[test]
fn anonymous_implementation_owner_is_explicit_and_survives_the_wire() {
    use verum_ast::source_implementation::AssociatedImplementationSource;
    let source = verum_fast_parser::Parser::new(
        "implement Token { fn value(&self) -> Int { 7 } }",
    ).parse_module().unwrap();
    let verum_ast::ItemKind::Impl(declaration) = &source.items[0].kind else {
        panic!("actual implementation source");
    };
    let root = AssociatedImplementationSource::new("".into(), declaration).unwrap();
    let named = AssociatedImplementationSource::new("main".into(), declaration).unwrap();
    assert_ne!(root, named);
    let encoded = serde_json::to_value(&root).unwrap();
    assert_eq!(serde_json::from_value::<AssociatedImplementationSource>(encoded.clone()).unwrap(), root);
    let mut malformed = encoded.clone();
    malformed["origin_module"] = "root..child".into();
    assert!(serde_json::from_value::<AssociatedImplementationSource>(malformed).is_err());
    let mut absent = encoded;
    absent.as_object_mut().unwrap().remove("origin_module");
    assert!(serde_json::from_value::<AssociatedImplementationSource>(absent).is_err());
}
