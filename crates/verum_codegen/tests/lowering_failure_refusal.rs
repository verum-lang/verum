//! Default AOT lowering must preserve failures, including startup failures.
//! The unsupported operation comes from ordinary source compilation. Malformed
//! constant references exercise the executable-input boundary; their producer
//! descriptors and independently decoded wire must report the same error.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{context::Context, values::AnyValue};
use verum_vbc::{
    codegen::VbcCodegen,
    instruction::{Instruction, Reg},
    module::VbcModule,
};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("source grammar"))
        .expect("source VBC")
}

fn routes(mut source: VbcModule) -> [VbcModule; 2] {
    source.bytecode.clear();
    for descriptor in &mut source.functions {
        descriptor.bytecode_offset = source.bytecode.len() as u32;
        if let Some(body) = &descriptor.instructions {
            verum_vbc::bytecode::encode_instructions_with_fixup(body, &mut source.bytecode);
        }
        descriptor.bytecode_length = source.bytecode.len() as u32 - descriptor.bytecode_offset;
    }
    let bytes = verum_vbc::serialize::serialize_module(&source).expect("source serialization");
    let mut wire = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire decode");
    for descriptor in &mut wire.functions {
        let start = descriptor.bytecode_offset as usize;
        let end = start + descriptor.bytecode_length as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end])
            .expect("independent body decode");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        descriptor.instructions = Some(body);
    }
    [source, wire]
}

#[test]
fn unsupported_body_is_not_a_successful_zero_result() {
    for module in routes(compile("fn main() -> Int { @vbc(TENSOR_SLICE, 0, 0) }")) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("unsupported-body").with_debug_info(false),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("unsupported source operation must refuse AOT in default mode");
        assert!(
            error
                .to_string()
                .contains("Unimplemented TensorExtended sub_op: 0x19"),
            "wrong refusal: {error}"
        );
    }
}

fn invalid_constant_body(constructor: bool) -> VbcModule {
    let mut module = compile(if constructor {
        "fn __tls_init_probe() {} fn main() -> Int { 17 }"
    } else {
        "fn broken() -> Int { 1 } fn main() -> Int { broken() }"
    });
    let name = if constructor {
        "__tls_init_probe"
    } else {
        "broken"
    };
    let id = module
        .find_function_by_name(name)
        .expect("selected fixture declaration");
    let descriptor = module.get_function_mut(id).unwrap();
    descriptor.register_count = descriptor.register_count.max(1);
    descriptor.instructions.as_mut().unwrap().insert(
        0,
        Instruction::LoadK {
            dst: Reg(0),
            const_id: u32::MAX,
        },
    );
    if constructor {
        module.global_ctors.push((id, 100));
    }
    module
}

#[test]
fn invalid_body_error_is_propagated_without_a_strictness_flag() {
    for module in routes(invalid_constant_body(false)) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("invalid-body").with_debug_info(false),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("malformed executable input cannot become a successful stub");
        assert_eq!(
            error.to_string(),
            "Internal error: LoadK: constant 4294967295 not found in pool"
        );
    }
}

#[test]
fn failed_constructor_is_not_erased_and_replaced_with_a_zero_body() {
    for module in routes(invalid_constant_body(true)) {
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("invalid-constructor").with_debug_info(false),
        );
        let error = lower
            .lower_module(&module)
            .expect_err("constructor failure must refuse compilation");
        assert_eq!(
            error.to_string(),
            "Internal error: LoadK: constant 4294967295 not found in pool"
        );
    }
}

#[test]
fn declared_foreign_function_keeps_external_linkage() {
    for module in routes(compile(
        "extern \"C\" { fn external_dependency(value: Int) -> Int; } fn main() -> Int { external_dependency(17) }",
    )) {
        assert!(
            module
                .ffi_symbols
                .iter()
                .any(|symbol| module.strings.get(symbol.name) == Some("external_dependency")),
            "ordinary FFI producer must register the foreign declaration"
        );
        let context = Context::create();
        let mut lower = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("declared-foreign-function").with_debug_info(false),
        );
        lower
            .lower_module(&module)
            .expect("declared foreign linkage");
        let foreign = lower
            .module()
            .get_function("external_dependency")
            .expect("actual foreign declaration");
        assert_eq!(
            foreign.count_basic_blocks(),
            0,
            "foreign linkage cannot receive a replacement implementation: {}",
            foreign.print_to_string()
        );
        lower.module().verify().expect("valid native module");
    }
}
