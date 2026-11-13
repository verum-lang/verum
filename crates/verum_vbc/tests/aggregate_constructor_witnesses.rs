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

fn check(source: &str, constructor: &str) -> Arc<verum_vbc::module::VbcModule> {
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("witnesses"))
        .compile_module(&ast)
        .unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&module).expect("all witnesses belong to their caller");
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
    let module = Arc::new(module);
    let mut interpreter = Interpreter::new(module.clone());
    interpreter.run_global_ctors().unwrap();
    assert_eq!(interpreter.execute_function(probe).unwrap().as_i64(), 42);
    module
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

#[test]
fn record_parameter_reaches_a_nested_nullary_constructor() {
    check(
        &(TYPES.to_owned() + r#"
        type State<Item> is { reason: Holder<Choice<Item>> };
        fn probe() -> Int {
            let state: Holder<State<Int>> = Holder.new(State { reason: Holder.new(Choice.Empty) });
            match state.item.reason.item { Choice.Empty => 42, Choice.Value(_) => 0 }
        }
    "#),
        "witnesses.probe",
    );
}

#[test]
fn record_annotation_supplies_its_own_field_context() {
    check(
        &(TYPES.to_owned() + r#"
        type State<Item> is { item: Holder<Choice<Item>> };
        fn probe() -> Int {
            let state: State<Bool> = State { item: Holder.new(Choice.Empty) };
            match state.item.item { Choice.Empty => 42, Choice.Value(_) => 0 }
        }
    "#),
        "witnesses.probe",
    );
}

#[test]
fn record_fields_use_instantiated_arguments_even_when_caller_slots_collide() {
    use verum_vbc::types::TypeRef;

    let module = check(
        &(TYPES.to_owned() + r#"
        type Envelope<Noise, Item> is { item: Holder<Choice<Item>>, noise: Noise };
        fn build<Item, Noise>(noise: Noise, _sample: Item) -> Envelope<Noise, Item> {
            Envelope { item: Holder.new(Choice.Empty), noise }
        }
        fn probe() -> Int {
            let result = build(42, true);
            match result.item.item { Choice.Empty => result.noise, Choice.Value(_) => 0 }
        }
    "#),
        "witnesses.probe",
    );
    let build = module.get_function(module.find_function_by_name("witnesses.build").unwrap()).unwrap();
    let item = build.type_params.iter().find(|param| module.get_string(param.name) == Some("Item")).unwrap();
    let choice = module.types.iter().find(|ty| module.get_string(ty.name) == Some("Choice")).unwrap();
    let body = decode_instructions(&module.bytecode[build.bytecode_offset as usize
        ..(build.bytecode_offset + build.bytecode_length) as usize]).unwrap();
    assert!(body.iter().any(|op| matches!(op, Instruction::CallG { type_args, .. }
        if matches!(type_args.as_slice(), [TypeRef::Instantiated { base, args }] if *base == choice.id
            && args.as_slice() == [TypeRef::Generic(item.id)]))), "{body:?}");
}
