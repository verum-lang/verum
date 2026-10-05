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
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .expect("memory grammar");
    memory.items.retain(|item| matches!(&item.kind, ItemKind::Function(f) if ["list_storage_read", "list_storage_write", "list_storage_move"].contains(&f.name.name.as_str())));
    ast.items.extend(memory.items);
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

#[test]
fn nominal_byte_constructor_preserves_record_slots_across_capacity_changes() {
    with_source(
        r#"
        type Byte is { x: Int, y: Int, z: Int };
        fn make() -> List<Byte> { List<Byte>.with_capacity(2) }
        fn probe() -> Int {
            let mut values: List<Byte> = make();
            values.push(Byte { x: 11, y: 37, z: 99 });
            values.shrink_to_fit();
            values.reserve(17);
            values[0].y
        }
        "#,
        |ir, jit| {
            assert!(
                !ir.contains("verum_alloc_byte_list_packed"),
                "nominal Byte selected packed allocator"
            );
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
fn owned_storage_read_write_and_overlap_preserve_value_slots() {
    with_source(
        r#"
        type Triple is {x:Int,y:Int,z:Int};
        fn probe()->Int {
            let values:List<Triple> =List<Triple>.with_capacity(3);
            @intrinsic("list_storage_write", values, 0, Triple{x:11,y:37,z:99});
            @intrinsic("list_storage_write", values, 1, Triple{x:22,y:41,z:88});
            @intrinsic("list_storage_move", values, 0, 1, 2);
            let first:Triple = @intrinsic("list_storage_read", values, 1);
            let second:Triple = @intrinsic("list_storage_read", values, 2);
            first.y + second.y
        }
        "#,
        |_, jit| unsafe {
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call(),
                78
            );
        },
    );
}

#[test]
fn packed_storage_access_uses_existing_exact_carrier_encoding() {
    // This pins the consumer ABI using a real emitted CBGR allocation. It
    // intentionally does not claim a completed native List<Byte> constructor.
    with_source(
        r#"
        fn make()->List<Int> { List<Int>.new() }
        fn make_backing()->Int { @intrinsic("cbgr_allocate", 4) }
        fn read(values:List<Byte>, index:Int)->Int { @intrinsic("list_storage_read", values, index) }
        fn storage_write_probe(values:List<Byte>, index:Int, value:Int) { @intrinsic("list_storage_write", values, index, value) }
        fn move_range(values:List<Byte>, source:Int, target:Int, count:Int) { @intrinsic("list_storage_move", values, source, target, count) }
        "#,
        |_, jit| unsafe {
            let handle = jit
                .get_function::<unsafe extern "C" fn() -> u64>("make")
                .unwrap()
                .call();
            let backing = jit
                .get_function::<unsafe extern "C" fn() -> *mut u8>("make_backing")
                .unwrap()
                .call();
            assert_eq!(
                *(backing.sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize) as *const u32),
                4
            );
            *(handle as *mut u32) = verum_vbc::types::TypeId::BYTE_LIST.0;
            *((handle as *mut u64).add(4)) = 4;
            *((handle as *mut u64).add(5)) = backing as u64;
            let write = jit
                .get_function::<unsafe extern "C" fn(u64, i64, i64)>("storage_write_probe")
                .unwrap();
            let read = jit
                .get_function::<unsafe extern "C" fn(u64, i64) -> i64>("read")
                .unwrap();
            let movement = jit
                .get_function::<unsafe extern "C" fn(u64, i64, i64, i64)>("move_range")
                .unwrap();
            for (i, value) in [0, 255, 42, 17].into_iter().enumerate() {
                write.call(handle, i as i64, value);
            }
            assert_eq!(std::slice::from_raw_parts(backing, 4), [0, 255, 42, 17]);
            movement.call(handle, 0, 1, 3);
            assert_eq!(std::slice::from_raw_parts(backing, 4), [0, 0, 255, 42]);
            movement.call(handle, 1, 0, 3);
            assert_eq!(std::slice::from_raw_parts(backing, 4), [0, 255, 42, 42]);
            assert_eq!(read.call(handle, 1) + read.call(handle, 2), 297);
        },
    );
}

#[test]
fn owned_storage_zero_move_does_not_touch_empty_backing() {
    with_source(
        r#"fn probe()->Int {let values:List<Int> =List<Int>.new(); @intrinsic("list_storage_move", values, 0, 0, 0); 7}"#,
        |_, jit| unsafe {
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call(),
                7
            );
        },
    );
}

#[test]
fn owned_storage_invalid_range_and_extent_refuse_before_writing() {
    const CHILD: &str = "VERUM_TEST_LIST_ACCESS_REFUSAL";
    if let Some(case) = std::env::var_os(CHILD) {
        with_source(
            r#"
            fn make()->List<Int> {List<Int>.with_capacity(2)}
            fn storage_write_probe(values:List<Int>, index:Int)->Int {@intrinsic("list_storage_write",values,index,37); 0}
            fn movement(values:List<Int>, source:Int,target:Int,count:Int)->Int {@intrinsic("list_storage_move",values,source,target,count); 0}
            "#,
            |_, jit| unsafe {
                let handle = jit
                    .get_function::<unsafe extern "C" fn() -> u64>("make")
                    .unwrap()
                    .call();
                let write = jit
                    .get_function::<unsafe extern "C" fn(u64, i64)>("storage_write_probe")
                    .unwrap();
                match case.to_str().unwrap() {
                    "index" => write.call(handle, 2),
                    "negative" => write.call(handle, -1),
                    "extent" => {
                        *((handle as *mut u64).add(4)) = 3;
                        write.call(handle, 2);
                    }
                    "foreign" => {
                        *(handle as *mut u32) = 99999;
                        write.call(handle, 0);
                    }
                    "shape" => {
                        *((handle as *mut u32).add(3)) = 24;
                        write.call(handle, 0);
                    }
                    "zero-null" | "zero-extent" => {
                        if case == "zero-null" {
                            *((handle as *mut u64).add(5)) = 0;
                        } else {
                            *((handle as *mut u64).add(4)) = 3;
                        }
                        jit.get_function::<unsafe extern "C" fn(u64, i64, i64, i64)>("movement")
                            .unwrap()
                            .call(handle, 0, 0, 0);
                    }
                    "move" => jit
                        .get_function::<unsafe extern "C" fn(u64, i64, i64, i64)>("movement")
                        .unwrap()
                        .call(handle, 0, 1, 2),
                    _ => panic!("unknown refusal case"),
                }
            },
        );
        panic!("invalid storage access returned");
    }
    for case in [
        "index",
        "negative",
        "extent",
        "foreign",
        "shape",
        "move",
        "zero-null",
        "zero-extent",
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "owned_storage_invalid_range_and_extent_refuse_before_writing",
                "--nocapture",
            ])
            .env(CHILD, case)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{case}");
        assert_ne!(
            output.status.code(),
            Some(101),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                output.status.signal(),
                Some(6),
                "{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn owned_storage_float_bits_survive_argument_return_and_move() {
    for ty in ["Float", "Float32"] {
        let source = r#"
        fn store(values:List<Float>, value:Float)->Int {
            let done = 0;
            @intrinsic("list_storage_write", values, 0, value); done
        }
        fn fetch(values:List<Float>)->Float { let value = @intrinsic("list_storage_read", values, 1); value }
        fn probe()->Float {
            let values:List<Float> =List<Float>.with_capacity(2);
            store(values,1.25);
            @intrinsic("list_storage_move", values, 0, 1, 1);
            fetch(values)
        }
    "#.replace("Float", ty).replace("fn probe()->Float32", "fn probe()->Float");
        with_source(&source, |_, jit| unsafe {
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> f64>("probe")
                    .unwrap()
                    .call(),
                1.25
            );
        });
    }
}

#[test]
fn owned_storage_wide_unboxed_value_is_refused() {
    const CHILD: &str = "VERUM_TEST_LIST_WIDE_REFUSAL";
    if std::env::var_os(CHILD).is_some() {
        with_source(
            r#"
            fn probe()->Int {
                let value = 18446744073709551616i128;
                let values:List<Int128> =List<Int128>.with_capacity(1);
                @intrinsic("list_storage_write",values,0,value); 0
            }
        "#,
            |_, _| panic!("unsupported wide storage reached JIT"),
        );
        return;
    }
    // Strict lowering exposes the precise unsupported-carrier error instead
    // of the existing module-wide lenient function-skip policy.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_storage_wide_unboxed_value_is_refused",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("VERUM_STRICT_CODEGEN", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("native i128 is not boxed"),
        "{diagnostic}"
    );
}

#[test]
fn public_storage_functions_accept_list_borrows() {
    with_source(
        r#"type Triple is {x:Int,y:Int,z:Int}; fn probe()->Int {
        let mut values:List<Triple> =List<Triple>.with_capacity(2);
        unsafe {
            list_storage_write(&mut values,0,Triple{x:11,y:37,z:99});
            list_storage_move(&mut values,0,1,1);
            let result:Triple = list_storage_read(&values,1);
            result.y
        }
    }"#,
        |_, jit| unsafe {
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call(),
                37
            );
        },
    );
}
