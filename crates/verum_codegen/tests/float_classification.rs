//! Classification interprets the opcode's precision, independently of its f64 carrier.
use std::sync::Arc;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::{
    bytecode::{decode_instructions, encode_instruction},
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    encoding::encode_reg,
    instruction::{Instruction, MathSubOpcode, Reg},
    interpreter::Interpreter,
    module::{FunctionDescriptor, ParamDescriptor, VbcModule},
    serialize::serialize_module,
    types::{TypeId, TypeRef},
    validate::validate_module,
    value::Value,
};

fn probe(op: MathSubOpcode, wide: bool, preserve: bool) -> VbcModule {
    let mut module = VbcModule::new("float_classification".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.params.push(ParamDescriptor {
        type_ref: TypeRef::Concrete(TypeId::FLOAT),
        ..Default::default()
    });
    function.return_type = TypeRef::Concrete(if preserve {
        TypeId::FLOAT
    } else {
        TypeId::BOOL
    });
    let (src, dst) = if wide {
        (Reg(257), Reg(270))
    } else {
        (Reg(0), Reg(1))
    };
    function.register_count = dst.0 + 1;
    let mut operands = List::new().into();
    encode_reg(dst, &mut operands);
    encode_reg(src, &mut operands);
    let mut body = List::new();
    if wide {
        body.push(Instruction::Mov {
            dst: src,
            src: Reg(0),
        });
    }
    body.push(Instruction::MathExtended {
        sub_op: op.to_byte(),
        operands,
    });
    body.push(Instruction::Ret {
        value: if preserve { src } else { dst },
    });
    for instruction in &body {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32;
    module.add_function(function);
    let mut wire =
        deserialize_module(&serialize_module(&module).expect("serialize")).expect("reload");
    validate_module(&wire).expect("valid VBC carrier and opcode");
    wire.functions[0].instructions = Some(decode_instructions(&wire.bytecode).expect("decode"));
    wire
}

fn values() -> [f64; 27] {
    // Exact ties-to-even midpoint between f32::MAX and 2^128.
    const F32_OVERFLOW_THRESHOLD: f64 = 3.4028235677973366e38;
    [
        0.0,
        -0.0,
        1.0,
        -1.0,
        f64::from(f32::MAX),
        -f64::from(f32::MAX),
        F32_OVERFLOW_THRESHOLD.next_down(),
        F32_OVERFLOW_THRESHOLD,
        F32_OVERFLOW_THRESHOLD.next_up(),
        -F32_OVERFLOW_THRESHOLD.next_down(),
        -F32_OVERFLOW_THRESHOLD,
        -F32_OVERFLOW_THRESHOLD.next_up(),
        f64::from_bits(0x7ff0_0000_0000_0001),
        f64::from_bits(0xfff8_0000_0000_0000),
        1e-50,
        1e300,
        -1e300,
        f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::from_bits(0xfff8_0000_0000_0001),
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::from(f32::MIN_POSITIVE),
        f64::from(f32::from_bits(1)),
        -f64::from(f32::from_bits(1)),
    ]
}

fn with_native(
    module: &VbcModule,
    check: impl FnOnce(&verum_llvm::execution_engine::ExecutionEngine),
) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("classification").with_debug_info(false),
    );
    lowering.lower_module(module).expect("lower real VBC");
    lowering.module().verify().expect("complete valid IR");
    // These scalar functions have no runtime dependencies. Retain their exact
    // emitted bodies and LLVM intrinsic declarations; no replacement definitions.
    let mut ir = Text::new();
    for line in lowering
        .module()
        .print_to_string()
        .to_str()
        .unwrap()
        .lines()
    {
        if line.starts_with("target ") || line.starts_with("attributes #") {
            ir.push_str(line);
            ir.push('\n');
        }
    }
    for function in lowering.module().get_functions() {
        let name = function.get_name().to_str().unwrap();
        if name == "probe" || name.starts_with("llvm.") {
            ir.push_str(function.print_to_string().to_str().unwrap());
            ir.push('\n');
        }
    }
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            ir.as_bytes(),
            "classification",
        ))
        .expect("exact scalar IR");
    executable.verify().expect("valid scalar IR");
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    check(&engine);
}

fn check(op: MathSubOpcode, wide: bool, expected: impl Fn(f64) -> bool) {
    let module = probe(op, wide, false);
    let id = module.find_function_by_name("probe").unwrap();
    let mut interpreter = Interpreter::new(Arc::new(module.clone()));
    with_native(&module, |engine| {
        // SAFETY: the explicit descriptor has one Float (f64) argument and Bool result.
        let native =
            unsafe { engine.get_function::<unsafe extern "C" fn(f64) -> bool>("probe") }.unwrap();
        for value in values() {
            let oracle = expected(value);
            let vm = interpreter
                .execute_function_with_args(id, &[Value::from_f64(value)])
                .unwrap();
            assert_eq!(vm.as_bool(), oracle, "VM {op:?} {value:?}");
            assert_eq!(
                unsafe { native.call(value) },
                oracle,
                "native {op:?} {value:?}"
            );
        }
    });
}

#[test]
fn f32_infinity_uses_operand_precision() {
    check(MathSubOpcode::IsInfF32, false, |x| (x as f32).is_infinite());
}
#[test]
fn f32_finiteness_uses_operand_precision() {
    check(MathSubOpcode::IsFiniteF32, false, |x| {
        (x as f32).is_finite()
    });
}
#[test]
fn f32_nan_uses_operand_precision() {
    check(MathSubOpcode::IsNanF32, false, |x| (x as f32).is_nan());
}
#[test]
fn f64_classification_preserves_full_range() {
    check(MathSubOpcode::IsInfF64, false, f64::is_infinite);
    check(MathSubOpcode::IsFiniteF64, false, f64::is_finite);
    check(MathSubOpcode::IsNanF64, false, f64::is_nan);
}
#[test]
fn classification_decodes_both_wide_registers() {
    check(MathSubOpcode::IsInfF32, true, |x| (x as f32).is_infinite());
    check(MathSubOpcode::IsFiniteF64, true, f64::is_finite);
}
#[test]
fn classification_never_rounds_its_source_register_in_place() {
    for op in [
        MathSubOpcode::IsInfF32,
        MathSubOpcode::IsFiniteF32,
        MathSubOpcode::IsNanF32,
    ] {
        let module = probe(op, false, true);
        let id = module.find_function_by_name("probe").unwrap();
        let mut interpreter = Interpreter::new(Arc::new(module.clone()));
        with_native(&module, |engine| {
            // SAFETY: the explicit descriptor returns its f64 argument after classification.
            let native =
                unsafe { engine.get_function::<unsafe extern "C" fn(f64) -> f64>("probe") }
                    .unwrap();
            for value in values().into_iter().filter(|x| !x.is_nan()) {
                let vm = interpreter
                    .execute_function_with_args(id, &[Value::from_f64(value)])
                    .unwrap();
                assert_eq!(vm.as_f64().to_bits(), value.to_bits());
                assert_eq!(unsafe { native.call(value) }.to_bits(), value.to_bits());
            }
        });
    }
}
#[test]
fn source_float32_producers_retain_classification_width() {
    for (name, op) in [
        ("is_nan_f32", MathSubOpcode::IsNanF32),
        ("is_infinite_f32", MathSubOpcode::IsInfF32),
        ("is_finite_f32", MathSubOpcode::IsFiniteF32),
    ] {
        let source = format!("fn probe(x: Float32) -> Bool {{ @intrinsic(\"{name}\", x) }}");
        let ast = Parser::new(&source)
            .parse_module()
            .expect("canonical grammar");
        let module = VbcCodegen::new()
            .compile_module(&ast)
            .expect("actual source producer");
        let id = module.find_function_by_name("probe").unwrap();
        let function = &module.functions[id.0 as usize];
        assert_eq!(function.params[0].type_ref, TypeRef::Concrete(TypeId::F32));
        let body = function.instructions.as_ref().expect("source instructions");
        assert!(
            body.iter().any(
                |i| matches!(i, Instruction::MathExtended { sub_op, .. } if *sub_op == op.to_byte())
            ),
            "{name}: {body:?}"
        );
        let mut interpreter = Interpreter::new(Arc::new(module.clone()));
        with_native(&module, |engine| {
            // SAFETY: the checked producer descriptor declares Float32 -> Bool.
            let native =
                unsafe { engine.get_function::<unsafe extern "C" fn(f32) -> bool>("probe") }
                    .unwrap();
            for x in values().map(|x| x as f32) {
                let expected = match op {
                    MathSubOpcode::IsNanF32 => x.is_nan(),
                    MathSubOpcode::IsInfF32 => x.is_infinite(),
                    MathSubOpcode::IsFiniteF32 => x.is_finite(),
                    _ => unreachable!(),
                };
                let vm = interpreter
                    .execute_function_with_args(id, &[Value::from_f64(f64::from(x))])
                    .unwrap();
                assert_eq!(vm.as_bool(), expected, "source VM {name} {x:?}");
                assert_eq!(
                    unsafe { native.call(x) },
                    expected,
                    "source native {name} {x:?}"
                );
            }
        });
    }
}

#[test]
fn classification_rejects_malformed_operands_and_unproven_carriers() {
    use verum_codegen::llvm::{context::FunctionContext, instruction::lower_instruction};
    let context = Context::create();
    let module = context.create_module("classification_refusal");
    let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
    let vbc = VbcModule::new("classification_refusal".into());
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, context.f64_type().const_float(1.0).into());
    for bytes in [&[][..], &[1], &[1, 0x80], &[1, 0, 7]] {
        assert!(
            lower_instruction(
                &mut ctx,
                &Instruction::MathExtended {
                    sub_op: MathSubOpcode::IsFiniteF32.to_byte(),
                    operands: bytes.to_vec(),
                }
            )
            .is_err(),
            "malformed {bytes:?}"
        );
    }
    for carrier in [
        context.ptr_type(Default::default()).const_null().into(),
        context.i32_type().const_zero().into(),
        context.struct_type(&[], false).const_zero().into(),
    ] {
        ctx.set_register(0, carrier);
        let error = lower_instruction(
            &mut ctx,
            &Instruction::MathExtended {
                sub_op: MathSubOpcode::IsFiniteF32.to_byte(),
                operands: [1, 0].to_vec(),
            },
        )
        .expect_err("no fabricated scalar from an unproven carrier");
        assert!(
            error.to_string().contains("floating-point value"),
            "{error}"
        );
    }
}
