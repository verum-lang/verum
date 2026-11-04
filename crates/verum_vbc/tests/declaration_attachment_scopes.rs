#![cfg(feature = "codegen")]
//! Source attachments keep their own lexical environments at every import boundary.
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

fn wire(module: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(module).unwrap()).unwrap()
}

fn selected(consumer: &mut VbcCodegen, module: &VbcModule) -> Result<usize, String> {
    let ast = Parser::new("module consumer; mount model.Packet; type Keep is { value: Packet<Int> };")
        .parse_module().unwrap();
    consumer.import_bootstrap_nominal_dependencies(&[&ast], &[module])
        .map_err(|error| error.to_string())
}

fn attached_declarations() -> VbcModule {
    let model = Parser::new("module model; public type Packet<T> is { public body:T }; public type Marker is protocol {};")
        .parse_module().unwrap();
    let inherent = Parser::new("module additions; mount model.Packet; mount hidden.Token as Local; implement<T> Packet<T> { public fn inspect(&self)->Int { 1 } }")
        .parse_module().unwrap();
    let protocol = Parser::new("module instances; mount model.{Packet,Marker}; mount other.Token as Local; implement<T> Marker for Packet<T> {}")
        .parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("bundle"))
        .collect_owned_declaration_interface(&[("model", &model), ("additions", &inherent), ("instances", &protocol)])
        .unwrap();
    let packet = module.types.iter().find(|ty| !ty.source_inherent_implementations.is_empty()).unwrap();
    assert_eq!(packet.source_inherent_implementations[0].origin_module(), "additions");
    assert!(packet.protocols.iter().any(|implementation| implementation.source_implementation.as_ref()
        .is_some_and(|source| source.origin_module() == "instances")));
    for (owner, target) in [("additions", "hidden.Token"), ("instances", "other.Token")] {
        let scope = module.declaration_type_bindings.get(&Text::from(owner)).expect(owner);
        assert_eq!(scope.get(&Text::from("Local")).unwrap().as_deref(), Some(target));
    }
    wire(&module)
}

#[test]
fn implementation_only_carrier_keeps_its_scope_through_bootstrap_and_wire() {
    let ast = Parser::new("module model; mount hidden.Token as Local; public type Packet<T> is { public body:T }; implement<T> Packet<T> { public fn inspect(&self)->Int { 1 } }")
        .parse_module().unwrap();
    let original = VbcCodegen::with_config(CodegenConfig::new("model")).compile_module(&ast).unwrap();
    let packet = original.types.iter().find(|ty| !ty.source_inherent_implementations.is_empty()).unwrap();
    assert!(packet.source_declaration.is_none(), "ordinary body compilation uses an implementation-only source carrier");
    assert!(original.declaration_type_bindings.contains_key(&Text::from("model")));
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    selected(&mut consumer, &wire(&original)).unwrap();
    let imported = wire(&consumer.finalize_module_from_state().unwrap());
    assert_eq!(imported.declaration_type_bindings, original.declaration_type_bindings);
    assert_eq!(imported.types.iter().flat_map(|ty| &ty.source_inherent_implementations).count(), 1);
}

#[test]
fn each_source_attachment_keeps_its_distinct_origin_scope() {
    let original = attached_declarations();
    for bootstrap in [false, true] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for _ in 0..2 {
            if bootstrap { selected(&mut consumer, &original).unwrap(); }
            else { consumer.import_archive_module_types(&original); }
        }
        let imported = wire(&consumer.finalize_module_from_state().unwrap());
        assert_eq!(imported.declaration_type_bindings, original.declaration_type_bindings, "bootstrap={bootstrap}");
        assert!(imported.types.iter().flat_map(|ty| &ty.protocols).any(|implementation|
            implementation.source_implementation.as_ref().is_some_and(|source| source.origin_module() == "instances")));
    }
}

#[test]
fn missing_and_changed_attachment_scopes_are_conflicts_in_both_orders() {
    let original = attached_declarations();
    for owner in ["additions", "instances"] {
        let mut missing = original.clone();
        missing.declaration_type_bindings.remove(&Text::from(owner));
        let mut changed = original.clone();
        changed.declaration_type_bindings.get_mut(&Text::from(owner)).unwrap()
            .insert("Local".into(), Some("intruder.Token".into()));
        for altered in [wire(&missing), wire(&changed)] {
            for (first, second) in [(&original, &altered), (&altered, &original)] {
                for bootstrap in [false, true] {
                    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
                    if bootstrap {
                        selected(&mut consumer, first).unwrap();
                        let error = selected(&mut consumer, second).unwrap_err();
                        assert!(error.contains(&format!("conflicting lexical type bindings for declaration owner {owner}")), "{error}");
                    } else {
                        consumer.import_archive_module_types(first);
                        consumer.import_archive_module_types(second);
                    }
                    let error = consumer.finalize_module_from_state().unwrap_err().to_string();
                    assert!(error.contains(&format!("conflicting lexical type bindings for declaration owner {owner}")), "{error}");
                }
            }
        }
    }
}

#[test]
fn a_nested_nominal_keeps_the_scope_used_by_its_consumer() {
    let ast = Parser::new("module model; mount hidden.Token as Local; public type Packet<T> is { public body:T, public universe:Type(1) };")
        .parse_module().unwrap();
    let mut original = VbcCodegen::with_config(CodegenConfig::new("model"))
        .collect_declaration_interface(&[&ast]).unwrap();
    original.types.iter_mut().find(|ty| ty.source_declaration.is_some()).unwrap().name =
        original.strings.intern("nested.Packet");
    let scope = original.declaration_type_bindings.remove(&Text::from("model")).unwrap();
    original.declaration_type_bindings.insert("model.nested".into(), scope);
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&wire(&original));
    let imported = wire(&consumer.finalize_module_from_state().unwrap());
    assert_eq!(imported.declaration_type_bindings, original.declaration_type_bindings);
}

#[test]
fn malformed_source_name_cannot_leave_a_successful_full_import() {
    let ast = Parser::new("module model; public type Packet is { public universe:Type(1) };")
        .parse_module().unwrap();
    let mut original = VbcCodegen::with_config(CodegenConfig::new("model"))
        .collect_declaration_interface(&[&ast]).unwrap();
    original.types.iter_mut().find(|ty| ty.source_declaration.is_some()).unwrap().name =
        verum_vbc::types::StringId(u32::MAX);
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&original);
    let error = consumer.finalize_module_from_state().unwrap_err().to_string();
    assert!(error.contains("semantic type has no name"), "{error}");
}
