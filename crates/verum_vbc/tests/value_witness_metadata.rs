#![cfg(feature = "codegen")]
//! Imported callable roles remain bound to their actual source declarations.
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenResult, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
};

const API: &str = r#"module api;
public type Cell<N: meta Int> is { public value: () };
public fn size<N: meta Int>(cell: Cell<N>) -> Int { N }
implement<N: meta Int> Cell<N> { public fn count(&self) -> Int { N } }
"#;
fn imported(mutate: impl FnOnce(&mut VbcModule)) -> CodegenResult<VbcModule> {
    let api = Parser::new(API).parse_module().expect("API grammar");
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("api"));
    let mut archive = producer.collect_declaration_interface(&[&api])?;
    let functions = producer.export_functions();
    mutate(&mut archive);
    let archive = deserialize_module(&serialize_module(&archive).unwrap()).unwrap();
    let consumer = Parser::new(
        r#"module consumer; mount api.{Cell,size};
        fn probe(cell: Cell<7>) -> Int { size(cell) + cell.count() }
    "#,
    )
    .parse_module()
    .expect("consumer grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.import_functions(&functions);
    codegen.import_bootstrap_nominal_dependencies(&[&consumer], &[&archive])?;
    codegen.collect_unit_declarations(&[&consumer])?;
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&consumer], ItemFailurePolicy::Strict)?;
    codegen.finalize_module_from_state()
}
#[test]
fn imported_source_roles_preserve_inferred_values() {
    let module = imported(|_| ()).expect("actual imported generic source compiles");
    let entry = module.find_function_by_name("consumer.probe").unwrap();
    let declaration = module.get_function(entry).unwrap();
    let mut cursor = declaration.bytecode_offset as usize;
    let end = cursor + declaration.bytecode_length as usize;
    let mut witnesses = Vec::new();
    while cursor < end {
        match verum_vbc::bytecode::decode_instruction(&module.bytecode, &mut cursor).unwrap() {
            verum_vbc::instruction::Instruction::CallG { type_args, .. }
            | verum_vbc::instruction::Instruction::SetCallWitness { type_args } => {
                witnesses.push(type_args)
            }
            _ => (),
        }
    }
    assert_eq!(
        witnesses,
        vec![vec![verum_vbc::types::TypeRef::ConstValue(7)]; 2],
        "both the imported free callable and receiver own exact value witnesses"
    );
}
#[test]
fn erased_value_parameter_header_is_refused() {
    let error = imported(|module| {
        let id = module.find_function_by_name("size").unwrap();
        let declaration = module.get_function_mut(id).unwrap();
        assert!(declaration.source_signature.take().is_some());
    })
    .expect_err("erasure cannot reinterpret a value slot as a type slot");
    assert!(
        format!("{error:?}").contains("lacks its source-owned parameter roles"),
        "{error:?}"
    );
}
#[test]
fn erased_owner_receiver_evidence_is_refused() {
    let error = imported(|module| {
        let id = module.find_function_by_name("Cell.count").unwrap();
        let declaration = module.get_function_mut(id).unwrap();
        assert!(declaration.semantic_params.take().is_some());
    })
    .expect_err("the receiver declaration owns its generic value role");
    assert!(
        format!("{error:?}").contains("lacks its declaration-owned semantic receiver"),
        "{error:?}"
    );
}
