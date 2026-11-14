//! Retaining nominal lifecycle metadata does not generalize its typed receiver.
#![cfg(feature = "codegen")]
use verum_ast::{ItemKind, Module};
use verum_common::Map;
use verum_fast_parser::Parser;
use verum_vbc::{
    FunctionId, TypeRef, VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen, context::FunctionInfo},
    deserialize::deserialize_module,
    serialize::serialize_module,
    types::TypeId,
};

fn wire(module: &VbcModule) -> VbcModule {
    let module = deserialize_module(&serialize_module(module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    module
}

fn protocol() -> Module {
    let mut source = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    source.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Drop")
    });
    assert_eq!(source.items.len(), 2);
    source
}

#[test]
fn an_archived_drop_slot_retains_its_full_generic_receiver() {
    let protocol = protocol();
    let source = Parser::new(
        r#"
        module app;
        mount core.base.protocols.Drop;
        type Token<T> is { value:T };
        implement Drop for Token<Int> {
            fn drop(&mut self)->Unit { assert_eq(self.value,73); }
        }
        fn main()->Int { { let value: Token<Int> = Token{value:73}; } 73 }
        fn failing()->Int { { let value: Token<Int> = Token{value:74}; } 73 }
    "#,
    )
    .parse_module()
    .unwrap();
    let units = [&protocol, &source];
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("app"));
    producer.collect_unit_declarations(&units).unwrap();
    producer
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let archived = wire(&producer.finalize_module_from_state().unwrap());
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("app"));
    consumer.import_archive_module_types(&archived);
    let mut remap = Map::new();
    for function in &archived.functions {
        let id = FunctionId(5000 + function.id.0);
        let owner = function
            .origin_module
            .and_then(|id| archived.get_string(id))
            .unwrap_or(&archived.name);
        let name = verum_vbc::module::qualify_module_name(
            owner,
            archived.get_string(function.name).unwrap(),
        );
        consumer.ctx_mut().register_function(
            name,
            FunctionInfo {
                id,
                param_count: function.params.len(),
                ..Default::default()
            },
        );
        consumer.record_archive_function_descriptor(&archived, function, id);
        remap.insert(function.id.0, id);
    }
    consumer.merge_archive_function_bodies(&archived, &remap.into_iter().collect());
    // Rebuild the exact nominal header, independently of whole-method replay.
    let mut header = source.clone();
    header.items.retain(|item| {
        matches!(
            &item.kind,
            ItemKind::Module(_) | ItemKind::Type(_) | ItemKind::Mount(_)
        )
    });
    consumer.collect_unit_declarations(&[&header]).unwrap();
    let completed = wire(&consumer.finalize_module_from_state().unwrap());
    for module in [archived, completed] {
        let token = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Token")
                    && ty.origin_module.and_then(|owner| module.get_string(owner)) == Some("app")
            })
            .unwrap();
        let selected = module.resolved_function_id(token.drop_fn.unwrap());
        let exact = TypeRef::instantiated(token.id, [TypeRef::Concrete(TypeId::I64)]);
        let foreign = TypeRef::instantiated(token.id, [TypeRef::Concrete(TypeId::BOOL)]);
        let plan = verum_vbc::drop_plan::build(&module, &exact).unwrap();
        assert!(
            matches!(plan,verum_vbc::drop_plan::DropPlan::Glue{function,..}if function==selected)
        );
        let error = verum_vbc::drop_plan::build(&module, &foreign).unwrap_err();
        assert!(
            error.contains("receiver belongs to another owner"),
            "{error}"
        );
        let entry = module.find_function_by_name("app.main").unwrap();
        let failing = module.find_function_by_name("app.failing").unwrap();
        let mut vm = verum_vbc::interpreter::Interpreter::new(module.into());
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
        let error = vm.execute_function(failing).unwrap_err();
        assert!(
            matches!(
                error,
                verum_vbc::interpreter::InterpreterError::AssertionFailed { .. }
            ),
            "{error}"
        );
    }
}
