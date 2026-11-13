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

fn specialize(mut module: verum_vbc::VbcModule) -> verum_vbc::VbcModule {
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
        "source must require actual generic specialization"
    );
    monomorphize_minimal(module, &graph)
        .expect("source mono")
        .module
}

fn execute(source: &str) -> i64 {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let module = specialize(VbcCodegen::new().compile_module(&ast).expect("source VBC"));
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

#[test]
fn native_signedness_uses_a_boolean_from_the_specialized_declaration() {
    let source = r#"
        type Count is UInt64;
        fn signed<T>() -> Bool { T.is_signed }
        fn outer<U>() -> Bool { signed<U>() }
        fn probe() -> Int {
            if outer<ISize>() && outer<Int8>() && outer<Float>() && !outer<USize>() && !outer<Count>() { 1 } else { 0 }
        }
    "#;
    assert_eq!(execute(source), 1);
}

#[cfg(target_os = "macos")]
#[test]
fn sequence_witnesses_execute_in_a_native_image_after_serialization() {
    use std::process::Command;
    use verum_llvm::targets::{CodeModel, FileType, RelocMode, TargetMachine};
    use verum_vbc::{deserialize::deserialize_module, serialize::serialize_module};

    let source = "fn digits<const Shape: [Int]>() -> Int { Shape[0] * 10 + Shape[1] }
                  fn via<const Values: [Int]>() -> Int { digits<Values>() }
                  fn main() -> Int { via<[3; 2]>() + digits<[4,-1]>() }";
    let ast = Parser::new(source).parse_module().unwrap();
    let encoded = serialize_module(&VbcCodegen::new().compile_module(&ast).unwrap()).unwrap();
    let mut wire = deserialize_module(&encoded).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    let specialized = specialize(wire);
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple).unwrap().create_target_machine(
        &triple, "generic", "", OptimizationLevel::Default, RelocMode::PIC, CodeModel::Default,
    ).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(&context, LoweringConfig::debug("sequence_witness").with_debug_info(false));
    lowering.lower_module(&specialized).unwrap();
    let native = lowering.module();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let object = dir.path().join("sequence.o");
    let executable = dir.path().join("sequence");
    machine.write_to_file(native, FileType::Object, &object).unwrap();
    let link = Command::new("cc").arg(object).args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
        .arg(&executable).output().unwrap();
    assert!(link.status.success(), "{}", String::from_utf8_lossy(&link.stderr));
    let run = Command::new(executable).output().unwrap();
    assert_eq!(run.status.code(), Some(72), "{}", String::from_utf8_lossy(&run.stderr));
}

#[test]
fn projected_member_layout_specializes_nested_generic_calls() {
    assert_eq!(execute(r#"
        type Measure is protocol { type Product; };
        type Numeric is ();
        type Logical is ();
        implement Measure for Numeric { type Product = Int; }
        implement Measure for Logical { type Product = Bool; }
        fn size<T>() -> Int { T.size }
        fn measured<F: Measure>() -> Int { size<F.Product>() }
        fn probe() -> Int { 10 * measured<Numeric>() + measured<Logical>() }
    "#), 81);
}
