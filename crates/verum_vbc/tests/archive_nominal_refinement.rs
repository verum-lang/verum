//! A forward carrier cannot erase the declaration imported later in the archive.
#![cfg(feature = "codegen")]
use verum_common::ResourceDiscipline;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    types::{TypeId, TypeRef},
};

#[test]
fn resolved_owned_fields_survive_a_duplicate_forward_carrier() {
    for (forward_first, affine) in [(true, false), (false, false), (true, true), (false, true)] {
        let source_text = r#"
            module core.provider;
            public type Deadline is { nanos: Int };
            public type Reason is Cancelled | Timeout { deadline: Deadline };
        "#;
        let source_text = if affine {
            source_text.replace("type Deadline", "type affine Deadline")
        } else {
            source_text.to_owned()
        };
        let expected = if affine {
            ResourceDiscipline::Affine
        } else {
            ResourceDiscipline::Unrestricted
        };
        let source = Parser::new(&source_text).parse_module().unwrap();
        let mut module = VbcCodegen::with_config(CodegenConfig::new("core.provider"))
            .compile_module(&source)
            .unwrap();
        let reason = module
            .types
            .iter()
            .find(|t| module.strings.get(t.name) == Some("Reason"))
            .unwrap()
            .clone();
        assert_eq!(
            module.resource_discipline(&TypeRef::Concrete(reason.id)),
            expected
        );
        let mut forward = reason.clone();
        forward.id = TypeId(module.types.iter().map(|t| t.id.0).max().unwrap() + 1);
        forward.name = module.strings.intern("core.provider.Reason");
        let field = &mut forward.variants[1].fields[0];
        field.type_ref = TypeRef::Concrete(TypeId::PTR);
        field.declaration_type = Some(TypeRef::Concrete(TypeId::PTR));
        if forward_first {
            module.types.insert(0, forward);
        } else {
            module.types.push(forward);
        }
        let module = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        consumer.import_archive_module_types(&module);
        let imported = consumer.finalize_module_from_state().unwrap();
        let reason = imported
            .types
            .iter()
            .find(|t| imported.strings.get(t.name) == Some("Reason"))
            .unwrap();
        assert_eq!(
            imported.resource_discipline(&TypeRef::Concrete(reason.id)),
            expected,
            "forward_first={forward_first}, fields={:?}",
            reason.variants[1].fields
        );
    }
}

#[test]
fn declaring_module_outranks_a_later_transitive_copy() {
    for proxy_first in [true, false] {
        let ast = Parser::new("module core.provider; public type Handle is { value: Int };")
            .parse_module()
            .unwrap();
        let declaration = VbcCodegen::with_config(CodegenConfig::new("core.provider"))
            .compile_module(&ast)
            .unwrap();
        let mut proxy = declaration.clone();
        proxy.name = "core.consumer".into();
        let handle = proxy
            .types
            .iter_mut()
            .find(|t| proxy.strings.get(t.name) == Some("Handle"))
            .unwrap();
        handle.fields[0].type_ref = TypeRef::Concrete(TypeId::PTR);
        handle.fields[0].declaration_type = Some(TypeRef::Concrete(TypeId::PTR));
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
        let modules = if proxy_first {
            [&proxy, &declaration]
        } else {
            [&declaration, &proxy]
        };
        for module in modules {
            consumer.import_archive_module_types(module);
        }
        let imported = consumer.finalize_module_from_state().unwrap();
        let handle = imported
            .types
            .iter()
            .find(|t| imported.strings.get(t.name) == Some("Handle"))
            .unwrap();
        assert_eq!(
            imported.resource_discipline(&TypeRef::Concrete(handle.id)),
            ResourceDiscipline::Unrestricted
        );
    }
}

#[test]
fn bootstrap_dependencies_select_the_declaration_over_a_promoted_carrier() {
    for forward_first in [true, false] {
        let ast = Parser::new(
            "module core.provider; public type affine Handle is { fd: Int }; public type Stream is Plain(Handle) | Empty;",
        ).parse_module().unwrap();
        let mut provider = VbcCodegen::with_config(CodegenConfig::new("core.provider"))
            .compile_module(&ast).unwrap();
        let mut forward = provider.types.iter()
            .find(|ty| provider.strings.get(ty.name) == Some("Handle"))
            .unwrap().clone();
        forward.id = TypeId(provider.types.iter().map(|ty| ty.id.0).max().unwrap() + 1);
        forward.name = provider.strings.intern("core.provider.Handle");
        forward.fields[0].type_ref = TypeRef::Concrete(TypeId::PTR);
        forward.fields[0].declaration_type = Some(TypeRef::Concrete(TypeId::PTR));
        if forward_first { provider.types.insert(0, forward); }
        else { provider.types.push(forward); }
        let source = Parser::new("module consumer; mount core.provider.Stream;")
            .parse_module().unwrap();
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        consumer.import_bootstrap_nominal_dependencies(&[&source], &[&provider]).unwrap();
        let imported = consumer.finalize_module_from_state().unwrap();
        let stream = imported.types.iter()
            .find(|ty| imported.strings.get(ty.name) == Some("Stream"))
            .unwrap();
        assert_eq!(imported.resource_discipline(&TypeRef::Concrete(stream.id)),
            ResourceDiscipline::Affine, "forward_first={forward_first}");
    }
}

#[test]
fn reused_codegen_remaps_a_new_archive_namespace() {
    let ast = Parser::new("module core.provider; public type Deadline is { nanos: Int }; public type Envelope is { deadline: Deadline };")
        .parse_module().unwrap();
    let declaration = VbcCodegen::with_config(CodegenConfig::new("core.provider"))
        .compile_module(&ast)
        .unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    consumer.import_archive_module_types(&declaration);
    consumer
        .compile_module(
            &Parser::new("module reset; fn probe() -> Int { 1 }")
                .parse_module()
                .unwrap(),
        )
        .unwrap();

    let mut proxy = declaration.clone();
    proxy.name = "core.transitive".into();
    let deadline = proxy
        .types
        .iter_mut()
        .find(|t| proxy.strings.get(t.name) == Some("Deadline"))
        .unwrap();
    deadline.id = TypeId(4000);
    let envelope = proxy
        .types
        .iter_mut()
        .find(|t| proxy.strings.get(t.name) == Some("Envelope"))
        .unwrap();
    envelope.id = TypeId(4001);
    envelope.fields[0].type_ref = TypeRef::Concrete(TypeId(4000));
    envelope.fields[0].declaration_type = Some(TypeRef::Concrete(TypeId(4000)));
    let proxy = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&proxy).unwrap(),
    )
    .unwrap();
    consumer.import_archive_module_types(&proxy);
    let imported = consumer.finalize_module_from_state().unwrap();
    let envelope = imported
        .types
        .iter()
        .find(|t| imported.strings.get(t.name) == Some("Envelope"))
        .unwrap();
    assert_eq!(
        imported.resource_discipline(&TypeRef::Concrete(envelope.id)),
        ResourceDiscipline::Unrestricted
    );
}
