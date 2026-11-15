//! Native slot width follows the selected declaration in assembled and wire modules.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, target_settings::TargetSettings};
use verum_llvm::{
    OptimizationLevel, context::Context, module::Linkage, passes::PassBuilderOptions,
};
use verum_vbc::{
    bytecode::{decode_instructions, encode_instruction},
    deserialize::deserialize_module,
    instruction::{Instruction, Reg, RegRange},
    module::{Constant, FunctionDescriptor, FunctionId, VbcModule},
    serialize::serialize_module,
    types::{TypeId, TypeRef},
    validate::validate_module,
};

const WIDE_VALUE: u128 = (1_u128 << 100) | 73;

fn append_function(
    module: &mut VbcModule,
    name: &str,
    result: TypeId,
    base: u32,
    body: &[Instruction],
) {
    let mut function = FunctionDescriptor::new(module.intern_string(name));
    function.id = FunctionId(module.functions.len() as u32);
    function.return_type = TypeRef::Concrete(result);
    function.register_count = 1;
    function.func_id_base = base;
    function.bytecode_offset = module.bytecode.len() as u32;
    for instruction in body {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
    module.add_function(function);
}

fn assembled_call(base: u32, operand: u32) -> VbcModule {
    let mut module = VbcModule::new("native_call_width".into());
    module.constants.push(Constant::Int128 {
        raw: WIDE_VALUE,
        signed: false,
    });
    append_function(
        &mut module,
        "probe",
        TypeId::U128,
        base,
        &[
            Instruction::Call {
                dst: Reg(0),
                func_id: operand,
                args: RegRange {
                    start: Reg(0),
                    count: 0,
                },
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    // A valid raw declaration must not displace an available rebased one.
    append_function(
        &mut module,
        "narrow_decoy",
        TypeId::INT,
        0,
        &[
            Instruction::LoadI {
                dst: Reg(0),
                value: 7,
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    append_function(
        &mut module,
        "wide_target",
        TypeId::U128,
        0,
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::Ret { value: Reg(0) },
        ],
    );
    // func_id_base belongs to the assembled module, not the wire format.
    assert_eq!(module.functions[0].func_id_base, base);
    decode_bodies(module, operand)
}

fn decode_bodies(mut module: VbcModule, operand: u32) -> VbcModule {
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        function.instructions = Some(decode_instructions(&module.bytecode[start..end]).unwrap());
    }
    assert!(matches!(
        &module.functions[0].instructions.as_ref().unwrap()[0],
        Instruction::Call { func_id, .. } if *func_id == operand
    ));
    module
}

fn execute_native(module: &VbcModule) -> u128 {
    let settings = TargetSettings::resolve(None, None, None).unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("call_width")
            .with_debug_info(false)
            .with_target(settings.triple.clone()),
    );
    lowering.lower_module(module).unwrap();
    let native = lowering.into_module();
    native.verify().unwrap();
    let probe = native.get_function("probe").unwrap();
    assert!(
        probe.count_basic_blocks() > 0,
        "actual generated probe body"
    );
    probe.set_linkage(Linkage::External);
    assert_eq!(probe.count_params(), 0);
    assert_eq!(
        probe.get_type().get_return_type(),
        Some(context.i128_type().into())
    );
    for function in native.get_functions() {
        if function.count_basic_blocks() > 0 && function != probe {
            function.set_linkage(Linkage::Internal);
        }
    }
    native
        .run_passes(
            "globaldce",
            &settings.machine().unwrap(),
            PassBuilderOptions::create(),
        )
        .unwrap();
    native.verify().unwrap();
    assert!(
        native
            .get_function("probe")
            .is_some_and(|function| function.count_basic_blocks() > 0),
        "globaldce must preserve the actual entry and its selected call graph"
    );
    let engine = native
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: the complete verified native signature is () -> i128; the engine
    // retains the actual selected callee and remains alive throughout the call.
    unsafe {
        engine
            .get_function::<unsafe extern "C" fn() -> u128>("probe")
            .unwrap()
            .call()
    }
}

#[test]
fn rebased_target_controls_call_slot_width_before_raw_fallback() {
    // Operand1 + base1 selects id2 (wide), while raw id1 is a narrow decoy.
    assert_eq!(execute_native(&assembled_call(1, 1)), WIDE_VALUE);
}

#[test]
fn absent_rebased_target_uses_the_raw_global_declarations_width() {
    // Operand2 + base100 is absent; lower_call selects the actual global id2.
    assert_eq!(execute_native(&assembled_call(100, 2)), WIDE_VALUE);
}

#[test]
fn canonical_wire_target_keeps_the_full_width_call_result() {
    // Wire calls use canonical IDs. No assembly base or declaration is injected
    // after decoding; hydration below only decodes the stored instruction bytes.
    let module = assembled_call(0, 2);
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    validate_module(&wire).expect("complete valid wire module");
    assert!(
        wire.functions
            .iter()
            .all(|function| function.func_id_base == 0)
    );
    assert_eq!(execute_native(&decode_bodies(wire, 2)), WIDE_VALUE);
}
