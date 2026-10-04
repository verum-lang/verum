//! T1567: Once uses the AtomicInt field declared by core/sync/once.vr.
use std::cell::RefCell;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::codegen::VbcCodegen;

thread_local! {
    static STORAGE: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    STORAGE.with(|storage| {
        let mut value = List::from_elem(0_u64, size.div_ceil(8) as usize).into_boxed_slice();
        let ptr = value.as_mut_ptr();
        storage.borrow_mut().push(value);
        ptr
    })
}
extern "C" fn unexpected_exit(_code: u64) {
    std::process::abort();
}
extern "C" fn unexpected_panic(_text: u64, _length: u64) {
    std::process::abort();
}
extern "C" fn unexpected_os_exit(_code: i32) {
    std::process::abort();
}
extern "C" fn unexpected_print(_text: *const u8) -> i32 {
    std::process::abort();
}

const CALLER: &str = r#"
    type MemoryOrdering is Relaxed | Acquire | Release | AcqRel | SeqCst;
    type AtomicInt is { value: Int };
    implement AtomicInt {
        fn new(value: Int) -> AtomicInt { AtomicInt { value } }
        fn load(&self, order: MemoryOrdering) -> Int { self.value }
        fn store(&self, value: Int, order: MemoryOrdering) { self.value = value; }
        fn compare_exchange(&self, expected: Int, desired: Int, success: MemoryOrdering, failure: MemoryOrdering) -> Result<Int, Int> {
            let old = self.value;
            if old == expected { self.value = desired; Result.Ok(old) } else { Result.Err(old) }
        }
    }
    fn spin_hint() {}
    fn compiler_fence(order: MemoryOrdering) {}
    type Cell is { value: Int };
    fn make_once() -> Once { Once.new() }
    fn once_run(once: &Once, cell: &mut Cell, add: Int) { once.call_once(|| { cell.value += add; }); }
    fn once_done(once: &Once) -> Bool { once.is_completed() }
    fn make_lock() -> OnceLock<Int> { OnceLock.new() }
    fn named_initializer() -> Int { 73 }
    fn lock_named(lock: &OnceLock<Int>) -> Int {
        let result = lock.get_or_init(named_initializer);
        *result
    }
    fn lock_value(lock: &OnceLock<Int>, cell: &mut Cell, value: Int) -> Int {
        let result = lock.get_or_init(|| { cell.value += 1; value });
        *result
    }
"#;

fn owner_name(owner: &verum_ast::Type) -> Option<&str> {
    match &owner.kind {
        verum_ast::ty::TypeKind::Path(path) => path.as_ident().map(|i| i.name.as_str()),
        verum_ast::ty::TypeKind::Generic { base, .. } => owner_name(base),
        _ => None,
    }
}

fn with_core_source(check: impl FnOnce(&Context, &str)) {
    let mut ast = Parser::new(CALLER).parse_module().expect("caller source");
    // Keep the actual Once/OnceLock declarations and relevant methods. Atomic
    // operations go through the same common LLVM lowering as the full stdlib.
    let mut once = Parser::new(include_str!("../../../core/sync/once.vr"))
        .parse_module()
        .expect("stdlib once syntax");
    once.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Const(c) => ["INCOMPLETE", "RUNNING", "COMPLETE", "POISONED"].contains(&c.name.name.as_str()),
        ItemKind::Type(t) => ["Once", "OnceGuard", "OnceLock"].contains(&t.name.name.as_str()),
        ItemKind::Impl(i) => {
            let ImplKind::Inherent(owner) = &i.kind else { return false; };
            let name = owner_name(owner);
            let wanted: &[&str] = if name == Some("Once") { &["new", "call_once", "call_once_slow", "is_completed"] }
                else if name == Some("OnceLock") { &["new", "get_or_init", "is_initialized"] } else { return false; };
            i.items.retain(|m| matches!(&m.kind, ImplItemKind::Function(f) if wanted.contains(&f.name.name.as_str())));
            true
        }
        _ => false,
    });
    ast.items.extend(once.items);
    let mut maybe = Parser::new(include_str!("../../../core/base/maybe.vr"))
        .parse_module()
        .expect("stdlib maybe syntax");
    maybe.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Impl(i) if matches!(&i.kind, ImplKind::Inherent(owner) if owner_name(owner) == Some("Maybe")) => {
            i.items.retain(|m| matches!(&m.kind, ImplItemKind::Function(f) if ["as_ref", "expect"].contains(&f.name.name.as_str())));
            true
        }
        _ => false,
    });
    ast.items.extend(maybe.items);
    // Canonical sums are declared in their source owners, as in bootstrap;
    // the test does not inject TypeIds or manufacture field descriptors.
    let maybe_decl = Parser::new("module core.base.maybe; public type Maybe<T> is None | Some(T);")
        .parse_module()
        .expect("Maybe declaration");
    let result_decl =
        Parser::new("module core.base.result; public type Result<T, E> is Ok(T) | Err(E);")
            .parse_module()
            .expect("Result declaration");
    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    codegen.register_stdlib_constants();
    codegen.register_stdlib_intrinsics();
    codegen
        .collect_unit_declarations(&[&maybe_decl, &result_decl, &ast])
        .expect("source declarations");
    codegen
        .compile_unit_items(
            &[&maybe_decl, &result_decl, &ast],
            verum_vbc::codegen::ItemFailurePolicy::Strict,
        )
        .expect("source bodies");
    let vbc = codegen.finalize_module().expect("source VBC");
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("once_storage").with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    let mut ir = Text::from(
        "declare ptr @verum_checked_malloc(i64)\ndeclare ptr @verum_os_alloc(i64)\ndeclare void @verum_internal_exit_i64(i64)\ndeclare void @verum_panic(ptr, i64)\ndeclare i32 @verum_internal_puts(ptr)\ndeclare void @verum_os_exit(i32)\n",
    );
    for name in [
        "verum_generic_eq",
        "verum_is_text_object",
        "verum_text_get_ptr",
        "verum_internal_strcmp",
        "verum_internal_memset",
    ] {
        let function = lower.module().get_function(name).expect(name);
        ir.push_str(function.print_to_string().to_str().expect("runtime IR"));
    }
    for function in lower.module().get_functions() {
        if function.count_basic_blocks() == 0
            && ![
                "verum_checked_malloc",
                "verum_os_alloc",
                "verum_internal_exit_i64",
                "verum_panic",
                "verum_internal_puts",
                "verum_os_exit",
            ]
            .contains(&function.get_name().to_str().expect("name"))
        {
            ir.push_str(
                function
                    .print_to_string()
                    .to_str()
                    .expect("intrinsic declaration"),
            );
        }
    }
    for global in lower.module().get_globals() {
        ir.push_str(global.print_to_string().to_str().expect("global IR"));
        ir.push('\n');
    }
    for descriptor in &vbc.functions {
        let name = vbc.get_string(descriptor.name).expect("function name");
        let function = lower.module().get_function(name).expect(name);
        assert!(function.verify(true), "{name}");
        ir.push_str(function.print_to_string().to_str().expect("source IR"));
        if let Some(adapter) = lower.module().get_function(&format!("{name}$trampoline$")) {
            ir.push_str(adapter.print_to_string().to_str().expect("adapter IR"));
        }
    }
    check(&context, &ir);
}

fn with_native(
    ir: &str,
    context: &Context,
    check: impl FnOnce(&verum_llvm::execution_engine::ExecutionEngine),
) {
    // Refuse the known bad lowering before executing it. The public call must
    // use the declaration's body, which loads the actual AtomicInt field.
    let caller = ir
        .split("@once_run(")
        .nth(1)
        .expect("once caller")
        .split("\n}")
        .next()
        .unwrap();
    assert!(
        caller.contains("@Once.call_once("),
        "must call declared Once body: {caller}"
    );
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let module = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "once_storage",
        ))
        .expect("source IR");
    module.verify().expect("valid source IR");
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    engine.add_global_mapping(
        &module.get_function("verum_checked_malloc").unwrap(),
        allocate as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("verum_os_alloc").unwrap(),
        allocate as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("verum_internal_exit_i64").unwrap(),
        unexpected_exit as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("verum_panic").unwrap(),
        unexpected_panic as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("verum_internal_puts").unwrap(),
        unexpected_print as *const () as usize,
    );
    engine.add_global_mapping(
        &module.get_function("verum_os_exit").unwrap(),
        unexpected_os_exit as *const () as usize,
    );
    check(&engine);
    STORAGE.with(|storage| storage.borrow_mut().clear());
}

#[test]
fn source_once_constructor_and_captured_callback_share_declared_atomic_storage() {
    with_core_source(|context, ir| {
        with_native(ir, context, |engine| {
            let mut cell = [0_u64; 4];
            // SAFETY: these are the exact source slot signatures; all source
            // allocations remain live until this callback returns.
            unsafe {
                let make = engine
                    .get_function::<unsafe extern "C" fn() -> u64>("make_once")
                    .unwrap();
                let run = engine
                    .get_function::<unsafe extern "C" fn(u64, u64, u64)>("once_run")
                    .unwrap();
                let done = engine
                    .get_function::<unsafe extern "C" fn(u64) -> bool>("once_done")
                    .unwrap();
                let once = make.call();
                assert!(!done.call(once));
                run.call(once, cell.as_mut_ptr() as u64, 7);
                assert_eq!(cell[3], 7);
                assert!(done.call(once));
                run.call(once, cell.as_mut_ptr() as u64, 99);
                assert_eq!(cell[3], 7);
            }
        })
    });
}

#[test]
fn source_once_lock_runs_initializer_once_and_returns_the_stored_value() {
    with_core_source(|context, ir| {
        with_native(ir, context, |engine| {
            let mut cell = [0_u64; 4];
            // SAFETY: exact source signatures and live allocator-backed records.
            unsafe {
                let make = engine
                    .get_function::<unsafe extern "C" fn() -> u64>("make_lock")
                    .unwrap();
                let value = engine
                    .get_function::<unsafe extern "C" fn(u64, u64, u64) -> u64>("lock_value")
                    .unwrap();
                let lock = make.call();
                assert_eq!(value.call(lock, cell.as_mut_ptr() as u64, 37), 37);
                assert_eq!(value.call(lock, cell.as_mut_ptr() as u64, 99), 37);
                assert_eq!(cell[3], 1);
            }
        })
    });
}

#[test]
fn source_once_lock_accepts_a_named_initializer_and_skips_the_next_closure() {
    with_core_source(|context, ir| {
        with_native(ir, context, |engine| {
            let mut cell = [0_u64; 4];
            // SAFETY: exact source signatures; the test owns all record storage.
            unsafe {
                let make = engine
                    .get_function::<unsafe extern "C" fn() -> u64>("make_lock")
                    .unwrap();
                let named = engine
                    .get_function::<unsafe extern "C" fn(u64) -> u64>("lock_named")
                    .unwrap();
                let captured = engine
                    .get_function::<unsafe extern "C" fn(u64, u64, u64) -> u64>("lock_value")
                    .unwrap();
                let lock = make.call();
                assert_eq!(named.call(lock), 73);
                assert_eq!(captured.call(lock, cell.as_mut_ptr() as u64, 99), 73);
                assert_eq!(
                    cell[3], 0,
                    "completed Once must not evaluate the second initializer"
                );
            }
        })
    });
}
