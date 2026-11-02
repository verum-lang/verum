//! The process shim and native reachability share the selected application declaration.
use std::sync::Arc;
use verum_ast::FileId;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::FastParser;
use verum_llvm::{
    OptimizationLevel, context::Context, memory_buffer::MemoryBuffer, targets::Target,
    values::AnyValue,
};
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, module::EntryMain};

#[test]
fn selected_root_executes_in_vm_and_native_code_among_two_dependency_mains() {
    let parser = FastParser::new();
    let files = [
        (
            1,
            "module first.main; public fn answer() -> Int { 42 } fn main() -> Int { 73 }",
        ),
        (
            2,
            "module consumer.main; mount first.main.answer; fn main() -> Int { answer() }",
        ),
        (3, "module second.main; fn main() -> Int { 91 }"),
    ]
    .map(|(id, source)| parser.parse_module_str(source, FileId::new(id)).unwrap());
    let mut codegen = VbcCodegen::new();
    codegen
        .collect_unit_declarations(&files.each_ref())
        .unwrap();
    codegen.resolve_pending_imports();
    for file in &files {
        codegen.compile_module_items(file).unwrap();
    }
    let module = Arc::new(codegen.finalize_module().unwrap());
    let owner = "consumer.main.main";
    let EntryMain::Unique { index } = module.entry_main_for(Some(owner)) else {
        panic!("expected the application main")
    };
    let value = Interpreter::new(module.clone())
        .execute_function(module.functions[index].id)
        .unwrap();
    assert_eq!(value.as_i64(), 42);

    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("installed-project").with_debug_info(false),
    );
    assert!(
        lowering
            .lower_module_with_entry(&module, Some("missing.main.main"))
            .is_err()
    );
    lowering
        .lower_module_with_entry(&module, Some(owner))
        .unwrap();
    lowering.module().verify().unwrap();
    // Execute the actual emitted process entry and the dependency body it
    // reaches. Neither fixture function imports the platform runtime.
    let mut ir = String::new();
    for name in ["verum_main", "first.main.answer"] {
        let function = lowering.module().get_function(name).unwrap();
        assert!(function.count_basic_blocks() > 0, "{name} must be emitted");
        ir.push_str(function.print_to_string().to_str().unwrap());
        ir.push('\n');
    }
    let buffer = MemoryBuffer::create_from_memory_range_copy(ir.as_bytes(), "project-entry");
    let native = context.create_module_from_ir(buffer).unwrap();
    Target::initialize_native(&Default::default()).unwrap();
    let engine = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        let main = engine
            .get_function::<unsafe extern "C" fn() -> i64>("verum_main")
            .unwrap();
        assert_eq!(main.call(), 42);
    }
}
