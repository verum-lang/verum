//! T1525: native atomic method results retain the declared scalar identity.
use verum_codegen::llvm::context::FunctionContext;
use verum_codegen::llvm::instruction::lower_instruction;
use verum_llvm::OptimizationLevel;
use verum_llvm::context::Context;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::module::VbcModule;

fn atomic_result(
    owner: &str,
    method: &str,
    initial: u64,
    operand: u64,
    qualified: bool,
) -> (u64, u64, bool) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let module = context.create_module("atomic_result");
    let i64_type = context.i64_type();
    let function = module.add_function("probe", i64_type.fn_type(&[i64_type.into()], false), None);
    let mut vbc = VbcModule::new("atomic_result".to_owned());
    let method_name = if qualified {
        format!("{owner}.{method}")
    } else {
        method.to_owned()
    };
    let method_id = vbc.intern_string(&method_name);
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, &vbc, function, "probe");
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_first_param().expect("receiver"));
    ctx.set_obj_register_type(0, owner.to_owned());
    ctx.mark_atomic_int_register(0);
    ctx.set_register(1, i64_type.const_int(operand, false).into());
    ctx.set_register(2, i64_type.const_zero().into());
    lower_instruction(
        &mut ctx,
        &Instruction::CallM {
            dst: Reg(3),
            receiver: Reg(0),
            method_id: method_id.0,
            args: RegRange {
                start: Reg(1),
                count: if method == "load" { 1 } else { 2 },
            },
        },
    )
    .expect("lower atomic method");
    let is_bool = ctx.is_bool_register(3);
    let result = ctx.get_register(3).expect("result").into_int_value();
    let result = ctx
        .builder()
        .build_int_z_extend_or_bit_cast(result, i64_type, "result")
        .expect("ABI result");
    ctx.builder().build_return(Some(&result)).expect("return");
    module.verify().expect("valid native IR");
    let mut object = [0_u64; 4];
    object[3] = initial;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT");
    // SAFETY: probe accepts the address of a live, aligned 32-byte atomic
    // object, reads/writes its value slot, and returns one u64.
    let old = unsafe {
        engine
            .get_function::<unsafe extern "C" fn(u64) -> u64>("probe")
            .expect("probe")
            .call(object.as_mut_ptr() as u64)
    };
    (old, object[3], is_bool)
}

#[test]
fn native_bool_load_and_swap_keep_bool_identity_and_values() {
    for qualified in [false, true] {
        assert_eq!(
            atomic_result("AtomicBool", "load", 0, 0, qualified),
            (0, 0, true)
        );
        assert_eq!(
            atomic_result("AtomicBool", "load", 1, 0, qualified),
            (1, 1, true)
        );
        assert_eq!(
            atomic_result("AtomicBool", "swap", 1, 0, qualified),
            (1, 0, true)
        );
        assert_eq!(
            atomic_result("AtomicBool", "swap", 0, 1, qualified),
            (0, 1, true)
        );
    }
}

#[test]
fn integer_atomic_results_remain_wide_integers() {
    let wide = (1_u64 << 48) + 7;
    for qualified in [false, true] {
        assert_eq!(
            atomic_result("AtomicInt", "load", wide, 0, qualified),
            (wide, wide, false)
        );
        assert_eq!(
            atomic_result("AtomicInt", "swap", wide, 13, qualified),
            (wide, 13, false)
        );
    }
}

#[test]
fn boolean_bitwise_old_values_keep_bool_identity() {
    for (method, initial, operand, after) in [
        ("fetch_and", 1, 0, 0),
        ("fetch_or", 0, 1, 1),
        ("fetch_xor", 1, 1, 0),
    ] {
        assert_eq!(
            atomic_result("AtomicBool", method, initial, operand, true),
            (initial, after, true)
        );
        assert_eq!(
            atomic_result("AtomicInt", method, initial, operand, true),
            (initial, after, false)
        );
    }
}
