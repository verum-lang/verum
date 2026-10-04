//! T1555: native callbacks consume the carrier emitted by source NewClosure.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::OptimizationLevel;
use verum_llvm::context::Context;
use verum_llvm::memory_buffer::MemoryBuffer;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::values::AnyValue;
use verum_vbc::codegen::VbcCodegen;

thread_local! {
    // Only the allocator is replaced in the JIT harness. Source closure
    // construction, capture loads and both callback consumers are real IR.
    static ALLOCATIONS: RefCell<Vec<Box<[u64]>>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn allocate(size: u64) -> *mut u64 {
    ALLOCATIONS.with(|allocations| {
        let mut storage = vec![0_u64; size.div_ceil(8) as usize].into_boxed_slice();
        let pointer = storage.as_mut_ptr();
        allocations.borrow_mut().push(storage);
        pointer
    })
}

extern "C" fn unexpected_exit(_code: u64) {
    std::process::abort();
}

const SOURCE: &str = r#"
    type Cell is { value: Int };
    type Once is { state: Int };
    fn make(cell: &mut Cell, increment: Int) -> fn() {
        || { cell.value += increment; }
    }
    fn invoke(f: fn()) { f(); }
    fn invoke_once(once: &Once, f: fn()) { once.call_once(f); }
    fn initialize(once: &Once, cell: &mut Cell, f: fn() -> Int) {
        once.call_once(|| { cell.value += f(); });
    }
    fn make_empty() -> fn() -> Int { || 73 }
    fn invoke_empty(f: fn() -> Int) -> Int { f() }
    fn make_with_argument(bias: Int) -> fn(Int) -> Int { |value| bias + value }
    fn invoke_with_argument(f: fn(Int) -> Int, value: Int) -> Int { f(value) }
"#;

fn with_source_ir(check: impl FnOnce(&Context, &str)) {
    let ast = Parser::new(SOURCE).parse_module().expect("source parses");
    let vbc = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("callback_abi").with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    let mut definitions = String::from(
        "declare ptr @verum_checked_malloc(i64)\ndeclare void @verum_internal_exit_i64(i64)\n",
    );
    for descriptor in &vbc.functions {
        let name = vbc.get_string(descriptor.name).expect("function name");
        let function = lower.module().get_function(name).expect(name);
        assert!(function.verify(true), "{name}");
        definitions.push_str(&function.print_to_string().to_string());
        definitions.push('\n');
    }
    check(&context, &definitions);
}

fn assert_once_abi(ir: &str) {
    let once = ir.split("@invoke_once(").nth(1).expect("once function");
    let once = once.split("\n}").next().unwrap();
    assert!(
        once.contains("call i64 %"),
        "callback must use common i64 return ABI: {once}"
    );
    assert!(
        once.contains("load ptr, ptr"),
        "must load function/env pointers: {once}"
    );
    assert!(
        !once.contains("once_fnptr_ptr"),
        "must not load an object-header offset: {once}"
    );
}

#[test]
fn source_once_callback_uses_the_same_carrier_as_call_closure() {
    with_source_ir(|_, ir| {
        assert!(ir.contains("@verum_checked_malloc(i64 16)"), "{ir}");
        assert!(ir.contains("store ptr null"), "captureless env: {ir}");
        assert_once_abi(ir);
    });
}

#[test]
fn captured_source_callback_runs_once_and_keeps_its_environment() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    with_source_ir(|context, ir| {
        // Fail safely on a bad emitter instead of executing an invalid pointer.
        assert_once_abi(ir);
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "callbacks",
            ))
            .expect("source IR");
        module.verify().expect("valid IR");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        engine.add_global_mapping(
            &module.get_function("verum_checked_malloc").unwrap(),
            allocate as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("verum_internal_exit_i64").unwrap(),
            unexpected_exit as *const () as usize,
        );
        let mut cell = [0_u64; 4];
        let mut once = [0_u64; 4];
        // SAFETY: source functions use the uniform native slot ABI. Record
        // storage includes the header; produced closures live in ALLOCATIONS
        // until after all calls, including their separately allocated envs.
        unsafe {
            let make = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("make")
                .unwrap();
            let invoke = engine
                .get_function::<unsafe extern "C" fn(u64)>("invoke")
                .unwrap();
            let invoke_once = engine
                .get_function::<unsafe extern "C" fn(u64, u64)>("invoke_once")
                .unwrap();
            let callback = make.call(cell.as_mut_ptr() as u64, 7);
            invoke.call(callback);
            assert_eq!(cell[3], 7);
            invoke_once.call(once.as_mut_ptr() as u64, callback);
            assert_eq!((cell[3], once[3]), (14, 2));
            invoke_once.call(once.as_mut_ptr() as u64, callback);
            assert_eq!((cell[3], once[3]), (14, 2));
            let other = make.call(cell.as_mut_ptr() as u64, 91);
            invoke_once.call(once.as_mut_ptr() as u64, other);
            assert_eq!(
                cell[3], 14,
                "completed state must not invoke another callback"
            );
            // OnceLock's closure shape: the callback captures a function
            // value and a destination, then invokes that captured function.
            let initialize = engine
                .get_function::<unsafe extern "C" fn(u64, u64, u64)>("initialize")
                .unwrap();
            let empty = engine
                .get_function::<unsafe extern "C" fn() -> u64>("make_empty")
                .unwrap()
                .call();
            once[3] = 0;
            initialize.call(once.as_mut_ptr() as u64, cell.as_mut_ptr() as u64, empty);
            assert_eq!((cell[3], once[3]), (87, 2));
            initialize.call(once.as_mut_ptr() as u64, cell.as_mut_ptr() as u64, empty);
            assert_eq!((cell[3], once[3]), (87, 2));
        }
    });
    ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
}

#[test]
fn ordinary_closures_keep_captureless_and_user_argument_abi() {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    with_source_ir(|context, ir| {
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                ir.as_bytes(),
                "callbacks",
            ))
            .expect("source IR");
        module.verify().expect("valid IR");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        engine.add_global_mapping(
            &module.get_function("verum_checked_malloc").unwrap(),
            allocate as *const () as usize,
        );
        engine.add_global_mapping(
            &module.get_function("verum_internal_exit_i64").unwrap(),
            unexpected_exit as *const () as usize,
        );
        // SAFETY: exact source signatures, all allocations remain alive.
        unsafe {
            let make = engine
                .get_function::<unsafe extern "C" fn() -> u64>("make_empty")
                .unwrap();
            let invoke = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("invoke_empty")
                .unwrap();
            assert_eq!(invoke.call(make.call()), 73);
            let make = engine
                .get_function::<unsafe extern "C" fn(u64) -> u64>("make_with_argument")
                .unwrap();
            let invoke = engine
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("invoke_with_argument")
                .unwrap();
            assert_eq!(invoke.call(make.call(31), 11), 42);
        }
    });
    ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
}
