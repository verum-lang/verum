//! Actual List source methods use their runtime storage encoding, not T.size.
use std::cell::{Cell, RefCell};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Set, Text};
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::{AnyValue, CallSiteValue},
};

thread_local! {
    static STORAGE: RefCell<List<(u64, Heap<[u64]>)>> = RefCell::new(List::new());
    static FAIL_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    if FAIL_ALLOCATION.replace(false) {
        return std::ptr::null_mut();
    }
    STORAGE.with(|s| {
        let mut allocation = List::from_elem(0, size.div_ceil(8) as usize).into_boxed_slice();
        let ptr = allocation.as_mut_ptr();
        s.borrow_mut().push((size, allocation));
        ptr
    })
}
extern "C" fn release(pointer: *mut u64, size: u64) {
    STORAGE.with(|storage| {
        let mut storage = storage.borrow_mut();
        let index = storage.iter().position(|(_, entry)| entry.as_ptr() == pointer)
            .expect("native release must belong to this test allocator");
        let (allocated, _) = storage.swap_remove(index);
        assert_eq!(size, allocated, "released allocation extent");
    });
}
extern "C" fn abort(_code: i32) {
    std::process::abort();
}
const BOUNDARIES: &[(&str, &str)] = &[
    ("verum_os_alloc", "declare ptr @verum_os_alloc(i64)\n"),
    ("verum_os_free", "declare void @verum_os_free(ptr, i64)\n"),
    ("verum_os_exit", "declare void @verum_os_exit(i32)\n"),
];
#[path = "../../verum_vbc/tests/fixtures/list_storage.rs"]
mod list_storage;

fn with_source(
    source: &str,
    check: impl FnOnce(&verum_llvm::module::Module, &verum_llvm::execution_engine::ExecutionEngine),
) {
    with_methods(source, true, check);
}

fn with_methods(
    source: &str,
    rename_methods: bool,
    check: impl FnOnce(&verum_llvm::module::Module, &verum_llvm::execution_engine::ExecutionEngine),
) {
    let vbc = list_storage::compile(source, rename_methods);
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("list_storage").with_debug_info(false),
    );
    lower.lower_module(&vbc).expect("source LLVM");
    // Pin the legacy ListPop opcode helper on the same owners produced by
    // source constructors. Source `List.pop` returns Maybe and has a separate
    // source-body test below; the opcode itself returns a raw slot or Unit.
    let pointer = context.ptr_type(verum_llvm::AddressSpace::default());
    let pop = lower.module().add_function(
        "test_storage_pop",
        context.i64_type().fn_type(&[pointer.into()], false),
        None,
    );
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(pop, "entry"));
    let result = verum_codegen::llvm::runtime::RuntimeLowering::new(&context)
        .lower_list_pop(
            &builder,
            lower.module(),
            pop.get_first_param().unwrap().into_pointer_value(),
        )
        .expect("ListPop helper");
    builder.build_return(Some(&result)).unwrap();
    let mut todo = List::new();
    todo.push(pop);
    todo.extend(vbc.functions.iter().filter_map(|function| lower.get_function_by_id(function.id.0)));
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
        ("verum_os_alloc", allocate as *const () as usize),
        ("verum_os_free", release as *const () as usize),
        ("verum_os_exit", abort as *const () as usize),
    ] {
        if let Some(f) = executable.get_function(name) {
            engine.add_global_mapping(&f, addr);
        }
    }
    if std::env::var_os("VERUM_TEST_TRACE_LIST_IR").is_some() {
        eprintln!("{ir}");
    }
    check(&executable, &engine);
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
        |module, jit| {
            assert!(
                module.get_function("verum_alloc_byte_list_packed").is_none(),
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
    with_source(
        r#"
        fn make()->List<Int> { List<Int>.new() }
        fn storage_read_probe(values:&List<Byte>, index:Int)->Int { @intrinsic("list_storage_read", values, index) }
        fn storage_write_probe(values:&mut List<Byte>, index:Int, value:Int) { @intrinsic("list_storage_write", values, index, value) }
        fn move_range(values:&mut List<Byte>, source:Int, target:Int, count:Int) { @intrinsic("list_storage_move", values, source, target, count) }
        "#,
        |_, jit| unsafe {
            let encoding = verum_vbc::types::TypeId::BYTE_LIST.0;
            let handle = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>("verum_list_allocate_storage")
                .unwrap()
                .call(4, encoding);
            assert_eq!(*(handle as *const u32), encoding);
            assert_eq!(*((handle as *const u64).add(4)), 4);
            let backing = *((handle as *const u64).add(5)) as *const u8;
            assert_eq!(
                *(backing.sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize) as *const u32),
                4
            );
            let write = jit
                .get_function::<unsafe extern "C" fn(u64, i64, i64)>("storage_write_probe")
                .unwrap();
            let read = jit
                .get_function::<unsafe extern "C" fn(u64, i64) -> i64>("storage_read_probe")
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
fn indexing_reads_and_writes_the_owners_storage_width() {
    with_source(
        r#"
        fn replace_middle(values:&mut List<Byte>)->Int {
            values.checked_push(7); values.checked_push(42); values.checked_push(91);
            let previous = values[1];
            values[1] = 255 as Byte;
            previous as Int
        }
        "#,
        |module, jit| unsafe {
            let replace = module.get_function("replace_middle").expect("source replacement");
            let body = replace.print_to_string();
            let body = body.to_str().unwrap();
            assert!(body.contains("@verum_list_storage_read("));
            assert!(body.contains("@verum_list_storage_write("));
            let allocate = jit.get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                "verum_list_allocate_storage").unwrap();
            let replace = jit.get_function::<unsafe extern "C" fn(u64) -> i64>("replace_middle").unwrap();
            for (encoding, width) in [(verum_vbc::TypeId::LIST, 8), (verum_vbc::TypeId::BYTE_LIST, 1)] {
                let list = allocate.call(3, encoding.0);
                assert_eq!(replace.call(list), 42);
                let data = *((list as *const u64).add(5)) as *const u8;
                if width == 1 {
                    assert_eq!(std::slice::from_raw_parts(data, 3), [7, 255, 91]);
                } else {
                    assert_eq!(std::slice::from_raw_parts(data.cast::<u64>(), 3), [7, 255, 91]);
                }
            }
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
            fn copied(values:&List<Int>)->List<Int> {values.clone()}
            fn storage_write_probe(values:&mut List<Int>, index:Int)->Int {@intrinsic("list_storage_write",values,index,37); 0}
            fn movement(values:&mut List<Int>, source:Int,target:Int,count:Int)->Int {@intrinsic("list_storage_move",values,source,target,count); 0}
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
                    "clone-null" | "pop-null" | "clone-length" | "clone-shape" => {
                        if case == "clone-length" {
                            *((handle as *mut u64).add(3)) = 3;
                        } else if case == "clone-shape" {
                            *((handle as *mut u32).add(3)) = 24;
                        } else {
                            *((handle as *mut u64).add(5)) = 0;
                        }
                        let name = if case == "pop-null" {
                            "test_storage_pop"
                        } else {
                            "copied"
                        };
                        jit.get_function::<unsafe extern "C" fn(u64) -> u64>(name)
                            .unwrap()
                            .call(handle);
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
        "clone-null",
        "pop-null",
        "clone-length",
        "clone-shape",
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
        fn store(values:&mut List<Float>, value:Float)->Int {
            let done = 0;
            @intrinsic("list_storage_write", values, 0, value); done
        }
        fn fetch(values:&List<Float>)->Float { let value = @intrinsic("list_storage_read", values, 1); value }
        fn probe()->Float {
            let mut values:List<Float> =List<Float>.with_capacity(2);
            store(&mut values,1.25);
            @intrinsic("list_storage_move", values, 0, 1, 1);
            fetch(&values)
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
        r#"mount core.intrinsics.memory.{list_storage_write, list_storage_move, list_storage_read};
        type Triple is {x:Int,y:Int,z:Int}; fn probe()->Int {
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

#[test]
fn internal_storage_allocator_keeps_exact_encoding_and_extent() {
    with_source(
        "fn ordinary()->List<Int> {List<Int>.new()}",
        |_, jit| unsafe {
            let allocate = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                    "verum_list_allocate_storage",
                )
                .expect("internal storage allocator");
            for (encoding, width) in [(512, 8), (527, 1)] {
                for capacity in [0, 3, 40] {
                    let handle = allocate.call(capacity, encoding);
                    assert_eq!(*(handle as *const u32), encoding);
                    assert_eq!(*((handle as *const u64).add(3)), 0);
                    assert_eq!(*((handle as *const u64).add(4)), capacity);
                    let data = *((handle as *const u64).add(5)) as *const u8;
                    if capacity == 0 {
                        assert!(data.is_null());
                    } else {
                        assert_eq!(
                            *(data.sub(verum_common::layout::ALLOCATION_HEADER_SIZE as usize)
                                as *const u32),
                            (capacity * width) as u32
                        );
                    }
                }
            }
        },
    );
}

#[test]
fn packed_and_slot_source_value_methods_survive_shrink_and_regrow() {
    with_source(
        r#"
        fn ordinary()->List<Int> {List<Int>.new()}
        fn copy(values:&List<Byte>)->List<Byte> {values.clone()}
        fn exercise(values:&mut List<Byte>)->Int {
            values.shrink_to_fit();
            values.reserve(3);
            values.checked_push(0);
            values.checked_push(255);
            values.checked_push(42);
            values.checked_insert(1,17);
            let removed=values.checked_remove(1);
            values.checked_set(0,7);
            values.checked_swap(0,2);
            let popped=match values.checked_pop() {Maybe.Some(x)=>x,Maybe.None=>10000};
            values.shrink_to_fit();
            values.reserve(17);
            let first=match values.checked_get(0) {Maybe.Some(x)=>x,Maybe.None=>10000};
            let second=match values.checked_get(1) {Maybe.Some(x)=>x,Maybe.None=>10000};
            removed+popped+first+second
        }
    "#,
        |module, jit| unsafe {
            let exercise = module.get_function("exercise").expect("compiled source caller");
            let source_get = exercise.get_basic_blocks().into_iter()
                .flat_map(|block| block.get_instructions())
                .filter_map(|instruction| CallSiteValue::try_from(instruction).ok())
                .filter_map(|call| call.get_called_fn_value())
                .find(|callee| {
                    callee.get_name().to_str().is_ok_and(|name| {
                        name.split('$').next() == Some("core.collections.list.List.checked_get")
                    })
                })
                .expect("exercise calls the actual source get body");
            let source_get = source_get.print_to_string();
            let source_get = source_get.to_str().unwrap();
            assert!(
                source_get.contains("list_storage_read"),
                "get still uses raw pointee layout: {source_get}"
            );
            let allocate = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                    "verum_list_allocate_storage",
                )
                .unwrap();
            let exercise = jit
                .get_function::<unsafe extern "C" fn(u64) -> i64>("exercise")
                .unwrap();
            for encoding in [512, 527] {
                let list = allocate.call(0, encoding);
                assert_eq!(exercise.call(list), 321, "encoding {encoding}");
                assert_eq!(*(list as *const u32), encoding);
                assert_eq!(*((list as *const u64).add(3)), 2);
                assert!(*((list as *const u64).add(4)) >= 19);
                let copied = jit
                    .get_function::<unsafe extern "C" fn(u64) -> u64>("copy")
                    .unwrap()
                    .call(list);
                assert_eq!(*(copied as *const u32), encoding);
                let pop = jit
                    .get_function::<unsafe extern "C" fn(u64) -> u64>("test_storage_pop")
                    .unwrap();
                assert_eq!(pop.call(copied), 255);
                assert_eq!(pop.call(copied), 42);
                assert_eq!(pop.call(copied), verum_vbc::value::nanbox::NAN_UNIT_HEADER);
                assert_eq!(*((list as *const u64).add(3)), 2);
                let empty = allocate.call(0, encoding);
                assert_eq!(pop.call(empty), verum_vbc::value::nanbox::NAN_UNIT_HEADER);
            }
        },
    );
}

#[test]
fn allocator_rejects_unknown_encoding_and_overflow_before_allocating() {
    const CHILD: &str = "VERUM_TEST_LIST_ALLOCATION_REFUSAL";
    if let Some(case) = std::env::var_os(CHILD) {
        with_source(
            "fn ordinary()->List<Int>{List<Int>.new()}",
            |_, jit| unsafe {
                let allocator = jit
                    .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                        "verum_list_allocate_storage",
                    )
                    .unwrap();
                let (capacity, encoding) = match case.to_str().unwrap() {
                    "encoding" => (0, 99999),
                    "negative" => (u64::MAX, 512),
                    "slots" => (u32::MAX as u64 / 8 + 1, 512),
                    "bytes" => (u32::MAX as u64 + 1, 527),
                    _ => panic!("unknown case"),
                };
                allocator.call(capacity, encoding);
            },
        );
        panic!("invalid allocation returned");
    }
    for case in ["encoding", "negative", "slots", "bytes"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "allocator_rejects_unknown_encoding_and_overflow_before_allocating",
                "--nocapture",
            ])
            .env(CHILD, case)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{case}");
        assert_ne!(
            result.status.code(),
            Some(101),
            "{case}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn resize_failure_preserves_owner_and_only_initialized_prefix_moves() {
    with_source(
        r#"
        mount core.intrinsics.memory.list_storage_resize;
        fn ordinary()->List<Int>{List<Int>.new()}
        fn initialize(values:&mut List<Int>) {values.checked_push(37);}
        fn resize(values:&mut List<Int>,cap:Int)->Bool {unsafe {list_storage_resize(values,cap)}}
    "#,
        |_, jit| unsafe {
            let allocate = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                    "verum_list_allocate_storage",
                )
                .unwrap();
            let initialize = jit
                .get_function::<unsafe extern "C" fn(u64)>("initialize")
                .unwrap();
            let resize = jit
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("resize")
                .unwrap();
            let write = jit
                .get_function::<unsafe extern "C" fn(u64, u64, u64)>("verum_list_storage_write")
                .unwrap();
            let read = jit
                .get_function::<unsafe extern "C" fn(u64, u64) -> u64>("verum_list_storage_read")
                .unwrap();
            for encoding in [512, 527] {
                let owner = allocate.call(4, encoding);
                initialize.call(owner);
                write.call(owner, 3, 99); // outside logical len; must not be copied
                let fields = owner as *const u64;
                let old_pointer = *fields.add(5);
                for capacity in [0, u64::MAX, u32::MAX as u64 + 1] {
                    assert_eq!(resize.call(owner, capacity), 0);
                    assert_eq!(*fields.add(3), 1);
                    assert_eq!(*fields.add(4), 4);
                    assert_eq!(*fields.add(5), old_pointer);
                    assert_eq!(read.call(owner, 0), 37);
                }
                FAIL_ALLOCATION.set(true);
                let failed = resize.call(owner, 8);
                FAIL_ALLOCATION.set(false);
                assert_eq!(failed, 0, "OS allocation failure must be recoverable");
                assert_eq!(*fields.add(3), 1);
                assert_eq!(*fields.add(4), 4);
                assert_eq!(*fields.add(5), old_pointer);
                assert_eq!(read.call(owner, 0), 37);
                assert_eq!(resize.call(owner, 8), 1);
                assert_eq!(read.call(owner, 0), 37);
                assert_eq!(read.call(owner, 3), 0, "uninitialized tail was copied");
            }
        },
    );
}

#[test]
fn fallible_source_resize_reports_oom_without_losing_values() {
    with_source(
        r#"
        fn ordinary()->List<Int>{List<Int>.new()}
        fn initialize(values:&mut List<Int>) {values.checked_push(37);}
        fn attempt(values:&mut List<Int>)->Int {
            match values.try_resize_buffer(8) {
                Result.Ok(())=>1,
                Result.Err(AllocError.OutOfMemory{requested})=>2,
                _=>10000,
            }
        }
    "#,
        |_, jit| unsafe {
            let allocate = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                    "verum_list_allocate_storage",
                )
                .unwrap();
            let initialize = jit
                .get_function::<unsafe extern "C" fn(u64)>("initialize")
                .unwrap();
            let attempt = jit
                .get_function::<unsafe extern "C" fn(u64) -> i64>("attempt")
                .unwrap();
            for encoding in [512, 527] {
                let owner = allocate.call(4, encoding);
                initialize.call(owner);
                let fields = owner as *const u64;
                let pointer = *fields.add(5);
                FAIL_ALLOCATION.set(true);
                let failure = attempt.call(owner);
                FAIL_ALLOCATION.set(false);
                assert_eq!(failure, 2);
                assert_eq!(*fields.add(5), pointer);
                assert_eq!(*fields.add(3), 1);
                assert_eq!(*fields.add(4), 4);
                assert_eq!(attempt.call(owner), 1);
                assert_eq!(*fields.add(4), 8);
            }
        },
    );
}

#[test]
fn source_constructors_use_tracked_storage_for_byte_and_record_values() {
    with_source(
        r#"
        type Record is {a:Int,b:Int,c:Int};
        fn bytes()->Byte {
            let mut values:List<Byte> = List<Byte>.checked_with_capacity(3);
            values.checked_push(255);
            match values.checked_get(0) {Maybe.Some(x)=>x,_=>0}
        }
        fn records()->Int {
            let attempt: Result<List<Record>, AllocError> = List<Record>.checked_try_with_capacity(2);
            let mut values:List<Record> = match attempt {Result.Ok(xs)=>xs,_=>List.new()};
            values.checked_push(Record{a:0,b:42,c:0});
            match values.checked_get(0) {Maybe.Some(x)=>x.b,_=>10000}
        }
    "#,
        |_, jit| unsafe {
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> u8>("bytes")
                    .unwrap()
                    .call(),
                255
            );
            assert_eq!(
                jit.get_function::<unsafe extern "C" fn() -> i64>("records")
                    .unwrap()
                    .call(),
                42
            );
        },
    );
}

#[test]
fn source_clear_truncate_release_and_regrow_keep_owner_valid() {
    with_source(
        r#"
        fn cycle(values:&mut List<Int>)->Int {
            values.checked_push(255); values.checked_push(42);
            values.checked_truncate(1); values.checked_clear(); values.shrink_to_fit();
            assert(values.len()==0 && values.capacity()==0);
            values.reserve(4); values.checked_push(37);
            match values.checked_get(0) {Maybe.Some(x)=>x,_=>10000}
        }
    "#,
        |_, jit| unsafe {
            let allocate = jit
                .get_function::<unsafe extern "C" fn(u64, u32) -> u64>(
                    "verum_list_allocate_storage",
                )
                .unwrap();
            let cycle = jit
                .get_function::<unsafe extern "C" fn(u64) -> i64>("cycle")
                .unwrap();
            for encoding in [512, 527] {
                assert_eq!(cycle.call(allocate.call(0, encoding)), 37);
            }
        },
    );
}
