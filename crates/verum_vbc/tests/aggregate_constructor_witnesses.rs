//! Declaration witnesses must reach constructors inside aggregate initializers.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

fn check(source: &str, constructor: &str) {
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("witnesses"))
        .compile_module(&ast)
        .unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let function = module
        .functions
        .iter()
        .find(|f| module.strings.get(f.name) == Some(constructor))
        .unwrap();
    let body = decode_instructions(
        &module.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize],
    )
    .unwrap();
    let witnesses: Vec<_> = body
        .iter()
        .filter_map(|op| match op {
            Instruction::CallG { type_args, .. } => Some(type_args),
            _ => None,
        })
        .collect();
    assert!(!witnesses.is_empty(), "{body:?}");
    assert!(
        witnesses
            .iter()
            .all(|args| args.iter().all(|ty| !ty.is_generic())),
        "{witnesses:?}"
    );
    let probe = module.find_function_by_name("witnesses.probe").unwrap();
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    assert_eq!(interpreter.execute_function(probe).unwrap().as_i64(), 42);
}

const TYPES: &str = r#"
module witnesses;
type Holder<T> is { item: T };
implement<T> Holder<T> {
    fn new(item: T) -> Holder<T> { Holder { item: item } }
}
type Choice<T> is Empty | Value(T);
"#;

#[test]
fn record_fields_supply_nested_constructor_witnesses() {
    check(
        &(TYPES.to_owned()
            + r#"
        type State is { reason: Holder<Choice<()>> };
        implement State {
            fn new() -> State { State { reason: Holder.new(Choice.Empty) } }
        }
        fn probe() -> Int {
            let state = State.new();
            match state.reason.item { Choice.Empty => 42, Choice.Value(_) => 0 }
        }
    "#),
        "State.new",
    );
}

#[test]
fn deep_static_constructor_retains_unit_witness() {
    check(
        &(TYPES.to_owned()
            + r#"
        type Task<T> is { result: Choice<T> };
        static ROOT: Holder<Choice<Task<()>>> = Holder.new(Choice.Empty);
        fn probe() -> Int { match ROOT.item { Choice.Empty => 42, Choice.Value(_) => 0 } }
    "#),
        "__tls_init_ROOT",
    );
}
