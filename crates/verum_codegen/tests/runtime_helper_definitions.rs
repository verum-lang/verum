//! Compiler-owned runtime definitions must complete earlier declarations.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, runtime::RuntimeLowering};
use verum_common::{Heap, List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    AddressSpace, OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::{AnyValue, CallSiteValue},
};
use verum_vbc::codegen::VbcCodegen;

thread_local! { static STORAGE: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new()); }
extern "C" fn os_allocate(size: u64) -> *mut u64 {
    STORAGE.with(|storage| {
        let mut allocation = List::from_elem(0_u64, size.div_ceil(8) as usize).into_boxed_slice();
        let result = allocation.as_mut_ptr();
        storage.borrow_mut().push(allocation);
        result
    })
}
extern "C" fn os_exit(code: i32) {
    std::process::exit(code);
}

fn with_source(check: impl FnOnce(&Context, &verum_llvm::module::Module)) {
    let ast = Parser::new(
        r#"
        type Cell is { value: Int };
        static mut ROOT: Cell = Cell { value: 73 };
        fn probe() -> Int { let cell = Cell { value: 37 }; cell.value }
        fn read_root() -> Int { ROOT.value }
        fn main() { probe(); read_root(); }
    "#,
    )
    .parse_module()
    .expect("source grammar");
    let mut codegen = VbcCodegen::new();
    let vbc = codegen.compile_module(&ast).expect("source VBC");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("helper_definitions").with_debug_info(false),
    );
    lower
        .lower_module(&vbc)
        .expect("complete lowering pipeline");
    check(&context, lower.module());
}

#[test]
fn source_pipeline_completes_runtime_bodies_before_static_initializers_run() {
    with_source(|context, module| {
        let malloc = module
            .get_function("verum_checked_malloc")
            .expect("runtime allocator");
        let exit = module
            .get_function("verum_internal_exit_i64")
            .expect("runtime exit");
        let malloc_ir = malloc.print_to_string();
        assert!(
            malloc_ir.to_str().unwrap().contains("@verum_os_alloc"),
            "{malloc_ir}"
        );
        let exit_ir = exit.print_to_string();
        assert!(
            exit_ir.to_str().unwrap().contains("@verum_os_exit"),
            "{exit_ir}"
        );
        let init = module
            .get_function("__verum_static_init")
            .expect("source static initializer");
        assert!(
            init.print_to_string()
                .to_str()
                .unwrap()
                .contains("__tls_init_ROOT")
        );
        let mut todo: List<_> = [
            init,
            module.get_function("probe").unwrap(),
            module.get_function("read_root").unwrap(),
        ]
        .into_iter()
        .collect();
        let mut seen = Set::<Text>::new();
        let mut ir = Text::new();
        while let Some(function) = todo.pop() {
            let name = Text::from(function.get_name().to_str().unwrap());
            if !seen.insert(name.clone()) {
                continue;
            }
            match name.as_str() {
                // The compiler-owned allocation and exit wrappers stay intact.
                // Only the OS boundary is replaced by the harness.
                "verum_os_alloc" => {
                    ir.push_str("declare ptr @verum_os_alloc(i64)\n");
                    continue;
                }
                "verum_os_exit" => {
                    ir.push_str("declare void @verum_os_exit(i32)\n");
                    continue;
                }
                _ => {}
            }
            ir.push_str(function.print_to_string().to_str().unwrap());
            ir.push('\n');
            for block in function.get_basic_blocks() {
                for instruction in block.get_instructions() {
                    if let Ok(call) = CallSiteValue::try_from(instruction) {
                        if let Some(callee) = call.get_called_fn_value() {
                            todo.push(callee);
                        }
                    }
                }
            }
        }
        for global in module.get_globals() {
            if !global.get_name().to_str().unwrap().starts_with("llvm.") {
                ir.push_str(global.print_to_string().to_str().unwrap());
                ir.push('\n');
            }
        }
        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let executable = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "helpers",
            ))
            .expect("retained source IR");
        executable.verify().expect("valid source IR");
        let engine = executable
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(
            &executable.get_function("verum_os_alloc").unwrap(),
            os_allocate as *const () as usize,
        );
        engine.add_global_mapping(
            &executable.get_function("verum_os_exit").unwrap(),
            os_exit as *const () as usize,
        );
        // SAFETY: names and signatures are the source declarations above. The
        // checked runtime allocator body has been retained and verified.
        unsafe {
            engine
                .get_function::<unsafe extern "C" fn()>("__verum_static_init")
                .unwrap()
                .call();
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("read_root")
                    .unwrap()
                    .call(),
                73
            );
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call(),
                37
            );
        }
    });
}

fn emit_allocation<'ctx>(
    context: &'ctx Context,
    module: &verum_llvm::module::Module<'ctx>,
    name: &str,
) -> verum_codegen::llvm::error::Result<()> {
    let function = module.add_function(
        name,
        context
            .ptr_type(AddressSpace::default())
            .fn_type(&[], false),
        None,
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let pointer = RuntimeLowering::new(context).emit_checked_malloc(
        &builder,
        module,
        context.i64_type().const_int(32, false),
        "allocation",
    )?;
    builder.build_return(Some(&pointer)).unwrap();
    Ok(())
}

#[test]
fn compatible_forward_declarations_keep_identity_and_complete_once() {
    for predeclare in [false, true] {
        let context = Context::create();
        let module = context.create_module("declaration_order");
        let ptr = context.ptr_type(AddressSpace::default());
        if predeclare {
            module.add_function(
                "verum_checked_malloc",
                ptr.fn_type(&[context.i64_type().into()], false),
                None,
            );
            module.add_function(
                "verum_internal_exit_i64",
                context
                    .void_type()
                    .fn_type(&[context.i64_type().into()], false),
                None,
            );
        }
        emit_allocation(&context, &module, "first").expect("first use");
        let malloc = module.get_function("verum_checked_malloc").unwrap();
        let exit = module.get_function("verum_internal_exit_i64").unwrap();
        assert!(
            malloc.count_basic_blocks() > 0,
            "allocator declaration left empty"
        );
        assert!(exit.count_basic_blocks() > 0, "exit declaration left empty");
        assert!(
            exit.get_enum_attribute(
                verum_llvm::attributes::AttributeLoc::Function,
                verum_llvm::attributes::Attribute::get_named_enum_kind_id("noreturn"),
            )
            .is_some(),
            "completed exit must preserve LLVM no-return semantics"
        );
        let before_malloc = malloc.print_to_string();
        let before_exit = exit.print_to_string();
        emit_allocation(&context, &module, "second").expect("repeat use");
        assert_eq!(module.get_function("verum_checked_malloc"), Some(malloc));
        assert_eq!(module.get_function("verum_internal_exit_i64"), Some(exit));
        assert_eq!(
            before_malloc.to_str().unwrap(),
            malloc.print_to_string().to_str().unwrap()
        );
        assert_eq!(
            before_exit.to_str().unwrap(),
            exit.print_to_string().to_str().unwrap()
        );
        assert!(module.get_function("verum_checked_malloc.1").is_none());
        module.verify().expect("valid completed declarations");
    }
}

#[test]
fn incompatible_runtime_forward_signatures_fail_closed() {
    for name in ["verum_checked_malloc", "verum_internal_exit_i64"] {
        let context = Context::create();
        let module = context.create_module("wrong_signature");
        module.add_function(name, context.i64_type().fn_type(&[], false), None);
        assert!(
            emit_allocation(&context, &module, "caller").is_err(),
            "{name} mismatch must be rejected before emitting a call"
        );
    }
}

#[test]
fn completed_exit_helper_terminates_instead_of_returning() {
    const CHILD: &str = "VERUM_TEST_T1603_EXIT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let context = Context::create();
        let module = context.create_module("exit_completion");
        module.add_function(
            "verum_internal_exit_i64",
            context
                .void_type()
                .fn_type(&[context.i64_type().into()], false),
            None,
        );
        emit_allocation(&context, &module, "unused_allocation").expect("canonical helpers");
        let exit = module.get_function("verum_internal_exit_i64").unwrap();
        assert!(
            exit.count_basic_blocks() > 0,
            "exit declaration must be completed"
        );
        module.verify().expect("valid completed exit");
        Target::initialize_native(&InitializationConfig::default()).unwrap();
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        engine.add_global_mapping(
            &module.get_function("verum_os_alloc").unwrap(),
            os_allocate as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("verum_os_exit").unwrap(),
            os_exit as *const () as usize,
        );
        // SAFETY: the tested canonical wrapper is void(i64). Its only OS edge
        // is the harness's terminating function; it must never return here.
        unsafe {
            engine
                .get_function::<unsafe extern "C" fn(i64)>("verum_internal_exit_i64")
                .unwrap()
                .call(77);
        }
        panic!("compiler-owned exit wrapper returned");
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "completed_exit_helper_terminates_instead_of_returning",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .expect("isolated exit control");
    assert_eq!(output.status.code(), Some(77), "child output: {output:?}");
}
