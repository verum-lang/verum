//! Actual List source methods use their runtime storage encoding, not T.size.
use std::cell::RefCell;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::{AnyValue, CallSiteValue},
};
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

thread_local! { static STORAGE: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new()); }
extern "C" fn allocate(size: u64) -> *mut u64 {
    STORAGE.with(|s| {
        let mut allocation = List::from_elem(0, size.div_ceil(8) as usize).into_boxed_slice();
        let ptr = allocation.as_mut_ptr();
        s.borrow_mut().push(allocation);
        ptr
    })
}
extern "C" fn release(_pointer: u64, _size: u64) {}
extern "C" fn abort(_code: u64) {
    std::process::abort();
}
const BOUNDARIES: &[(&str, &str)] = &[
    (
        "verum_checked_malloc",
        "declare ptr @verum_checked_malloc(i64)\n",
    ),
    ("verum_os_alloc", "declare ptr @verum_os_alloc(i64)\n"),
    ("verum_dealloc", "declare void @verum_dealloc(ptr, i64)\n"),
    (
        "verum_internal_exit_i64",
        "declare void @verum_internal_exit_i64(i64)\n",
    ),
];
fn is_list(ty: &verum_ast::Type) -> bool {
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(p) => p.as_ident().is_some_and(|i| i.name == "List"),
        verum_ast::ty::TypeKind::Generic { base, .. } => is_list(base),
        _ => false,
    }
}
fn with_source(
    source: &str,
    check: impl FnOnce(&str, &verum_llvm::execution_engine::ExecutionEngine),
) {
    let mut ast = Parser::new(source).parse_module().expect("caller grammar");
    let mut core = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module()
        .expect("core grammar");
    core.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Type(t) => t.name.name == "List",
        ItemKind::Function(f) => f.name.name == "list_max_len",
        ItemKind::Impl(i) if matches!(&i.kind, ImplKind::Inherent(t) if is_list(t)) => {
            i.items.retain(|m| matches!(&m.kind, ImplItemKind::Function(f) if ["new", "len", "capacity", "reserve", "shrink_to_fit", "resize_buffer", "free_buffer"].contains(&f.name.name.as_str())));
            true
        },
        _ => false,
    });
    ast.items.extend(core.items);
    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    codegen.register_stdlib_constants();
    codegen.register_stdlib_intrinsics();
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("declarations");
    codegen
        .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
        .expect("source bodies");
    let mut vbc = codegen.finalize_module().expect("source VBC");
    vbc.resolve_protocol_dispatch();
    for function in &mut vbc.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &vbc.functions {
        let body = &vbc.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        verum_vbc::mono::discover_call_instantiations(
            &vbc,
            &verum_vbc::bytecode::decode_instructions(body).unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .expect("source call discovery");
    }
    let vbc = verum_vbc::mono::monomorphize_minimal(vbc, &graph)
        .expect("source mono")
        .module;
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("list_storage").with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    let mut todo = List::new();
    for fd in &vbc.functions {
        todo.push(
            lower
                .module()
                .get_function(vbc.get_string(fd.name).unwrap())
                .unwrap(),
        );
    }
    let mut seen = Set::<Text>::new();
    let mut ir = Text::new();
    // Follow actual direct calls so the JIT includes the real allocation,
    // reallocation and deallocation code, only replacing OS allocation edges.
    while let Some(function) = todo.pop() {
        let name = Text::from(function.get_name().to_str().unwrap());
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some((_, decl)) = BOUNDARIES.iter().find(|(n, _)| *n == name.as_str()) {
            ir.push_str(decl);
            continue;
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
    for global in lower.module().get_globals() {
        if !global.get_name().to_str().unwrap().starts_with("llvm.") {
            ir.push_str(global.print_to_string().to_str().unwrap());
            ir.push('\n');
        }
    }
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "list_storage",
        ))
        .expect("source IR");
    executable.verify().expect("valid source IR");
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    for (name, addr) in [
        ("verum_checked_malloc", allocate as *const () as usize),
        ("verum_os_alloc", allocate as *const () as usize),
        ("verum_dealloc", release as *const () as usize),
        ("verum_internal_exit_i64", abort as *const () as usize),
    ] {
        if let Some(f) = executable.get_function(name) {
            engine.add_global_mapping(&f, addr);
        }
    }
    if std::env::var_os("VERUM_TEST_TRACE_LIST_IR").is_some() {
        eprintln!("{ir}");
    }
    check(&ir, &engine);
    STORAGE.with(|s| s.borrow_mut().clear());
}
#[test]
fn value_slot_constructor_uses_cbgr_storage() {
    with_source(
        "fn make() -> List<Int> { List<Int>.with_capacity(40) }",
        |_, jit| unsafe {
            let handle = jit
                .get_function::<unsafe extern "C" fn() -> u64>("make")
                .unwrap()
                .call();
            assert_ne!(handle, 0);
            assert_eq!(*(handle as *const u32), verum_vbc::types::TypeId::LIST.0);
            let data = *((handle as *const u64).add(5)) as *const u8;
            let size =
                *(data.sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize) as *const u32);
            assert_eq!(size, 40 * verum_common::layout::VALUE_SLOT_SIZE as u32);
        },
    );
}

#[test]
fn value_slot_push_and_read_agree_after_growth() {
    with_source(
        r#"
        fn make() -> List<Int> { List<Int>.with_capacity(1) }
        fn append(xs: &mut List<Int>, value: Int) { xs.push(value); }
        fn probe() -> Int {
            let mut xs: List<Int> = make();
            append(&mut xs, 0);
            append(&mut xs, 255);
            append(&mut xs, 42);
            xs[0] + xs[1] + xs[2]
        }
    "#,
        |_, jit| {
            let result = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(result, 297);
        },
    );
}

#[test]
fn int_source_reserve_shrink_regrow_layout_control() {
    with_source(
        r#"
        fn probe() -> Int {
            let mut values: List<Int> = List<Int>.new();
            values.reserve(3);
            values.push(7);
            values.shrink_to_fit();
            values.reserve(17);
            values[0]
        }
    "#,
        |_, jit| {
            let result = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(result, 7);
        },
    );
}

#[test]
fn storage_survives_receiver_argument_and_return() {
    with_source(
        r#"
        fn make() -> List<Int> { List<Int>.with_capacity(2) }
        fn grow(values: &mut List<Int>) { values.reserve(17); }
        fn probe() -> Int {
            let mut values: List<Int> = make();
            values.push(7);
            grow(&mut values);
            values.shrink_to_fit();
            values[0]
        }
    "#,
        |_, jit| {
            let actual = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 7);
        },
    );
}

#[test]
fn cloned_list_keeps_storage_and_allocator_provenance() {
    with_source(
        r#"
        fn probe() -> Int {
            let mut values: List<Int> = List<Int>.with_capacity(3);
            values.push(7);
            let mut copied: List<Int> = values.clone();
            copied.reserve(17);
            copied.shrink_to_fit();
            copied[0] + values[0]
        }
    "#,
        |_, jit| {
            let actual = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 14);
        },
    );
}

#[test]
fn record_value_uses_a_slot_not_its_inline_extent() {
    with_source(
        r#"
        type Cell is { x: Int, y: Int, z: Int };
        fn probe() -> Int {
            let mut values: List<Cell> = List<Cell>.new();
            values.reserve(2);
            values.push(Cell { x: 11, y: 37, z: 99 });
            values.shrink_to_fit();
            values.reserve(17);
            values[0].y
        }
    "#,
        |_, jit| {
            let actual = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 37);
        },
    );
}

#[test]
fn unknown_nominal_storage_is_refused() {
    const CHILD: &str = "VERUM_TEST_INVALID_LIST_STORAGE";
    if std::env::var_os(CHILD).is_some() {
        with_source(
            r#"
            type Foreign is { len: Int, cap: Int, ptr: Int };
            fn probe() -> Int {
                let value = Foreign { len: 0, cap: 0, ptr: 0 };
                @intrinsic("list_storage_stride", value)
            }
        "#,
            |_, jit| {
                unsafe {
                    jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                        .unwrap()
                        .call()
                };
            },
        );
        panic!("foreign nominal incorrectly supplied a storage layout");
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unknown_nominal_storage_is_refused",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .expect("negative child");
    // The test OS-exit boundary aborts. A Rust assertion failure exits101.
    assert!(!child.status.success());
    assert_ne!(child.status.code(), Some(101));
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            child.status.signal(),
            Some(6),
            "must refuse through the exit boundary, not fault on memory"
        );
    }
}

#[test]
fn zero_capacity_clone_can_reserve_after_empty_shrink() {
    with_source(
        r#"
        fn probe() -> Int {
            let values: List<Int> = List<Int>.new();
            let mut copied: List<Int> = values.clone();
            copied.shrink_to_fit();
            copied.reserve(2);
            copied.push(7);
            copied[0]
        }
    "#,
        |_, jit| {
            let actual = unsafe {
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 7);
        },
    );
}
