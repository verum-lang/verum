//! Native ring operations use the declared owner, initialized run and allocation extent.
use std::process::Command;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Text, layout};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    types::TypeId,
};

extern "C" fn rejected(status: u64) {
    std::process::exit(72 + status as i32);
}

#[test]
fn declared_slots_transfer_borrow_and_reject_invalid_extents() {
    const MODE: &str = "VERUM_TEST_DEQUE_INVALID_SLOT";
    let mut source = Parser::new(include_str!("../../../core/collections/deque.vr"))
        .parse_module()
        .unwrap();
    source.items.retain(|item| {
        matches!(&item.kind, verum_ast::ItemKind::Module(m) if m.items.is_none())
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Deque")
    });
    let methods = Parser::new(
        r#"
        public fn take_slot(q: &mut Deque<Int>, i: Int) -> Int {
            @intrinsic("deque_storage_take", q, i)
        }
        public fn write_slot(q: &mut Deque<Int>, i: Int, v: Int) {
            @intrinsic("deque_storage_write", q, i, v)
        }
        public fn borrow_slot(q: &mut Deque<Int>, i: Int) -> &mut Int {
            @intrinsic("deque_storage_ref_mut", q, i)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    source.items.extend(methods.items);
    let vbc = VbcCodegen::with_config(CodegenConfig::new("core.collections.deque"))
        .compile_module(&source)
        .unwrap();
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("ring").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let mut ir = Text::from("declare void @verum_internal_exit_i64(i64)\n");
    let address = lower
        .module()
        .get_function("verum_deque_storage_slot")
        .unwrap();
    ir.push_str(address.print_to_string().to_str().unwrap());
    let actual_name = |suffix: &str| {
        let f = vbc
            .functions
            .iter()
            .find(|f| vbc.get_string(f.name).is_some_and(|n| n.ends_with(suffix)))
            .unwrap();
        lower.get_function_by_id(f.id.0).unwrap()
    };
    let wrappers = [
        actual_name("take_slot"),
        actual_name("write_slot"),
        actual_name("borrow_slot"),
    ];
    for f in wrappers {
        ir.push_str(f.print_to_string().to_str().unwrap());
    }
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let exe = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "ring",
        ))
        .unwrap();
    exe.verify().unwrap();
    let engine = exe
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_global_mapping(
        &exe.get_function("verum_internal_exit_i64").unwrap(),
        rejected as *const () as usize,
    );
    // The backing includes the documented CBGR header. The runtime owner is
    // the ordinary 24-byte ObjectHeader followed by its four declared words.
    const N: usize = layout::ALLOCATION_HEADER_SIZE as usize / 8;
    let mut backing = [0_u64; N + 4];
    backing[layout::ALLOCATION_HEADER_SIZE_OFFSET as usize / 8] = 32;
    let data = backing[N..].as_mut_ptr();
    let mut owner = [0_u64; 7];
    owner[0] = TypeId::DEQUE.0.into();
    let shape = layout::OBJECT_HEADER_SIZE_OFFSET as usize;
    // SAFETY: shape is the aligned u32 ObjectHeader.size within our owner.
    unsafe {
        owner
            .as_mut_ptr()
            .cast::<u8>()
            .add(shape)
            .cast::<u32>()
            .write(56);
    }
    owner[3..].copy_from_slice(&[data as u64, 3, 2, 4]);
    let owner_ptr = owner.as_mut_ptr();
    // SAFETY: the three lowered source declarations take an address word and
    // integer slots. The stack allocations outlive all native calls.
    unsafe {
        let take = engine
            .get_function::<unsafe extern "C" fn(*mut u64, u64) -> u64>(
                wrappers[0].get_name().to_str().unwrap(),
            )
            .unwrap();
        let write = engine
            .get_function::<unsafe extern "C" fn(*mut u64, u64, u64)>(
                wrappers[1].get_name().to_str().unwrap(),
            )
            .unwrap();
        let borrow = engine
            .get_function::<unsafe extern "C" fn(*mut u64, u64) -> *mut u64>(
                wrappers[2].get_name().to_str().unwrap(),
            )
            .unwrap();
        match std::env::var(MODE).as_deref() {
            Ok("vacant") => {
                borrow.call(owner_ptr, 1);
                panic!("borrowed vacant slot");
            }
            Ok("extent") => {
                backing[0] = 33;
                std::hint::black_box(&backing);
                take.call(owner_ptr, 3);
                panic!("accepted a partial extra slot");
            }
            _ => {}
        }
        write.call(owner_ptr, 3, 41);
        write.call(owner_ptr, 0, 17);
        let slot = borrow.call(owner_ptr, 3);
        assert_eq!(slot, data.add(3));
        slot.write(42);
        assert_eq!(take.call(owner_ptr, 3), 42);
        assert_eq!(backing[N + 3], 0);
        assert_eq!(take.call(owner_ptr, 0), 17);
    }
    for mode in ["vacant", "extent"] {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "declared_slots_transfer_borrow_and_reject_invalid_extents",
            ])
            .env(MODE, mode)
            .status()
            .unwrap();
        assert_eq!(child.code(), Some(73), "{mode}");
    }
}
