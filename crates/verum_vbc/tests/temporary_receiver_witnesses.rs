//! Chained calls carry witnesses from the temporary receiver's declaration.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::Map;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    types::{TypeId, TypeRef},
    value::Value,
};

#[test]
fn imported_callable_signatures_preserve_references_without_inventing_evidence() {
    let source = Parser::new(
        r#"
module producer;
type Payload is { value: Int };
type Factory is {};
implement Factory { fn borrow(value: &Payload) -> &Payload { value } }
"#,
    )
    .parse_module()
    .unwrap();
    let mut provider = VbcCodegen::with_config(CodegenConfig::new("producer"));
    let dependency = provider.compile_module(&source).unwrap();
    let source_payload = dependency
        .types
        .iter()
        .find(|ty| dependency.strings.get(ty.name) == Some("Payload"))
        .unwrap()
        .id;
    let source = Parser::new("module consumer; mount producer.{Factory, Payload};")
        .parse_module()
        .unwrap();
    for has_signature in [true, false] {
        let mut exports = provider.export_functions();
        if !has_signature {
            for info in exports.values_mut() {
                info.callable_signature = None;
            }
        }
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        codegen.import_functions(&exports);
        codegen
            .import_bootstrap_nominal_dependencies(&[&source], &[&dependency])
            .unwrap();
        codegen.collect_unit_declarations(&[&source]).unwrap();
        let module = codegen.compile_function_bodies(&source).unwrap();
        let payload = module
            .types
            .iter()
            .find(|ty| module.strings.get(ty.name) == Some("Payload"))
            .unwrap()
            .id;
        assert_ne!(
            source_payload, payload,
            "fixture must change the nominal pool"
        );
        let exports = codegen.export_functions();
        let signature = &exports["producer.Factory.borrow"].callable_signature;
        if has_signature {
            use verum_vbc::types::{CbgrTier, Mutability};
            let borrowed = TypeRef::Reference {
                inner: Box::new(TypeRef::Concrete(payload)),
                tier: CbgrTier::Tier0,
                mutability: Mutability::Immutable,
            };
            assert_eq!(
                *signature,
                Some(TypeRef::Function {
                    params: vec![borrowed.clone()],
                    return_type: Box::new(borrowed),
                    contexts: Default::default(),
                })
            );
        } else {
            assert!(
                signature.is_none(),
                "ABI types cannot establish source evidence"
            );
        }
    }
}

#[test]
fn imported_temporary_receiver_and_callback_supply_all_declared_witnesses() {
    let source = Parser::new(
        r#"
module producer;
type Outcome<T, E> is Good(T) | Bad(E);
implement<T, E> Outcome<T, E> {
    fn map_error<F, O: fn(E) -> F>(self, f: O) -> Outcome<T, F> {
        match self { Good(value) => Good(value), Bad(error) => Bad(f(error)) }
    }
}
type Factory is {};
implement Factory {
    fn decode(fail: Bool) -> Outcome<Int, Bool> {
        if fail { Outcome.Bad(false) } else { Outcome.Good(7) }
    }
}
"#,
    )
    .parse_module()
    .unwrap();
    let mut provider = VbcCodegen::with_config(CodegenConfig::new("producer"));
    let module = provider.compile_module(&source).unwrap();
    let dependency = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let source = Parser::new(
        r#"
module consumer;
mount producer.{Factory, Outcome};
fn probe(fail: Bool) -> Int {
    let value = Factory.decode(fail).map_error(|_| 11);
    match value { Good(value) => value, Bad(error) => error }
}
"#,
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.import_functions(&provider.export_functions());
    codegen.import_type_layouts(&provider.export_type_layouts());
    codegen.import_type_field_names(&provider.export_type_field_names());
    codegen
        .import_bootstrap_nominal_dependencies(&[&source], &[&dependency])
        .unwrap();
    codegen.collect_unit_declarations(&[&source]).unwrap();
    let module = codegen.compile_function_bodies(&source).unwrap();

    let mut pc = 0;
    let mut calls = 0;
    let mut pending = None;
    while pc < module.bytecode.len() {
        let type_args =
            match verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut pc).unwrap() {
                verum_vbc::Instruction::SetCallWitness { type_args } => {
                    pending = Some(type_args);
                    None
                }
                verum_vbc::Instruction::CallM { method_id, .. }
                    if module
                        .get_string(verum_vbc::types::StringId(method_id))
                        .is_some_and(|name| name.ends_with("map_error")) =>
                {
                    Some(pending.take().expect("method witnesses"))
                }
                verum_vbc::Instruction::CallG {
                    func_id, type_args, ..
                } if module
                    .band_reference_name(func_id)
                    .is_some_and(|name| name.ends_with("Outcome.map_error")) =>
                {
                    Some(type_args)
                }
                _ => None,
            };
        if let Some(type_args) = type_args {
            calls += 1;
            assert!(!type_args.iter().any(|ty| ty.is_generic()), "{type_args:?}");
            assert_eq!(type_args.len(), 4, "{type_args:?}");
            assert_eq!(
                &type_args[..3],
                &[
                    TypeRef::Concrete(TypeId::INT),
                    TypeRef::Concrete(TypeId::BOOL),
                    TypeRef::Concrete(TypeId::INT)
                ]
            );
            assert_eq!(
                type_args[3],
                TypeRef::Function {
                    params: vec![TypeRef::Concrete(TypeId::BOOL)],
                    return_type: Box::new(TypeRef::Concrete(TypeId::INT)),
                    contexts: Default::default(),
                }
            );
        }
    }
    assert_eq!(calls, 1, "selected imported generic call");

    let functions = codegen.export_functions();
    let remap: Map<_, _> = dependency
        .functions
        .iter()
        .map(|function| {
            let name = dependency.strings.get(function.name).unwrap();
            (function.id.0, functions[name].id)
        })
        .collect();
    assert!(codegen.merge_archive_function_bodies(&dependency, &remap.into()) > 0);
    let mut module = codegen.finalize_module_from_state().unwrap();
    for function in &mut module.functions {
        function.value_uses = None;
    }
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let entry = module.find_function_by_name("consumer.probe").unwrap();
    for (fail, expected) in [(false, 7), (true, 11)] {
        assert_eq!(
            Interpreter::new(Arc::new(module.clone()))
                .execute_function_with_args(entry, &[Value::from_bool(fail)])
                .unwrap()
                .as_i64(),
            expected
        );
    }
}

#[test]
fn transformation_of_a_temporary_result_retains_payload_types() {
    for producer in ["Outcome.Good(7)", "Outcome.Bad(false)"] {
        let source = format!(
            r#"
type Outcome<T, E> is Good(T) | Bad(E);
implement<T, E> Outcome<T, E> {{
    fn map_error<F, O: fn(E) -> F>(self, f: O) -> Outcome<T, F> {{
        match self {{ Good(value) => Good(value), Bad(error) => Bad(f(error)) }}
    }}
}}
fn make() -> Outcome<Int, Bool> {{ {producer} }}
fn probe() -> Int {{
    let mapped = make().map_error(|_| 7);
    match mapped {{ Good(value) => value, Bad(error) => error }}
}}
"#
        );
        for source in [
            source.clone(),
            source
                .replace(
                    "fn make() -> Outcome<Int, Bool>",
                    "type Factory is {}; implement Factory { fn make() -> Outcome<Int, Bool>",
                )
                .replace("fn probe()", "} fn probe()")
                .replace("make().", "Factory.make()."),
        ] {
            for source in [
                source.clone(),
                source
                    .replace("Outcome", "Result")
                    .replace("Good", "Ok")
                    .replace("Bad", "Err")
                    .replace("map_error", "map_err"),
            ] {
                let ast = Parser::new(&source).parse_module().unwrap();
                for bootstrap in [false, true] {
                    let mut codegen =
                        VbcCodegen::with_config(CodegenConfig::new("temporary_receiver"));
                    let mut module = if bootstrap {
                        codegen.collect_unit_declarations(&[&ast]).unwrap();
                        codegen
                            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                            .unwrap();
                        codegen.finalize_module_from_state().unwrap()
                    } else {
                        codegen.compile_module(&ast).unwrap()
                    };
                    for function in &mut module.functions {
                        function.value_uses = None;
                    }
                    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
                    let entry = module
                        .find_function_by_name("temporary_receiver.probe")
                        .unwrap();
                    let result = Interpreter::new(Arc::new(module)).execute_function(entry);
                    assert_eq!(
                        result.unwrap().as_i64(),
                        7,
                        "{producer}, bootstrap={bootstrap}"
                    );
                }
            }
        }
    }
}
