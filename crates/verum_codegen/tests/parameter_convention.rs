//! Native calls need the actual selected body convention; callable cleanup refuses.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, serialize::serialize_module,
};

fn compile(source: &str) -> verum_vbc::VbcModule {
    let module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap();
    let mut wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    wire
}
#[test]
fn native_mixed_parameter_convention_is_refused_before_emission_of_the_call() {
    let mut module = compile("fn take(value: Int) -> Int { value } fn main() -> Int { take(73) }");
    let take = module.find_function_by_name("take").unwrap();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.id == take)
        .unwrap();
    function.owned_value_parameters = false;
    function.value_uses = None;
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("mixed_convention").with_debug_info(false),
    );
    let error = lowering.lower_module(&module).unwrap_err();
    assert!(
        error.to_string().contains("incompatible owned parameter"),
        "{error}"
    );
}
#[test]
fn native_callable_cleanup_has_an_explicit_executable_refusal() {
    let module = compile(
        "fn known() -> Int { 73 } fn dispose(selected: fn() -> Int) {} fn main() -> Int { dispose(known); 73 }",
    );
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("callable_cleanup").with_debug_info(false),
    );
    lowering.lower_module(&module).unwrap();
    lowering.module().verify().unwrap();
    let ir = lowering.module().print_to_string().to_string();
    assert!(
        ir.contains(
            "native callable cleanup requires a producer-issued environment ownership receipt"
        ),
        "{ir}"
    );
    let body = lowering
        .module()
        .get_function("dispose")
        .unwrap()
        .print_to_string()
        .to_string();
    assert!(body.contains("unreachable"), "{body}");
}
