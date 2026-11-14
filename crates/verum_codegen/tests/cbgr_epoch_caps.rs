//! GetEpochCaps consumes a saved snapshot, not an allocation header or a name.
use std::sync::atomic::Ordering;
use verum_codegen::llvm::{
    CbgrLowering, FunctionContext, LlvmLoweringError, instruction::lower_instruction,
};
use verum_common::{cbgr::AllocationHeader, layout};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::{
    bytecode::decode_instructions,
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    encoding::decode_reg,
    instruction::{CbgrSubOpcode, Instruction},
    serialize::serialize_module,
};

fn source_operation() -> (Instruction, u16, u16) {
    let source = Parser::new(
        "fn epoch_caps(reference: &Int) -> UInt32 { @vbc(CBGR_GET_EPOCH_CAPS, reference) }",
    )
    .parse_module()
    .unwrap();
    let original = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let find = |module: &verum_vbc::module::VbcModule| {
        decode_instructions(&module.bytecode).unwrap().into_iter().find(|operation|
            matches!(operation, Instruction::CbgrExtended { sub_op, .. } if *sub_op == CbgrSubOpcode::GetEpochCaps as u8)
        ).expect("real source emitted the registered opcode")
    };
    let operation = find(&wire);
    assert_eq!(find(&original), operation);
    let Instruction::CbgrExtended { operands, .. } = &operation else {
        unreachable!()
    };
    let mut offset = 0;
    let dst = decode_reg(operands, &mut offset).unwrap().0;
    let src = decode_reg(operands, &mut offset).unwrap().0;
    assert_eq!(offset, operands.len());
    (operation, dst, src)
}

#[repr(C, align(32))]
struct Allocation {
    header: AllocationHeader,
    data: [u64; 4],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Snapshot {
    pointer: *mut u8,
    generation: u32,
    epoch_caps: u32,
}
#[repr(C)]
struct FatSnapshot {
    thin: Snapshot,
    metadata: u64,
    offset: u32,
    reserved: u32,
}

#[test]
fn source_opcode_reads_actual_native_producer_snapshot_after_header_changes() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let (operation, dst, src) = source_operation();
    let context = Context::create();
    let module = context.create_module("saved_epoch_caps");
    let builder = context.create_builder();
    let pointer = context.ptr_type(Default::default());
    let make = module.add_function(
        "make",
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
    // An unrelated same-spelling function must never supply snapshot metadata.
    let impostor = module.add_function(
        "ThinRef.capabilities",
        context
            .i16_type()
            .fn_type(&[context.i64_type().into()], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(impostor, "entry"));
    builder
        .build_return(Some(&context.i16_type().const_int(7, false)))
        .unwrap();
    for (name, shape) in [
        ("read_thin", cbgr.thin_ref_type()),
        ("read_fat", cbgr.fat_ref_type()),
    ] {
        let function = module.add_function(
            name,
            context.i64_type().fn_type(&[pointer.into()], false),
            None,
        );
        let mut ctx = FunctionContext::new(&context, &module, function, name);
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        let snapshot = ctx
            .builder()
            .build_load(
                shape,
                function.get_first_param().unwrap().into_pointer_value(),
                "saved_snapshot",
            )
            .unwrap();
        ctx.set_register(src, snapshot);
        lower_instruction(&mut ctx, &operation).expect("canonical saved snapshot");
        ctx.builder()
            .build_return(Some(&ctx.get_register(dst).unwrap()))
            .unwrap();
        let ir = function.print_to_string().to_string();
        assert!(
            !ir.contains("call "),
            "snapshot extraction does not dispatch by name: {ir}"
        );
        assert!(
            !ir.contains("getelementptr"),
            "only the saved carrier is read: {ir}"
        );
    }
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: emitted signatures match these host declarations. `make` receives
    // a live canonical AllocationHeader; readers load only the live snapshots.
    unsafe {
        let make = engine
            .get_function::<unsafe extern "C" fn(*mut u8, *mut Snapshot)>("make")
            .unwrap();
        let read = engine
            .get_function::<unsafe extern "C" fn(*const Snapshot) -> u64>("read_thin")
            .unwrap();
        let read_fat = engine
            .get_function::<unsafe extern "C" fn(*const FatSnapshot) -> u64>("read_fat")
            .unwrap();
        for epoch in [0u16, 1, 0xA55A, u16::MAX] {
            for caps in [0u16, 0x41, 0xBF, 0x8001] {
                let mut allocation = Allocation {
                    header: AllocationHeader::new(32, 32, 1, caps),
                    data: [17, 29, 31, 37],
                };
                allocation.header.store_epoch(epoch, Ordering::Relaxed);
                let mut snapshot = Snapshot {
                    pointer: std::ptr::null_mut(),
                    generation: 0,
                    epoch_caps: 0,
                };
                make.call(allocation.data.as_mut_ptr().cast(), &mut snapshot);
                let expected = (u64::from(epoch) << layout::CAPS_BITS) | u64::from(caps);
                assert_eq!(u64::from(snapshot.epoch_caps), expected);
                allocation.header.store_epoch(!epoch, Ordering::Relaxed);
                allocation
                    .header
                    .store_capabilities(!caps, Ordering::Relaxed);
                allocation.data = [101, 103, 107, 109];
                std::hint::black_box(&mut allocation.data);
                assert_eq!(read.call(&snapshot), expected);
                let mut fat = FatSnapshot {
                    thin: snapshot,
                    metadata: 4,
                    offset: 2,
                    reserved: 0,
                };
                assert_eq!(read_fat.call(&fat), expected);
                // Inspecting metadata must not access a null/expired pointee.
                snapshot.pointer = std::ptr::null_mut();
                fat.thin.pointer = std::ptr::without_provenance_mut(1);
                assert_eq!(read.call(&snapshot), expected);
                assert_eq!(read_fat.call(&fat), expected);
            }
        }
    }
}

#[test]
fn native_opcode_refuses_addresses_scalars_and_incompatible_structures() {
    let (operation, _, src) = source_operation();
    let context = Context::create();
    let module = context.create_module("incompatible_snapshot_carriers");
    let carriers = [
        context.ptr_type(Default::default()).const_null().into(),
        context.i64_type().const_int(73, false).into(),
        context
            .struct_type(
                &[context.i64_type().into(), context.i64_type().into()],
                false,
            )
            .const_zero()
            .into(),
        context
            .struct_type(
                &[
                    context.ptr_type(Default::default()).into(),
                    context.i32_type().into(),
                    context.i64_type().into(),
                ],
                false,
            )
            .const_zero()
            .into(),
    ];
    for (index, carrier) in carriers.into_iter().enumerate() {
        let name = format!("refused_{index}");
        let function = module.add_function(&name, context.void_type().fn_type(&[], false), None);
        let mut ctx = FunctionContext::new(&context, &module, function, name.as_str());
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        ctx.set_register(src, carrier);
        assert!(
            matches!(lower_instruction(&mut ctx, &operation), Err(LlvmLoweringError::InvalidType(message)) if message.contains("preserved ThinRef or FatRef"))
        );
        ctx.builder().build_return(None).unwrap();
        assert!(!function.print_to_string().to_string().contains("load "));
    }
    module.verify().unwrap();
}

#[test]
fn ordinary_source_reference_cannot_be_reinterpreted_as_native_metadata() {
    use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
    let source =
        Parser::new("fn main() -> UInt32 { let value = 73; @vbc(CBGR_GET_EPOCH_CAPS, &value) }")
            .parse_module()
            .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let mut module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    // Native lowering consumes prepared decoded bodies. Loading the wire alone
    // leaves descriptor.instructions absent and cannot exercise an instruction.
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions = decode_instructions(&module.bytecode[start..end]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    assert!(module.functions.iter().any(|function| function.instructions.as_ref().is_some_and(|body|
        body.iter().any(|instruction| matches!(instruction, Instruction::CbgrExtended { sub_op, .. } if *sub_op == CbgrSubOpcode::GetEpochCaps as u8)))));
    let context = Context::create();
    let mut lower = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("no_snapshot").with_debug_info(false),
    );
    let result = lower.lower_module(&module);
    assert!(
        matches!(&result, Err(LlvmLoweringError::InvalidType(message)) if message.contains("preserved ThinRef or FatRef")),
        "ordinary source reference must retain its exact refusal: {result:?}"
    );
}

#[test]
fn malformed_snapshot_operands_cannot_alias_an_existing_register() {
    let context = Context::create();
    let module = context.create_module("malformed_snapshot_operands");
    for (index, operands) in [
        &[][..],
        &[0][..],
        &[0x80, 0][..],
        &[0, 0x80][..],
        &[0, 1, 2][..],
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("malformed_{index}");
        let function = module.add_function(&name, context.void_type().fn_type(&[], false), None);
        let mut ctx = FunctionContext::new(&context, &module, function, name.as_str());
        ctx.builder()
            .position_at_end(context.append_basic_block(function, "entry"));
        let snapshot = CbgrLowering::new(&context).thin_ref_type().const_zero();
        ctx.set_register(0, snapshot.into());
        ctx.set_register(1, snapshot.into());
        let operation = Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::GetEpochCaps as u8,
            operands: operands.into(),
        };
        assert!(
            matches!(
                lower_instruction(&mut ctx, &operation),
                Err(LlvmLoweringError::UnsupportedInstruction(_))
            ),
            "malformed operands {operands:?} must not default to r0"
        );
        ctx.builder().build_return(None).unwrap();
    }
    module.verify().unwrap();
}

#[test]
fn wide_snapshot_registers_keep_their_encoded_identity() {
    use verum_common::List;
    use verum_vbc::{Reg, encoding::encode_reg};
    let context = Context::create();
    let module = context.create_module("wide_snapshot_registers");
    let function = module.add_function("wide", context.i64_type().fn_type(&[], false), None);
    let mut ctx = FunctionContext::new(&context, &module, function, "wide");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    let snapshot = ctx
        .builder()
        .build_insert_value(
            CbgrLowering::new(&context).thin_ref_type().const_zero(),
            context.i32_type().const_int(0xCAFE8001, false),
            2,
            "snapshot",
        )
        .unwrap()
        .into_struct_value();
    ctx.set_register(0, context.i64_type().const_zero().into());
    ctx.set_register(258, snapshot.into());
    let mut operands = List::<u8>::new().to_vec();
    encode_reg(Reg(257), &mut operands);
    encode_reg(Reg(258), &mut operands);
    lower_instruction(
        &mut ctx,
        &Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::GetEpochCaps as u8,
            operands,
        },
    )
    .unwrap();
    let result = ctx.get_register(257).unwrap().into_int_value();
    assert_eq!(result.get_zero_extended_constant(), Some(0xCAFE8001));
    ctx.builder().build_return(Some(&result)).unwrap();
    module.verify().unwrap();
}
