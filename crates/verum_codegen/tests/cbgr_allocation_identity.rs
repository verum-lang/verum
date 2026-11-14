//! Allocation headers and explicit generation requests use one bounded issuer.
#[path = "support/source_image.rs"]
mod source_image;

const SOURCE: &str = r#"
    @intrinsic("cbgr_allocate") unsafe fn allocate(size:Int, align:Int)->Int;
    @intrinsic("cbgr_deallocate") unsafe fn release(address:Int);
    @intrinsic("cbgr_get_generation") unsafe fn generation(address:*const Byte)->UInt32;
    @intrinsic("cbgr_new_generation") fn reserve_generation()->Int;
    fn main()->Int {
        unsafe {
            let first = allocate(64,8);
            let first_generation = generation(first as *const Byte) as Int;
            let reserved = reserve_generation();
            let second = allocate(64,8);
            let second_generation = generation(second as *const Byte) as Int;
            release(first);
            release(second);
            if first_generation > 0 && first_generation < reserved && reserved < second_generation {73} else {0}
        }
    }
"#;

#[test]
fn bridge_allocations_and_explicit_requests_share_vm_identity() {
    let wire = source_image::module(SOURCE);
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
}

#[test]
#[cfg(target_os = "macos")]
fn bridge_allocations_and_explicit_requests_share_native_identity() {
    source_image::run(SOURCE);
}

fn exhausted_source() -> verum_common::Text {
    format!(
        r#"{}
        fn main()->Int {{
            unsafe {{
                let last = allocate(64,8);
                if last == 0 {{ return 1; }}
                if generation(last as *const Byte) as Int != {} {{ return 2; }}
                let refused = allocate(64,8);
                if refused != 0 {{ return 3; }}
                let explicit_refused = try {{ reserve_generation(); 0 }} recover |_| {{ 1 }};
                if explicit_refused != 1 {{ return 4; }}
                release(last);
                if allocate(64,8) != 0 {{ return 5; }}
                73
            }}
        }}
    "#,
        SOURCE.split("    fn main()").next().unwrap(),
        verum_common::cbgr::GEN_MAX
    )
    .into()
}

#[test]
fn vm_issuer_exhaustion_does_not_allocate_or_reuse_zero() {
    let wire = source_image::module(&exhausted_source());
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.into());
    vm.state.cbgr_generation_counter = u64::from(verum_common::cbgr::GEN_MAX) - 1;
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    assert_eq!(
        vm.state.cbgr_generation_counter,
        u64::from(verum_common::cbgr::GEN_MAX)
    );
    assert!(vm.state.cbgr_allocations.is_empty());
    assert!(vm.state.cbgr_bridge_extents.is_empty());
}

#[test]
#[cfg(target_os = "macos")]
fn native_issuer_exhaustion_refuses_allocation_and_explicit_request() {
    source_image::run_with(&exhausted_source(), |module| {
        let counter = module.get_global("verum_ir_generation_counter").unwrap();
        let ty = counter
            .get_initializer()
            .unwrap()
            .into_int_value()
            .get_type();
        counter.set_initializer(&ty.const_int(u64::from(verum_common::cbgr::GEN_MAX) - 1, false));
    });
}

#[test]
fn native_generation_compare_exchange_is_bounded_under_competing_issuers() {
    use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
    use verum_common::List;
    use verum_llvm::{
        OptimizationLevel,
        context::Context,
        memory_buffer::MemoryBuffer,
        targets::{InitializationConfig, Target},
        values::AnyValue,
    };
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("generation_boundary").with_debug_info(false),
    );
    lowering
        .lower_module(&source_image::module("fn main() {} "))
        .unwrap();
    let module = lowering.module();
    let counter = module.get_global("verum_ir_generation_counter").unwrap();
    let issuer = module
        .get_function("verum_cbgr_try_new_generation")
        .unwrap();
    assert!(issuer.verify(true));
    let ty = counter
        .get_initializer()
        .unwrap()
        .into_int_value()
        .get_type();
    let initial = u64::from(verum_common::cbgr::GEN_MAX) - 128;
    counter.set_initializer(&ty.const_int(initial, false));
    let source = format!(
        "{}\n{}",
        counter.print_to_string().to_string(),
        issuer.print_to_string().to_string()
    );
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            source.as_bytes(),
            "actual_generation_issuer",
        ))
        .unwrap();
    executable.verify().unwrap();
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    let address = engine
        .get_function_address("verum_cbgr_try_new_generation")
        .unwrap();
    // SAFETY: the emitted function has no arguments and returns i64. Its code
    // and global remain owned by engine until all scoped workers have joined.
    let issue: unsafe extern "C" fn() -> u64 = unsafe { std::mem::transmute(address) };
    let issued: List<u64> = std::thread::scope(|scope| {
        let workers: List<_> = (0..8)
            .map(|_| scope.spawn(move || (0..64).map(|_| {
                        // SAFETY: all workers share the emitted atomic issuer;
                        // the owning engine outlives this scoped execution.
                        unsafe { issue() }
                    }).collect::<List<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect()
    });
    let mut successful: List<_> = issued.iter().copied().filter(|id| *id != 0).collect();
    successful.sort_unstable();
    assert_eq!(
        successful,
        (initial + 1..=u64::from(verum_common::cbgr::GEN_MAX)).collect::<List<_>>()
    );
    assert_eq!(issued.iter().filter(|id| **id == 0).count(), 8 * 64 - 128);
    for _ in 0..8 {
        // SAFETY: the same actual issuer and engine remain live.
        assert_eq!(
            unsafe { issue() },
            0,
            "exhaustion cannot wrap or mint zero as an identity"
        );
    }
}

fn retired_source() -> verum_common::Text {
    format!(
        r#"{}
        @intrinsic("cbgr_invalidate") unsafe fn retire(address: Int);
        fn main() -> Int {{
            unsafe {{
                let first = allocate(64, 8);
                if first == 0 {{ return 1; }}
                let former = generation(first as *const Byte) as Int;
                retire(first);
                if generation(first as *const Byte) as Int != 0 {{ return 2; }}
                let second = allocate(64, 8);
                if second == 0 {{ return 3; }}
                let next = generation(second as *const Byte) as Int;
                release(first);
                release(second);
                if former > 0 && next > former {{ 73 }} else {{ 4 }}
            }}
        }}
    "#,
        SOURCE.split("    fn main()").next().unwrap()
    )
    .into()
}

#[test]
fn vm_retirement_cannot_publish_a_future_allocation_identity() {
    let wire = source_image::module(&retired_source());
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    assert!(vm.state.cbgr_allocations.is_empty());
}

#[test]
#[cfg(target_os = "macos")]
fn native_retirement_cannot_publish_a_future_allocation_identity() {
    source_image::run(&retired_source());
}

#[test]
fn native_invalidation_rejects_a_non_address_operand() {
    use verum_codegen::llvm::{FunctionContext, instruction::lower_instruction};
    use verum_llvm::context::Context;
    use verum_vbc::instruction::{CbgrSubOpcode, Instruction};
    let context = Context::create();
    let module = context.create_module("invalidation_operand");
    let function = module.add_function("refuse", context.void_type().fn_type(&[], false), None);
    let mut ctx = FunctionContext::new(&context, &module, function, "refuse");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, context.f64_type().const_float(1.0).into());
    let result = lower_instruction(
        &mut ctx,
        &Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::Invalidate as u8,
            operands: vec![0],
        },
    );
    assert!(
        matches!(result, Err(verum_codegen::llvm::LlvmLoweringError::TypeLowering(message))
        if message.contains("requires an address operand")),
        "non-address must be refused before a runtime call"
    );
}
