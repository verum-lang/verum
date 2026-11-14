//! Native reference producers must share the VM's epoch/capability encoding.
use std::sync::atomic::Ordering;
use verum_codegen::llvm::CbgrLowering;
use verum_common::{cbgr::AllocationHeader, layout};
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{InitializationConfig, Target},
};

#[repr(C, align(32))]
struct Allocation {
    header: AllocationHeader,
    data: [u8; 32],
}

#[test]
fn actual_thin_ref_producer_keeps_epoch_separate_from_capabilities() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let context = Context::create();
    let module = context.create_module("thin_ref_producer");
    let builder = context.create_builder();
    let pointer = context.ptr_type(Default::default());
    let function = module.add_function(
        "packed",
        context.i32_type().fn_type(&[pointer.into()], false),
        None,
    );
    let entry = context.append_basic_block(function, "entry");
    builder.position_at_end(entry);
    let mut cbgr = CbgrLowering::new(&context);
    let reference = cbgr
        .create_thin_ref(
            &builder,
            function.get_first_param().unwrap().into_pointer_value(),
        )
        .unwrap();
    let packed = builder
        .build_extract_value(reference, 2, "actual_packed")
        .unwrap();
    builder.build_return(Some(&packed)).unwrap();
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: the generated function takes one pointer and returns its packed u32.
    let packed = unsafe {
        engine
            .get_function::<unsafe extern "C" fn(*mut u8) -> u32>("packed")
            .unwrap()
    };
    for epoch in [1u16, 0x1234, u16::MAX] {
        for caps in [1u16, 3, 0, 0x00ff] {
            let mut allocation = Allocation {
                header: AllocationHeader::new(32, 32, 1, caps),
                data: [0; 32],
            };
            allocation.header.store_epoch(epoch, Ordering::Relaxed);
            // SAFETY: the live allocation has the real 32-byte header immediately
            // preceding the pointer read by CbgrLowering::create_thin_ref.
            let actual = unsafe { packed.call(allocation.data.as_mut_ptr()) };
            assert_eq!(
                actual,
                (u32::from(epoch) << layout::CAPS_BITS) | u32::from(caps),
                "epoch={epoch:#x}, caps={caps:#x}"
            );
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Snapshot {
    pointer: *mut u8,
    generation: u32,
    epoch_caps: u32,
}

#[test]
fn actual_native_checks_distinguish_lifetime_and_capability_refusals() {
    use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
    use verum_common::{Text, cbgr::caps};
    use verum_llvm::{memory_buffer::MemoryBuffer, values::AnyValue};
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let context = Context::create();
    let ast = verum_fast_parser::Parser::new("fn main() {}")
        .parse_module()
        .unwrap();
    let vbc = verum_vbc::codegen::VbcCodegen::new()
        .compile_module(&ast)
        .unwrap();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("reference_validation").with_debug_info(false),
    );
    lower.lower_module(&vbc).unwrap();
    let module = lower.module();
    let builder = context.create_builder();
    let pointer = context.ptr_type(Default::default());
    let make = module.add_function(
        "snapshot",
        context
            .void_type()
            .fn_type(&[pointer.into(), pointer.into()], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(make, "entry"));
    let mut cbgr = CbgrLowering::new(&context);
    let snapshot = cbgr
        .create_thin_ref(
            &builder,
            make.get_nth_param(0).unwrap().into_pointer_value(),
        )
        .unwrap();
    builder
        .build_store(
            make.get_nth_param(1).unwrap().into_pointer_value(),
            snapshot,
        )
        .unwrap();
    builder.build_return(None).unwrap();
    for (name, required) in [
        ("direct_read", caps::READ),
        ("direct_write", caps::WRITE),
        ("direct_both", caps::READ | caps::WRITE),
    ] {
        let function = module.add_function(
            name,
            context.bool_type().fn_type(&[pointer.into()], false),
            None,
        );
        builder.position_at_end(context.append_basic_block(function, "entry"));
        let snapshot = builder
            .build_load(
                cbgr.thin_ref_type(),
                function.get_first_param().unwrap().into_pointer_value(),
                "snapshot",
            )
            .unwrap()
            .into_struct_value();
        let valid = cbgr
            .reference_is_valid(&builder, &module, snapshot, required)
            .unwrap();
        builder.build_return(Some(&valid)).unwrap();
    }
    // Keep the actual emitted validator and its callers; unrelated platform
    // services are not replaced by host-side doubles in this focused JIT gate.
    let mut definitions = Text::new();
    for name in [
        "snapshot",
        "direct_read",
        "direct_write",
        "direct_both",
        "verum_cbgr_validate_ref",
        "verum_cbgr_check",
        "verum_cbgr_check_write",
        "verum_cbgr_check_fat",
    ] {
        let function = module.get_function(name).unwrap();
        assert!(function.verify(true), "{name}");
        definitions.push_str(&function.print_to_string().to_string());
        definitions.push('\n');
    }
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            definitions.as_bytes(),
            "actual_reference_runtime",
        ))
        .unwrap();
    executable.verify().unwrap();
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: these exact ABI types match the emitted pointer arguments. Every
    // allocation/header below remains alive through its validation call.
    unsafe {
        let make = engine
            .get_function::<unsafe extern "C" fn(*mut u8, *mut Snapshot)>("snapshot")
            .unwrap();
        let read = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("direct_read")
            .unwrap();
        let write = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("direct_write")
            .unwrap();
        let both = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("direct_both")
            .unwrap();
        let public_read = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("verum_cbgr_check")
            .unwrap();
        let public_write = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("verum_cbgr_check_write")
            .unwrap();
        let public_fat = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> bool>("verum_cbgr_check_fat")
            .unwrap();
        let mut allocation = Allocation {
            header: AllocationHeader::new(32, 32, 1, caps::ALL as u16),
            data: [0; 32],
        };
        allocation.header.store_epoch(0x1234, Ordering::Release);
        let mut reference = Snapshot {
            pointer: std::ptr::null_mut(),
            generation: 0,
            epoch_caps: 0,
        };
        make.call(allocation.data.as_mut_ptr(), &mut reference);
        assert_eq!(
            reference.epoch_caps,
            (0x1234 << layout::CAPS_BITS) | caps::ALL
        );
        assert!(read.call(&reference));
        assert!(write.call(&reference));
        assert!(both.call(&reference));
        assert!(public_read.call(&reference));
        assert!(public_write.call(&reference));
        assert!(public_fat.call(&reference));

        allocation
            .header
            .store_generation(reference.generation + 1, Ordering::Release);
        assert!(
            !read.call(&reference),
            "stale generation on a live allocation"
        );
        assert!(!public_read.call(&reference));
        allocation
            .header
            .store_generation(reference.generation, Ordering::Release);
        allocation.header.store_epoch(0x1235, Ordering::Release);
        assert!(!read.call(&reference), "stale epoch on a live allocation");
        assert!(!public_fat.call(&reference));
        allocation.header.store_epoch(0x1234, Ordering::Release);

        let readonly = Snapshot {
            epoch_caps: (0x1234 << layout::CAPS_BITS) | caps::READ,
            ..reference
        };
        assert!(read.call(&readonly));
        assert!(!write.call(&readonly), "readonly snapshot cannot write");
        assert!(!both.call(&readonly), "one requested bit cannot grant both");
        assert!(!public_write.call(&readonly));
        allocation
            .header
            .store_capabilities(caps::READ as u16, Ordering::Release);
        assert!(
            !write.call(&reference),
            "allocation can revoke snapshot authority"
        );
        assert!(!public_write.call(&reference));
        assert!(read.call(&reference));
        allocation.header.store_capabilities(0, Ordering::Release);
        assert!(!read.call(&reference), "revoked read capability");

        let null = Snapshot {
            pointer: std::ptr::null_mut(),
            ..reference
        };
        assert!(
            !read.call(&null),
            "null is refused before accessing its header"
        );
        let zero = Snapshot {
            pointer: 1usize as *mut u8,
            generation: 0,
            ..reference
        };
        assert!(
            !read.call(&zero),
            "invalid generation is refused before header access"
        );
    }
}

#[test]
fn explicit_check_and_dereference_instructions_emit_real_validation() {
    use verum_codegen::llvm::{FunctionContext, instruction::lower_instruction};
    use verum_llvm::values::AnyValue;
    use verum_vbc::{Reg, instruction::Instruction};
    let context = Context::create();
    let module = context.create_module("instruction_reference_checks");
    for (name, operation) in [
        ("explicit_check", Instruction::ChkRef { ref_reg: Reg(0) }),
        (
            "read_reference",
            Instruction::Deref {
                dst: Reg(1),
                ref_reg: Reg(0),
            },
        ),
    ] {
        let function = module.add_function(
            name,
            context
                .i32_type()
                .fn_type(&[context.ptr_type(Default::default()).into()], false),
            None,
        );
        let mut ctx = FunctionContext::new(&context, &module, function, name);
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        let snapshot = ctx
            .builder()
            .build_load(
                CbgrLowering::new(&context).thin_ref_type(),
                function.get_first_param().unwrap().into_pointer_value(),
                "actual_snapshot",
            )
            .unwrap();
        ctx.set_register(0, snapshot);
        lower_instruction(&mut ctx, &operation).unwrap();
        ctx.builder()
            .build_return(Some(&context.i32_type().const_int(73, false)))
            .unwrap();
        assert!(function.verify(true));
        let ir = function.print_to_string().to_string();
        assert!(
            ir.contains("call i1 @verum_cbgr_validate_ref"),
            "{name}: {ir}"
        );
        assert!(
            ir.contains("call void @verum_panic"),
            "invalid references need an enforced refusal: {ir}"
        );
    }
    module.verify().unwrap();
}
