//! T1576: source-declared layout witnesses survive real mono and native lowering.
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
        LoweringConfig::debug("generic_layout").with_debug_info(false),
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
            "generic_layout",
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
fn byte_int_and_nested_caller_layout_witnesses_execute_natively() {
    assert_eq!(
        execute("fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }"),
        1
    );
    assert_eq!(
        execute("fn size<T>() -> Int { T.size } fn probe() -> Int { size<Int>() }"),
        8
    );
    assert_eq!(
        execute(
            "fn size<T>() -> Int { T.size } fn outer<U>() -> Int { size<U>() } fn probe() -> Int { outer<Byte>() }"
        ),
        1
    );
}

#[test]
fn native_record_and_c_layouts_use_declaration_facts() {
    assert_eq!(
        execute(
            "type Triple is { a: Int, b: Int, c: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Triple>() }"
        ),
        24
    );
    assert_eq!(
        execute(
            "@repr(C) type Pair is { a: Byte, b: Int32 }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Pair>() }"
        ),
        8
    );
}

#[test]
fn native_shadowed_names_do_not_select_unrelated_layouts() {
    assert_eq!(
        execute(
            "type T is { a: Int, b: Int, c: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }"
        ),
        1
    );
    assert_eq!(
        execute(
            "type Byte is { a: Int, b: Int }; fn size<T>() -> Int { T.size } fn probe() -> Int { size<Byte>() }"
        ),
        16
    );
}

#[test]
fn native_structural_reference_witnesses_survive_nested_calls() {
    for (ty, expected) in [
        ("&Byte", 16),
        ("&checked Byte", 16),
        ("&unsafe Byte", 8),
        ("&[Byte]", 32),
    ] {
        assert_eq!(
            execute(&format!(
                "type Shape is {ty}; fn size<T>() -> Int {{ T.size }} fn outer<U>() -> Int {{ size<U>() }} fn probe() -> Int {{ outer<Shape>() }}"
            )),
            expected,
            "{ty}"
        );
    }
    assert_eq!(
        execute("fn size<T>() -> Int { (&T).size } fn probe() -> Int { size<Byte>() }"),
        16
    );
}
