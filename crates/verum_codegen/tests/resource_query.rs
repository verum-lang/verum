//! T1807: native ownership queries use the same declared witnesses as the interpreter.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::{AnyValue, CallSiteValue},
};
use verum_vbc::{
    codegen::VbcCodegen,
    mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
};

fn execute(source: &str) -> i64 {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut module = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    // TypeResource is compiler-internal. Preserve actual source call witnesses,
    // replacing the single branch-free query in this test module.
    let original = std::mem::take(&mut module.bytecode);
    for function in &mut module.functions {
        let mut body = verum_vbc::bytecode::decode_instructions(
            &original[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        for instruction in &mut body {
            if let verum_vbc::Instruction::TypeLayout { dst, type_ref, .. } = instruction {
                *instruction = verum_vbc::Instruction::TypeResource {
                    dst: *dst,
                    type_ref: type_ref.clone(),
                };
            }
        }
        function.bytecode_offset = module.bytecode.len() as u32;
        for instruction in &body {
            verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        function.instructions = Some(body);
        function.value_uses = None;
    }
    // The compiler phase recovers this derived flag from the declaration roster.
    for function in &mut module.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    let mut graph = InstantiationGraph::new();
    for function in &module.functions {
        let body = &module.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        let instructions = verum_vbc::bytecode::decode_instructions(body).unwrap();
        discover_call_instantiations(&module, &instructions, function.func_id_base, &mut graph)
            .unwrap();
    }
    assert!(
        !graph.is_empty(),
        "source must require actual generic specialization: {source}"
    );
    let module = monomorphize_minimal(module, &graph)
        .expect("source mono")
        .module;
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("resource_query").with_debug_info(false),
    );
    lowering.lower_module(&module).expect("native lowering");
    // Keep the exact reachable native graph: no runtime replacement or inferred
    // answer in this harness. Unused erased generic bodies need not execute.
    let mut pending = List::new();
    pending.push(
        lowering
            .module()
            .get_function("probe")
            .expect("source probe"),
    );
    let mut seen = Set::<Text>::new();
    let mut ir = Text::new();
    while let Some(function) = pending.pop() {
        let name = Text::from(function.get_name().to_str().unwrap());
        if !seen.insert(name) {
            continue;
        }
        ir.push_str(function.print_to_string().to_str().unwrap());
        ir.push('\n');
        for block in function.get_basic_blocks() {
            for instruction in block.get_instructions() {
                if let Ok(call) = CallSiteValue::try_from(instruction)
                    && let Some(callee) = call.get_called_fn_value()
                {
                    pending.push(callee);
                }
            }
        }
    }
    assert!(
        !ir.contains("verum_internal_exit"),
        "concrete witnesses must resolve: {ir}"
    );
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let native = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "resource_query",
        ))
        .unwrap();
    native.verify().expect("verified source/mono IR");
    let jit = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
            .unwrap()
            .call()
    }
}

#[test]
fn specialized_native_query_agrees_for_owned_ordinary_and_borrowed_types() {
    use verum_common::ResourceDiscipline as D;
    for (ty, expected) in [
        ("Int", D::Unrestricted),
        ("Socket", D::Affine),
        ("Ticket", D::Linear),
        ("Holder<Socket>", D::Affine),
        ("Holder<Int>", D::Unrestricted),
        ("&Socket", D::Unrestricted),
        ("&checked Socket", D::Unrestricted),
        ("&unsafe Socket", D::Unrestricted),
    ] {
        let source = format!(
            r#"
type affine Socket is {{ fd: Int }};
type linear Ticket is {{ id: Int }};
type Holder<T> is {{ value: T }};
fn discipline<T>() -> Int {{ T.size }}
fn outer<U>() -> Int {{ discipline<U>() }}
fn probe() -> Int {{ outer<{ty}>() }}
"#
        );
        assert_eq!(execute(&source), expected as u8 as i64, "{ty}");
    }
}
