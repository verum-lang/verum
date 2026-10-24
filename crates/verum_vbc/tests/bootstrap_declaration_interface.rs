//! Forward bootstrap calls use declaration metadata before a dependency has a body.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, VbcModule,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
    module::FunctionId,
    types::{TypeId, TypeRef},
};

const PROVIDER: &str = r#"
module core.provider;
public type Holder<T> is { item: T };
implement<T> Holder<T> {
    public fn new(item: T) -> Holder<T> { Holder { item: item } }
}
"#;
const CONSUMER: &str = r#"
module core.consumer;
mount core.provider.Holder;
type Choice<T> is Empty | Value(T);
static ROOT: Holder<Choice<()>> = Holder.new(Choice.Empty);
fn probe() -> Int { match ROOT.item { Choice.Empty => 42, Choice.Value(_) => 0 } }
"#;

#[test]
fn declaration_interface_supplies_forward_constructor_witnesses() {
    check_forward_constructor(false);
}

#[test]
fn declaration_interface_preserves_reexported_constructor_witnesses() {
    check_forward_constructor(true);
}

fn check_forward_constructor(reexported: bool) {
    let provider = Parser::new(PROVIDER).parse_module().unwrap();
    let source = if reexported {
        CONSUMER.replace("core.provider.Holder", "core.exports.Holder")
    } else { CONSUMER.to_owned() };
    let consumer = Parser::new(&source).parse_module().unwrap();
    let mut declarations = VbcCodegen::with_config(CodegenConfig::new("core.provider"));
    let interface = declarations
        .collect_declaration_interface(&[&provider])
        .unwrap();
    assert!(interface.bytecode.is_empty());
    assert!(interface.global_ctors.is_empty());
    assert!(interface.functions.iter().all(|f| f.bytecode_length == 0));
    let mut stub = declarations
        .export_functions()
        .get("Holder.new")
        .unwrap()
        .clone();
    stub.id = FunctionId(verum_vbc::stub_ranges::STAGE1_BASE);
    stub.return_type = None;
    stub.callable_signature = None;
    let functions = [
        ("Holder.new".to_owned(), stub.clone()),
        ("core.provider.Holder.new".to_owned(), stub),
    ]
    .into_iter()
    .collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.consumer"));
    codegen.import_functions(&functions);
    codegen
        .import_bootstrap_nominal_dependencies_with_resolver(&[&consumer], &[&interface], |name| {
            Some(if name == "core.exports.Holder" { "core.provider.Holder" } else { name }.into())
        })
        .unwrap();
    codegen.collect_unit_declarations(&[&consumer]).unwrap();
    codegen
        .compile_unit_items(&[&consumer], ItemFailurePolicy::Strict)
        .unwrap();
    // This body is deliberately emitted after its consumer's initializer.
    codegen
        .compile_unit_items(&[&provider], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let module: VbcModule = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let init = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some("__tls_init_ROOT"))
        .unwrap();
    let body = decode_instructions(
        &module.bytecode
            [init.bytecode_offset as usize..(init.bytecode_offset + init.bytecode_length) as usize],
    )
    .unwrap();
    let witnesses: List<_> = body
        .iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(witnesses.len(), 1, "{body:?}");
    assert!(
        matches!(&witnesses[0][0], TypeRef::Instantiated { args, .. }
        if args == &[TypeRef::Concrete(TypeId::UNIT)]),
        "{witnesses:?}"
    );
    let probe = module.find_function_by_name("core.consumer.probe").unwrap();
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    assert_eq!(interpreter.execute_function(probe).unwrap().as_i64(), 42);
}
