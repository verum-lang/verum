//! Lifecycle metadata belongs to the selected canonical protocol declaration.
#![cfg(feature = "codegen")]
use verum_ast::{ItemKind, Module};
use verum_common::{List, Map};
use verum_fast_parser::Parser;
use verum_vbc::{
    FunctionId, TypeRef, VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

fn protocol() -> Module {
    let mut source = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    source.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Clone")
    });
    assert_eq!(source.items.len(), 2);
    source
}
fn compile(source: &str) -> VbcModule {
    let protocol = protocol();
    let source = Parser::new(source).parse_module().unwrap();
    let units: List<_> = [&protocol, &source].into_iter().collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen.collect_unit_declarations(&units).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}
fn token(module: &VbcModule) -> &verum_vbc::types::TypeDescriptor {
    module
        .types
        .iter()
        .find(|d| {
            module
                .get_string(d.name)
                .is_some_and(|name| name == "Token" || name == "app.Token")
        })
        .unwrap()
}
fn wire(module: &VbcModule) -> VbcModule {
    let module = deserialize_module(&serialize_module(module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    module
}
fn selected(module: &VbcModule) -> &verum_vbc::module::FunctionDescriptor {
    let reference = token(module)
        .clone_fn
        .expect("canonical source Clone is recorded");
    let function = module
        .get_function(module.resolved_function_id(reference))
        .expect("exact selected body");
    assert!(function.has_source_body);
    assert!(function.source_signature.is_some());
    assert!(function.declaration_identity.is_some());
    function
}

fn import_bodies(codegen: &mut VbcCodegen, archived: &VbcModule) {
    use verum_vbc::codegen::context::FunctionInfo;
    // This is the existing archive body import interface. Only IDs are
    // relocated; no lifecycle descriptor or body is supplied by the fixture.
    let mut remap = Map::new();
    for function in &archived.functions {
        let id = FunctionId(5000 + function.id.0);
        let name = archived.get_string(function.name).unwrap();
        let owner = function
            .origin_module
            .and_then(|n| archived.get_string(n))
            .unwrap_or(&archived.name);
        let name = verum_vbc::module::qualify_module_name(owner, name);
        codegen.ctx_mut().register_function(
            name,
            FunctionInfo {
                id,
                param_count: function.params.len(),
                ..Default::default()
            },
        );
        codegen.record_archive_function_descriptor(archived, function, id);
        remap.insert(function.id.0, id);
    }
    codegen.merge_archive_function_bodies(archived, &remap.into_iter().collect());
}

#[test]
fn canonical_generic_clone_keeps_its_receiver_telescope_through_wire() {
    let module = compile(
        r#"
        module app;
        mount core.base.protocols.Clone;
        public type Token<T> is { public value: T };
        implement<T> Clone for Token<T> {
            fn clone(&self)->Token<T> { Token { value: self.value } }
        }
    "#,
    );
    for module in [&module, &wire(&module)] {
        let function = selected(module);
        let argument = TypeRef::Concrete(verum_vbc::types::TypeId::I64);
        assert_eq!(function.type_params.len(), 1);
        let substitution =
            verum_vbc::mono::TypeSubstitution::new(&function.type_params, &[argument.clone()]);
        let expected = TypeRef::Instantiated {
            base: token(module).id,
            args: vec![argument],
        };
        assert_eq!(
            substitution
                .apply(function.semantic_return.as_ref().unwrap())
                .unwrap(),
            expected
        );
        let receiver = function.semantic_parameter_types().unwrap()[0]
            .as_ref()
            .unwrap();
        let receiver = substitution.apply(receiver).unwrap();
        assert!(
            matches!(receiver, TypeRef::Reference { inner, mutability: verum_vbc::types::Mutability::Immutable, .. } if *inner == expected)
        );
    }
}

#[test]
fn renamed_and_forward_implementations_record_the_same_declared_lifecycle() {
    for forward in [false, true] {
        let declaration = "public type Token is { public value: Int };";
        let implementation = "implement Reproduce for Token { fn clone(&self)->Token { Token { value: self.value } } }";
        let source = format!(
            "module app; mount core.base.protocols.Clone as Reproduce; {} {} fn main()->Int{{let original=Token{{value:73}}; original.clone().value}}",
            if forward { implementation } else { declaration },
            if forward { declaration } else { implementation }
        );
        let module = wire(&compile(&source));
        selected(&module);
        let entry = module.find_function_by_name("app.main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn inherent_and_foreign_protocol_namesakes_never_publish_clone_authority() {
    for implementation in [
        "implement Token { fn clone(&self)->Token { Token{value: self.value} } }",
        "type Clone is protocol { fn clone(&self)->Self; }; implement Clone for Token { fn clone(&self)->Token { Token{value: self.value} } }",
    ] {
        let source = format!("module app; type Token is {{value:Int}}; {implementation}");
        let module = compile(&source);
        for module in [&module, &wire(&module)] {
            assert_eq!(token(module).clone_fn, None);
        }
    }
}

#[test]
fn recorded_clone_metadata_does_not_turn_ordinary_place_copy_into_a_user_call() {
    let module = wire(&compile(
        r#"
        module app;
        mount core.base.protocols.Clone;
        type Token is { value: Int };
        implement Clone for Token { fn clone(&self)->Token { Token{value:0} } }
        fn main()->Int { let original=Token{value:73}; let copied=original; copied.value }
    "#,
    ));
    let function = selected(&module);
    assert_ne!(function.id, FunctionId(u32::MAX));
    let entry = module.find_function_by_name("app.main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}

#[test]
fn declaration_headers_do_not_advertise_an_uncompiled_clone_body() {
    let protocol = protocol();
    let source = Parser::new(
        r#"
        module app;
        mount core.base.protocols.Clone;
        type Token is { value: Int };
        implement Clone for Token { fn clone(&self)->Token { Token{value:self.value} } }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen
        .collect_unit_declarations(&[&protocol, &source])
        .unwrap();
    codegen.resolve_pending_imports();
    let headers = codegen.finalize_module_from_state().unwrap();
    // A forward reference is linking identity, not a completed body. The
    // header pass may retain that identity so archive closure can find it.
    let reference = token(&headers)
        .clone_fn
        .expect("declared lifecycle reference");
    let function = headers.get_function(headers.resolved_function_id(reference));
    assert!(!function.is_some_and(|function| function.has_source_body));
    assert!(headers.functions.iter().all(|f| !f.has_source_body));
}

#[test]
fn actual_archive_import_remaps_clone_to_its_original_declaring_owner() {
    let archived = wire(&compile(
        r#"
        module app;
        mount core.base.protocols.Clone;
        public type Token<T> is { public value: T };
        implement<T> Clone for Token<T> { fn clone(&self)->Token<T> { Token{value:self.value} } }
    "#,
    ));
    let original = selected(&archived).id;
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.import_archive_module_types(&archived);
    import_bodies(&mut codegen, &archived);
    let imported = wire(&codegen.finalize_module_from_state().unwrap());
    let clone = selected(&imported);
    let original = archived.get_function(original).unwrap();
    assert_eq!(clone.source_signature, original.source_signature);
    let expected_owner = token(&imported).id;
    assert!(
        matches!(&clone.semantic_return, Some(TypeRef::Instantiated { base, .. }) if *base == expected_owner)
    );
    assert_eq!(clone.type_params.len(), 1);
}

#[test]
fn an_inherent_namesake_does_not_replace_the_selected_protocol_body() {
    let module = wire(&compile(
        r#"
        module app;
        mount core.base.protocols.Clone;
        type Token is { value: Int };
        implement Token { fn clone(&self)->Int { 0 } }
        implement Clone for Token { fn clone(&self)->Token { Token{value:self.value} } }
    "#,
    ));
    assert_eq!(
        selected(&module).semantic_return,
        Some(TypeRef::Concrete(token(&module).id))
    );
}

#[test]
fn nominal_header_completion_retains_the_selected_clone() {
    let protocol = protocol();
    let source = Parser::new(r#"
        module app;
        mount core.base.protocols.Clone as Reproduce;
        type Token is { value: Int };
        implement Reproduce for Token {
            fn clone(&self)->Token { Token { value: self.value } }
        }
        public fn main()->Int { let original=Token{value:73}; original.clone().value }
    "#).parse_module().unwrap();
    let units: List<_> = [&protocol, &source].into_iter().collect();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
    producer.collect_unit_declarations(&units).unwrap();
    // Replay the actual nominal header, preserving its lexical mounts. The
    // implementation and every executable body remain the original source.
    let mut header = source.clone();
    header.items.retain(|item| matches!(&item.kind,
        ItemKind::Module(_) | ItemKind::Mount(_) | ItemKind::Type(_)));
    producer.collect_unit_declarations(&[&header]).unwrap();
    producer.resolve_pending_imports();
    producer.compile_unit_items(&units, ItemFailurePolicy::Strict).unwrap();
    let produced = producer.finalize_module_from_state().unwrap();
    let archived = wire(&produced);
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    consumer.import_archive_module_types(&archived);
    import_bodies(&mut consumer, &archived);
    consumer.collect_unit_declarations(&[&header]).unwrap();
    let completed = consumer.finalize_module_from_state().unwrap();
    for module in [produced, archived, wire(&completed)] {
        assert_eq!(token(&module).visibility, verum_vbc::types::Visibility::Private);
        selected(&module);
        let entry = module.find_function_by_name("app.main").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn a_foreign_owners_lifecycle_row_cannot_complete_a_same_shaped_record() {
    let source = r#"
        module app;
        mount core.base.protocols.Clone;
        type Token is { value: Int };
        type Other is { value: Int };
        implement Clone for Other {
            fn clone(&self)->Other { Other { value: self.value } }
        }
    "#;
    let original = compile(source);
    let foreign = original.types.iter().find(|descriptor|
        original.get_string(descriptor.name).is_some_and(|name|
            name == "Other" || name == "app.Other")).unwrap().clone();
    let target = token(&original).id;
    for transfer_row in [false, true] {
        let mut altered_module = original.clone();
        let altered = altered_module.types.iter_mut()
            .find(|descriptor| descriptor.id == target).unwrap();
        altered.clone_fn = foreign.clone_fn;
        if transfer_row { altered.protocols = foreign.protocols.clone(); }
        assert!(altered.clone_fn.is_some());
        let protocol = protocol();
        let ast = Parser::new(source).parse_module().unwrap();
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
        consumer.import_archive_module_types(&altered_module);
        // The type-only importer deliberately clears executable slots. Merge
        // the actual selected source bodies before exercising completion.
        import_bodies(&mut consumer, &altered_module);
        let error = consumer.collect_declaration_interface(&[&protocol, &ast])
            .expect_err("a real foreign body cannot acquire this declaration's lifecycle");
        assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)),
            "transfer_row={transfer_row}: {error:?}");
    }
}

#[test]
fn an_unrelated_declared_protocol_cannot_supply_canonical_clone_completion() {
    let source = r#"
        module app;
        mount core.base.protocols.Clone;
        type Token is { value: Int };
        type LocalClone is protocol { fn clone(&self)->Self; };
        implement LocalClone for Token {
            fn clone(&self)->Token { Token { value: self.value } }
        }
    "#;
    let mut altered = compile(source);
    let canonical = altered.types.iter().find(|descriptor|
        altered.get_string(descriptor.name) == Some("Clone")
            && descriptor.origin_module.and_then(|owner| altered.get_string(owner))
                == Some("core.base.protocols")).unwrap().id;
    let target_id = token(&altered).id;
    let target = altered.types.iter_mut().find(|descriptor| descriptor.id == target_id).unwrap();
    assert_eq!(target.clone_fn, None);
    assert_eq!(target.protocols.len(), 1);
    target.clone_fn = Some(target.protocols[0].methods[0]);
    target.protocols[0].protocol = verum_vbc::types::ProtocolId(canonical.0);
    let protocol = protocol();
    let ast = Parser::new(source).parse_module().unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    consumer.import_archive_module_types(&altered);
    import_bodies(&mut consumer, &altered);
    let error = consumer.collect_declaration_interface(&[&protocol, &ast])
        .expect_err("the selected implementation must actually declare canonical Clone");
    assert!(matches!(error.kind, verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)),
        "{error:?}");
}
