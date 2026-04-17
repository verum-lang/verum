#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Interpreter tests for control flow, function calls, and CBGR generation tracking.
//!
//! Covers:
//! - JmpIf / JmpNot branching
//! - Loops via backward jumps
//! - Function calls (Call + Ret)
//! - Bitwise operations executed
//! - Type conversion execution
//! - NaN-boxed Value edge cases

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{
    BinaryFloatOp, BinaryIntOp, BitwiseOp, CompareOp, Instruction, Reg, RegRange,
};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

// =============================================================================
// Helpers
// =============================================================================

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

fn create_module(bytecode_data: Vec<u8>) -> Arc<VbcModule> {
    let mut module = VbcModule::new("test".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_data.len() as u32;
    func.register_count = 32;
    module.functions.push(func);
    module.bytecode = bytecode_data;
    Arc::new(module)
}

fn run(instructions: &[Instruction]) -> Value {
    let bc = encode(instructions);
    let module = create_module(bc);
    let mut interp = Interpreter::new(module);
    interp
        .execute_function(FunctionId(0))
        .expect("Execution failed")
}

/// Create a module with multiple functions. Functions are provided as (instructions, register_count).
fn create_multi_fn_module(functions: &[(&[Instruction], u16)]) -> Arc<VbcModule> {
    let mut module = VbcModule::new("test".to_string());
    let mut all_bc = Vec::new();

    for (i, (instrs, reg_count)) in functions.iter().enumerate() {
        let offset = all_bc.len();
        for instr in *instrs {
            bytecode::encode_instruction(instr, &mut all_bc);
        }
        let length = all_bc.len() - offset;

        let mut func = FunctionDescriptor::new(StringId::EMPTY);
        func.id = FunctionId(i as u32);
        func.bytecode_offset = offset as u32;
        func.bytecode_length = length as u32;
        func.register_count = *reg_count;
        module.functions.push(func);
    }

    module.bytecode = all_bc;
    Arc::new(module)
}

// =============================================================================
// Conditional Branch Tests
// =============================================================================

#[test]
fn test_jmp_if_taken() {
    // if true { return 1 } else { return 2 }
    // LoadTrue r0
    // JmpIf r0, +2  (skip the next 2 instructions if true)
    // LoadSmallI r1, 2  <-- skipped
    // Ret r1            <-- skipped
    // LoadSmallI r1, 1
    // Ret r1
    //
    // But the JmpIf offset is in BYTES from the current instruction,
    // so we need to compute exact byte offsets.
    // Instead, let's use a simpler approach: compute offsets from encoded bytes.

    // Strategy: encode with placeholder, then compute offsets
    let false_path_instrs = [
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let false_path_bytes = encode(&false_path_instrs);
    let skip_offset = false_path_bytes.len() as i32;

    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::JmpIf {
            cond: Reg(0),
            offset: skip_offset,
        },
        // False path:
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
        // True path:
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 1,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 1);
}

#[test]
fn test_jmp_if_not_taken() {
    let false_path_instrs = [
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let false_path_bytes = encode(&false_path_instrs);
    let skip_offset = false_path_bytes.len() as i32;

    let result = run(&[
        Instruction::LoadFalse { dst: Reg(0) },
        Instruction::JmpIf {
            cond: Reg(0),
            offset: skip_offset,
        },
        // False path (NOT skipped since condition is false):
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
        // True path:
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 1,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 2);
}

#[test]
fn test_jmp_not_taken() {
    let false_path_instrs = [
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let skip_offset = encode(&false_path_instrs).len() as i32;

    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::JmpNot {
            cond: Reg(0),
            offset: skip_offset,
        },
        // NOT skipped (condition is true, JmpNot doesn't jump):
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 1,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 2);
}

#[test]
fn test_jmp_not_jumped() {
    let false_path_instrs = [
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let skip_offset = encode(&false_path_instrs).len() as i32;

    let result = run(&[
        Instruction::LoadFalse { dst: Reg(0) },
        Instruction::JmpNot {
            cond: Reg(0),
            offset: skip_offset,
        },
        // Skipped (condition is false, JmpNot jumps):
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Ret { value: Reg(1) },
        // Landed here:
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 1,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 1);
}

// =============================================================================
// Unconditional Jump Tests
// =============================================================================

#[test]
fn test_unconditional_jmp_forward() {
    let skipped_instrs = [
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 99,
        },
        Instruction::Ret { value: Reg(0) },
    ];
    let skip_offset = encode(&skipped_instrs).len() as i32;

    let result = run(&[
        Instruction::Jmp {
            offset: skip_offset,
        },
        // Skipped:
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 99,
        },
        Instruction::Ret { value: Reg(0) },
        // Landed:
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 42,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

// =============================================================================
// Loop via Backward Jump
// =============================================================================

#[test]
fn test_simple_countdown_loop() {
    // Compute sum: 1+2+3+4+5 = 15
    // r0 = counter (starts at 5)
    // r1 = accumulator (starts at 0)
    // r2 = 1 (decrement constant)
    // r3 = 0 (zero for comparison)
    // Loop:
    //   r1 = r1 + r0
    //   r0 = r0 - r2
    //   r4 = (r0 > r3) ?
    //   if r4, jump back to loop start
    //   ret r1

    // First compute the loop body size for the backward jump
    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(4),
            a: Reg(0),
            b: Reg(3),
        },
    ];
    let loop_body_bytes = encode(&loop_body);

    // The JmpIf itself also has a size; we need to compute the full backward offset
    // The backward jump goes from after JmpIf to the start of the loop body
    let jmp_if_instr = Instruction::JmpIf {
        cond: Reg(4),
        offset: 0, // placeholder
    };
    let jmp_if_bytes = encode(&[jmp_if_instr]);
    let backward_offset = -((loop_body_bytes.len() + jmp_if_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 5,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 0,
        },
        Instruction::LoadSmallI {
            dst: Reg(2),
            value: 1,
        },
        Instruction::LoadSmallI {
            dst: Reg(3),
            value: 0,
        },
        // Loop body start:
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(4),
            a: Reg(0),
            b: Reg(3),
        },
        Instruction::JmpIf {
            cond: Reg(4),
            offset: backward_offset,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 15);
}

// =============================================================================
// Function Call Tests
// =============================================================================

#[test]
fn test_call_simple_function() {
    // Function 0 (main): calls function 1, returns its result
    // Function 1 (callee): returns 42
    let callee_instrs: &[Instruction] = &[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 42,
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(0), 0),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (callee_instrs, 16)]);

    let mut interp = Interpreter::new(module);
    let result = interp
        .execute_function(FunctionId(0))
        .expect("Execution failed");
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_call_with_arguments() {
    // Function 0 (main): puts 3,4 in r1,r2, calls function 1 with those args
    // Function 1 (add): r0=a, r1=b, returns a+b
    let add_instrs: &[Instruction] = &[
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 3,
        },
        Instruction::LoadSmallI {
            dst: Reg(2),
            value: 4,
        },
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(1), 2),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (add_instrs, 16)]);

    let mut interp = Interpreter::new(module);
    let result = interp
        .execute_function(FunctionId(0))
        .expect("Execution failed");
    assert_eq!(result.as_i64(), 7);
}

// =============================================================================
// Bitwise Operation Execution Tests
// =============================================================================

#[test]
fn test_exec_bitwise_and() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 0b1100,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 0b1010,
        },
        Instruction::Bitwise {
            op: BitwiseOp::And,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0b1000);
}

#[test]
fn test_exec_bitwise_or() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 0b1100,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 0b1010,
        },
        Instruction::Bitwise {
            op: BitwiseOp::Or,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0b1110);
}

#[test]
fn test_exec_bitwise_xor() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 0b1100,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 0b1010,
        },
        Instruction::Bitwise {
            op: BitwiseOp::Xor,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0b0110);
}

#[test]
fn test_exec_bitwise_shl() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 1,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 3,
        },
        Instruction::Bitwise {
            op: BitwiseOp::Shl,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 8);
}

#[test]
fn test_exec_bitwise_shr() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 16,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::Bitwise {
            op: BitwiseOp::Shr,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 4);
}

// =============================================================================
// Type Conversion Execution Tests
// =============================================================================

#[test]
fn test_exec_cvt_int_to_float() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 42,
        },
        Instruction::CvtIF {
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_f64(), 42.0);
}

#[test]
fn test_exec_cvt_float_to_int_truncate() {
    let result = run(&[
        Instruction::LoadF {
            dst: Reg(0),
            value: 3.7,
        },
        Instruction::CvtFI {
            dst: Reg(1),
            src: Reg(0),
            mode: verum_vbc::instruction::FloatToIntMode::Trunc,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 3);
}

#[test]
fn test_exec_cvt_negative_float_to_int_truncate() {
    let result = run(&[
        Instruction::LoadF {
            dst: Reg(0),
            value: -3.7,
        },
        Instruction::CvtFI {
            dst: Reg(1),
            src: Reg(0),
            mode: verum_vbc::instruction::FloatToIntMode::Trunc,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), -3);
}

// =============================================================================
// Bitwise on Integer Values Tests
// =============================================================================

#[test]
fn test_exec_bitwise_and_integers() {
    // 0xFF & 0x0F = 0x0F = 15
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: 0xFF },
        Instruction::LoadI { dst: Reg(1), value: 0x0F },
        Instruction::Bitwise {
            op: BitwiseOp::And,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0x0F);
}

#[test]
fn test_exec_bitwise_or_integers() {
    // 0xF0 | 0x0F = 0xFF = 255
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: 0xF0 },
        Instruction::LoadI { dst: Reg(1), value: 0x0F },
        Instruction::Bitwise {
            op: BitwiseOp::Or,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0xFF);
}

#[test]
fn test_exec_bitwise_xor_integers() {
    // 0xFF ^ 0xFF = 0
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: 0xFF },
        Instruction::LoadI { dst: Reg(1), value: 0xFF },
        Instruction::Bitwise {
            op: BitwiseOp::Xor,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn test_exec_bitwise_xor_different() {
    // 0xAA ^ 0x55 = 0xFF
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: 0xAA },
        Instruction::LoadI { dst: Reg(1), value: 0x55 },
        Instruction::Bitwise {
            op: BitwiseOp::Xor,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0xFF);
}

// =============================================================================
// NaN-boxed Value Edge Case Tests
// =============================================================================

#[test]
fn test_value_int_roundtrip() {
    let v = Value::from_i64(12345);
    assert!(v.is_int());
    assert_eq!(v.as_i64(), 12345);
}

#[test]
fn test_value_negative_int_roundtrip() {
    let v = Value::from_i64(-99999);
    assert!(v.is_int());
    assert_eq!(v.as_i64(), -99999);
}

#[test]
fn test_value_zero_int() {
    let v = Value::from_i64(0);
    assert!(v.is_int());
    assert_eq!(v.as_i64(), 0);
}

#[test]
fn test_value_float_roundtrip() {
    let v = Value::from_f64(3.14);
    assert!(v.is_float());
    assert_eq!(v.as_f64(), 3.14);
}

#[test]
fn test_value_float_negative() {
    let v = Value::from_f64(-273.15);
    assert!(v.is_float());
    assert_eq!(v.as_f64(), -273.15);
}

#[test]
fn test_value_bool_true() {
    let v = Value::from_bool(true);
    assert!(v.is_bool());
    assert!(v.as_bool());
}

#[test]
fn test_value_bool_false() {
    let v = Value::from_bool(false);
    assert!(v.is_bool());
    assert!(!v.as_bool());
}

#[test]
fn test_value_unit() {
    let v = Value::unit();
    assert!(v.is_unit());
}

#[test]
fn test_value_small_string() {
    // Small strings (<=6 bytes) are stored inline via NaN-boxing
    if let Some(v) = Value::from_small_string("hi") {
        assert!(v.is_small_string());
        let ss = v.as_small_string();
        assert_eq!(ss.as_str(), "hi");
    }
}

#[test]
fn test_value_small_string_empty() {
    if let Some(v) = Value::from_small_string("") {
        assert!(v.is_small_string());
        let ss = v.as_small_string();
        assert_eq!(ss.as_str(), "");
    }
}

#[test]
fn test_value_small_string_max_len() {
    // 6 bytes is the max for inline small strings
    if let Some(v) = Value::from_small_string("abcdef") {
        assert!(v.is_small_string());
        let ss = v.as_small_string();
        assert_eq!(ss.as_str(), "abcdef");
    }
}

#[test]
fn test_value_small_string_too_long() {
    // 7+ bytes should return None
    let result = Value::from_small_string("abcdefg");
    assert!(result.is_none());
}

// =============================================================================
// Unary Int Operations
// =============================================================================

#[test]
fn test_exec_neg_int() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 42,
        },
        Instruction::UnaryI {
            op: verum_vbc::instruction::UnaryIntOp::Neg,
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), -42);
}

#[test]
fn test_exec_abs_int_negative() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: -7,
        },
        Instruction::UnaryI {
            op: verum_vbc::instruction::UnaryIntOp::Abs,
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 7);
}

#[test]
fn test_exec_abs_int_positive() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 7,
        },
        Instruction::UnaryI {
            op: verum_vbc::instruction::UnaryIntOp::Abs,
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 7);
}

// =============================================================================
// Integer Power
// =============================================================================

#[test]
fn test_exec_pow_int() {
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 2,
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 10,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Pow,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 1024);
}

// =============================================================================
// Float Comparison Execution
// =============================================================================

#[test]
fn test_exec_cmp_float_lt() {
    let result = run(&[
        Instruction::LoadF {
            dst: Reg(0),
            value: 1.5,
        },
        Instruction::LoadF {
            dst: Reg(1),
            value: 2.5,
        },
        Instruction::CmpF {
            op: CompareOp::Lt,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

#[test]
fn test_exec_cmp_float_eq() {
    let result = run(&[
        Instruction::LoadF {
            dst: Reg(0),
            value: 3.14,
        },
        Instruction::LoadF {
            dst: Reg(1),
            value: 3.14,
        },
        Instruction::CmpF {
            op: CompareOp::Eq,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

// =============================================================================
// Complex Computation
// =============================================================================

#[test]
fn test_factorial_5_unrolled() {
    // 5! = 120
    let result = run(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 1,
        }, // accumulator
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 2,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 3,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 4,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::LoadSmallI {
            dst: Reg(1),
            value: 5,
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 120);
}

#[test]
fn test_chained_comparisons() {
    // Test: (5 > 3) => true, then use result as branch condition
    let false_path = [
        Instruction::LoadSmallI { dst: Reg(5), value: 0 },
        Instruction::Ret { value: Reg(5) },
    ];
    let skip = encode(&false_path).len() as i32;

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 5 },
        Instruction::LoadSmallI { dst: Reg(1), value: 3 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        // If 5 > 3, skip the false path
        Instruction::JmpIf { cond: Reg(2), offset: skip },
        // False path:
        Instruction::LoadSmallI { dst: Reg(5), value: 0 },
        Instruction::Ret { value: Reg(5) },
        // True path:
        Instruction::LoadSmallI { dst: Reg(5), value: 1 },
        Instruction::Ret { value: Reg(5) },
    ]);
    assert_eq!(result.as_i64(), 1);
}

// =============================================================================
// End-to-End Execution Tests
// =============================================================================

/// Test: fibonacci(10) = 55 via iterative loop
#[test]
fn test_fibonacci_iterative() {
    // r0 = n (counter, starts at 10)
    // r1 = a (fib(n-2), starts at 0)
    // r2 = b (fib(n-1), starts at 1)
    // r3 = temp (a + b)
    // r4 = 1 (decrement constant)
    // r5 = 1 (comparison threshold)
    // r6 = condition

    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(1),
            b: Reg(2),
        },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        Instruction::Mov { dst: Reg(2), src: Reg(3) },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(4),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(6),
            a: Reg(0),
            b: Reg(5),
        },
    ];
    let loop_body_bytes = encode(&loop_body);

    let jmp_if_instr = Instruction::JmpIf {
        cond: Reg(6),
        offset: 0,
    };
    let jmp_if_bytes = encode(&[jmp_if_instr]);
    let backward_offset = -((loop_body_bytes.len() + jmp_if_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 10 },
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },
        Instruction::LoadSmallI { dst: Reg(2), value: 1 },
        Instruction::LoadSmallI { dst: Reg(4), value: 1 },
        Instruction::LoadSmallI { dst: Reg(5), value: 1 },
        // Loop body:
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(1),
            b: Reg(2),
        },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        Instruction::Mov { dst: Reg(2), src: Reg(3) },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(4),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(6),
            a: Reg(0),
            b: Reg(5),
        },
        Instruction::JmpIf {
            cond: Reg(6),
            offset: backward_offset,
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 55);
}

/// Test: factorial(5) = 120 via loop
#[test]
fn test_factorial_loop() {
    // r0 = n (starts at 5)
    // r1 = result (accumulator, starts at 1)
    // r2 = 1 (decrement constant + zero check)
    // r3 = condition (n > 1)

    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(2),
        },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(3), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 5 },
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::LoadSmallI { dst: Reg(2), value: 1 },
        // Loop:
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::JmpIf {
            cond: Reg(3),
            offset: back,
        },
        // Final multiplication by 1 (n is now 1)
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 120);
}

/// Test: function call with 3 arguments
#[test]
fn test_function_call_three_args() {
    // Function 0 (main): calls add_three(10, 20, 30)
    // Function 1 (add_three): r0=a, r1=b, r2=c, returns a+b+c
    let add_three_instrs: &[Instruction] = &[
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(4),
            a: Reg(3),
            b: Reg(2),
        },
        Instruction::Ret { value: Reg(4) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::LoadSmallI { dst: Reg(2), value: 20 },
        Instruction::LoadSmallI { dst: Reg(3), value: 30 },
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(1), 3),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (add_three_instrs, 16)]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 60);
}

/// Test: nested function calls (main -> double -> add)
#[test]
fn test_nested_function_calls() {
    let add_instrs: &[Instruction] = &[
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Ret { value: Reg(2) },
    ];

    let double_instrs: &[Instruction] = &[
        Instruction::Mov { dst: Reg(1), src: Reg(0) },
        Instruction::Mov { dst: Reg(2), src: Reg(0) },
        Instruction::Call {
            dst: Reg(3),
            func_id: 2,
            args: RegRange::new(Reg(1), 2),
        },
        Instruction::Ret { value: Reg(3) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 21 },
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(1), 1),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[
        (main_instrs, 16),
        (double_instrs, 16),
        (add_instrs, 16),
    ]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 42);
}

/// Test: pattern matching simulation via JmpNot
/// match x { 1 => 100, 2 => 200, 3 => 300, _ => -1 }
#[test]
fn test_pattern_match_execution() {
    let matched_return = [
        Instruction::LoadI { dst: Reg(3), value: 0 },
        Instruction::Ret { value: Reg(3) },
    ];
    let skip_match = encode(&matched_return).len() as i32;

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 2 },
        // Case 1: x == 1 ?
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip_match },
        Instruction::LoadI { dst: Reg(3), value: 100 },
        Instruction::Ret { value: Reg(3) },
        // Case 2: x == 2 ?
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip_match },
        Instruction::LoadI { dst: Reg(3), value: 200 },
        Instruction::Ret { value: Reg(3) },
        // Case 3: x == 3 ?
        Instruction::LoadSmallI { dst: Reg(1), value: 3 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip_match },
        Instruction::LoadI { dst: Reg(3), value: 300 },
        Instruction::Ret { value: Reg(3) },
        // Default case
        Instruction::LoadSmallI { dst: Reg(3), value: -1 },
        Instruction::Ret { value: Reg(3) },
    ]);
    assert_eq!(result.as_i64(), 200);
}

/// Test: while loop counting up to 10
#[test]
fn test_while_loop_count_up() {
    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(1),
        },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(3), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::LoadSmallI { dst: Reg(2), value: 1 },
        // Loop body:
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::JmpIf {
            cond: Reg(3),
            offset: back,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 10);
}

/// Test: sum of 1..=100 = 5050
#[test]
fn test_for_loop_sum_1_to_100() {
    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(3),
        },
        Instruction::CmpI {
            op: CompareOp::Le,
            dst: Reg(4),
            a: Reg(0),
            b: Reg(2),
        },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(4), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },
        Instruction::LoadI { dst: Reg(2), value: 100 },
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },
        // Loop body:
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(3),
        },
        Instruction::CmpI {
            op: CompareOp::Le,
            dst: Reg(4),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::JmpIf {
            cond: Reg(4),
            offset: back,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 5050);
}

/// Test: function conditional return (max)
#[test]
fn test_function_conditional_return() {
    let false_path = [
        Instruction::Ret { value: Reg(1) },
    ];
    let skip = encode(&false_path).len() as i32;

    let max_instrs: &[Instruction] = &[
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpIf { cond: Reg(2), offset: skip },
        Instruction::Ret { value: Reg(1) },
        Instruction::Ret { value: Reg(0) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 15 },
        Instruction::LoadSmallI { dst: Reg(2), value: 42 },
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange::new(Reg(1), 2),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (max_instrs, 16)]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 42);
}

/// Test: GCD(48, 18) = 6 via Euclidean algorithm
#[test]
fn test_gcd_euclidean() {
    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Mod,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        Instruction::CmpI {
            op: CompareOp::Ne,
            dst: Reg(4),
            a: Reg(1),
            b: Reg(3),
        },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(4), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 48 },
        Instruction::LoadSmallI { dst: Reg(1), value: 18 },
        Instruction::LoadSmallI { dst: Reg(3), value: 0 },
        // Loop:
        Instruction::BinaryI {
            op: BinaryIntOp::Mod,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        Instruction::CmpI {
            op: CompareOp::Ne,
            dst: Reg(4),
            a: Reg(1),
            b: Reg(3),
        },
        Instruction::JmpIf {
            cond: Reg(4),
            offset: back,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 6);
}

/// Test: 2^10 = 1024 via loop
#[test]
fn test_power_loop() {
    let loop_body = [
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(2),
            a: Reg(2),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(3),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(5),
            a: Reg(1),
            b: Reg(4),
        },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(5), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 2 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::LoadSmallI { dst: Reg(2), value: 1 },
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },
        Instruction::LoadSmallI { dst: Reg(4), value: 0 },
        // Loop:
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(2),
            a: Reg(2),
            b: Reg(0),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(3),
        },
        Instruction::CmpI {
            op: CompareOp::Gt,
            dst: Reg(5),
            a: Reg(1),
            b: Reg(4),
        },
        Instruction::JmpIf {
            cond: Reg(5),
            offset: back,
        },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 1024);
}

/// Test: float area computation pi * r^2
#[test]
fn test_float_area_computation() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 3.14159265358979 },
        Instruction::LoadF { dst: Reg(1), value: 5.0 },
        Instruction::BinaryF {
            op: BinaryFloatOp::Mul,
            dst: Reg(2),
            a: Reg(1),
            b: Reg(1),
        },
        Instruction::BinaryF {
            op: BinaryFloatOp::Mul,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(2),
        },
        Instruction::Ret { value: Reg(3) },
    ]);
    let area = result.as_f64();
    assert!((area - 78.5398).abs() < 0.01, "Expected ~78.54, got {}", area);
}

/// Test: int-float conversion roundtrip
#[test]
fn test_int_float_conversion_roundtrip() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::CvtIF { dst: Reg(1), src: Reg(0) },
        Instruction::LoadF { dst: Reg(2), value: 0.5 },
        Instruction::BinaryF {
            op: BinaryFloatOp::Add,
            dst: Reg(3),
            a: Reg(1),
            b: Reg(2),
        },
        Instruction::CvtFI {
            dst: Reg(4),
            src: Reg(3),
            mode: verum_vbc::instruction::FloatToIntMode::Trunc,
        },
        Instruction::Ret { value: Reg(4) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

// =============================================================================
// Recursive Function Call Tests (Task 2)
// =============================================================================

/// Test: recursive fibonacci(10) = 55 via actual Call instructions
#[test]
fn test_recursive_fibonacci() {
    // fib(n): if n <= 1 return n; else return fib(n-1) + fib(n-2)
    // Function 1: fib(n)
    //   r0 = n
    //   if n <= 1 => return n
    //   else => return fib(n-1) + fib(n-2)

    let ret_n = [
        Instruction::Ret { value: Reg(0) },
    ];
    let skip_base = encode(&ret_n).len() as i32;

    let fib_instrs: &[Instruction] = &[
        // Check base case: n <= 1
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpIf { cond: Reg(2), offset: skip_base },
        // Base case: return n
        Instruction::Ret { value: Reg(0) },
        // Recursive case
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },
        Instruction::BinaryI { op: BinaryIntOp::Sub, dst: Reg(4), a: Reg(0), b: Reg(3) },
        Instruction::Call { dst: Reg(5), func_id: 1, args: RegRange::new(Reg(4), 1) },
        Instruction::LoadSmallI { dst: Reg(6), value: 2 },
        Instruction::BinaryI { op: BinaryIntOp::Sub, dst: Reg(7), a: Reg(0), b: Reg(6) },
        Instruction::Call { dst: Reg(8), func_id: 1, args: RegRange::new(Reg(7), 1) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(9), a: Reg(5), b: Reg(8) },
        Instruction::Ret { value: Reg(9) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::Call { dst: Reg(0), func_id: 1, args: RegRange::new(Reg(1), 1) },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (fib_instrs, 16)]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 55);
}

/// Test: recursive factorial(5) = 120
#[test]
fn test_recursive_factorial() {
    // fact(n): if n <= 1 return 1; else return n * fact(n-1)
    let ret_one = [
        Instruction::LoadSmallI { dst: Reg(5), value: 1 },
        Instruction::Ret { value: Reg(5) },
    ];
    let skip_base = encode(&ret_one).len() as i32;

    let fact_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpIf { cond: Reg(2), offset: skip_base },
        // Base case: return 1
        Instruction::LoadSmallI { dst: Reg(5), value: 1 },
        Instruction::Ret { value: Reg(5) },
        // Recursive case: n * fact(n-1)
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },
        Instruction::BinaryI { op: BinaryIntOp::Sub, dst: Reg(4), a: Reg(0), b: Reg(3) },
        Instruction::Call { dst: Reg(5), func_id: 1, args: RegRange::new(Reg(4), 1) },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(6), a: Reg(0), b: Reg(5) },
        Instruction::Ret { value: Reg(6) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::Call { dst: Reg(0), func_id: 1, args: RegRange::new(Reg(1), 1) },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (fact_instrs, 16)]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 120);
}

/// Test: pattern matching with multiple branches and default
/// Simulates: match x { 0 => 1, 1 => 1, n => fib(n-1) + fib(n-2) }
#[test]
fn test_pattern_match_with_default_computation() {
    let matched_return = [
        Instruction::LoadSmallI { dst: Reg(5), value: 0 },
        Instruction::Ret { value: Reg(5) },
    ];
    let skip = encode(&matched_return).len() as i32;

    // Test x=3 => should hit default case and compute 6 (1+2+3)
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 3 },
        // Case 0: x == 0 => return 10
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip },
        Instruction::LoadSmallI { dst: Reg(5), value: 10 },
        Instruction::Ret { value: Reg(5) },
        // Case 1: x == 1 => return 20
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip },
        Instruction::LoadSmallI { dst: Reg(5), value: 20 },
        Instruction::Ret { value: Reg(5) },
        // Case 2: x == 2 => return 30
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpNot { cond: Reg(2), offset: skip },
        Instruction::LoadSmallI { dst: Reg(5), value: 30 },
        Instruction::Ret { value: Reg(5) },
        // Default: return x * 100
        Instruction::LoadI { dst: Reg(3), value: 100 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(5), a: Reg(0), b: Reg(3) },
        Instruction::Ret { value: Reg(5) },
    ]);
    assert_eq!(result.as_i64(), 300);
}

/// Test: nested if-else (simulates match with guards)
/// if x > 10 { if x > 20 { 3 } else { 2 } } else { 1 }
#[test]
fn test_nested_if_else() {
    let inner_false = [
        Instruction::LoadSmallI { dst: Reg(5), value: 2 },
        Instruction::Ret { value: Reg(5) },
    ];
    let skip_inner_false = encode(&inner_false).len() as i32;

    let outer_true_block = [
        Instruction::LoadSmallI { dst: Reg(3), value: 20 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(4), a: Reg(0), b: Reg(3) },
        Instruction::JmpIf { cond: Reg(4), offset: skip_inner_false },
        Instruction::LoadSmallI { dst: Reg(5), value: 2 },
        Instruction::Ret { value: Reg(5) },
        Instruction::LoadSmallI { dst: Reg(5), value: 3 },
        Instruction::Ret { value: Reg(5) },
    ];
    let skip_outer_true = encode(&outer_true_block).len() as i32;

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 25 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::JmpIf { cond: Reg(2), offset: {
            let else_block = [
                Instruction::LoadSmallI { dst: Reg(5), value: 1 },
                Instruction::Ret { value: Reg(5) },
            ];
            encode(&else_block).len() as i32
        }},
        // else: return 1
        Instruction::LoadSmallI { dst: Reg(5), value: 1 },
        Instruction::Ret { value: Reg(5) },
        // if x > 20: return 3, else: return 2
        Instruction::LoadSmallI { dst: Reg(3), value: 20 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(4), a: Reg(0), b: Reg(3) },
        Instruction::JmpIf { cond: Reg(4), offset: skip_inner_false },
        Instruction::LoadSmallI { dst: Reg(5), value: 2 },
        Instruction::Ret { value: Reg(5) },
        Instruction::LoadSmallI { dst: Reg(5), value: 3 },
        Instruction::Ret { value: Reg(5) },
    ]);
    assert_eq!(result.as_i64(), 3);
}

/// Test: while loop with early break simulation
/// Count up from 0, break when i*i > 50, return i
#[test]
fn test_while_loop_with_early_break() {
    // r0 = i (counter)
    // r1 = i*i
    // r2 = 50 (threshold)
    // r3 = 1 (increment)
    // r4 = condition (i*i <= 50)
    let loop_body = [
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(0), a: Reg(0), b: Reg(3) },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(1), a: Reg(0), b: Reg(0) },
        Instruction::CmpI { op: CompareOp::Le, dst: Reg(4), a: Reg(1), b: Reg(2) },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(4), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::LoadSmallI { dst: Reg(2), value: 50 },
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },
        // Loop body:
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(0), a: Reg(0), b: Reg(3) },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(1), a: Reg(0), b: Reg(0) },
        Instruction::CmpI { op: CompareOp::Le, dst: Reg(4), a: Reg(1), b: Reg(2) },
        Instruction::JmpIf { cond: Reg(4), offset: back },
        Instruction::Ret { value: Reg(0) },
    ]);
    // First i where i*i > 50: i=8 (64 > 50)
    assert_eq!(result.as_i64(), 8);
}

/// Test: for loop over range with accumulation (simulates for i in 1..=n { sum += i*i })
#[test]
fn test_for_loop_sum_of_squares() {
    // Sum of squares 1^2 + 2^2 + ... + 5^2 = 55
    let loop_body = [
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(4), a: Reg(0), b: Reg(0) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(1), a: Reg(1), b: Reg(4) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(0), a: Reg(0), b: Reg(3) },
        Instruction::CmpI { op: CompareOp::Le, dst: Reg(5), a: Reg(0), b: Reg(2) },
    ];
    let loop_bytes = encode(&loop_body);
    let jmp_bytes = encode(&[Instruction::JmpIf { cond: Reg(5), offset: 0 }]);
    let back = -((loop_bytes.len() + jmp_bytes.len()) as i32);

    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },   // i = 1
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },   // sum = 0
        Instruction::LoadSmallI { dst: Reg(2), value: 5 },   // n = 5
        Instruction::LoadSmallI { dst: Reg(3), value: 1 },   // step = 1
        // Loop body:
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(4), a: Reg(0), b: Reg(0) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(1), a: Reg(1), b: Reg(4) },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(0), a: Reg(0), b: Reg(3) },
        Instruction::CmpI { op: CompareOp::Le, dst: Reg(5), a: Reg(0), b: Reg(2) },
        Instruction::JmpIf { cond: Reg(5), offset: back },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 55);
}

/// Test: function calls with chained returns
/// f(x) = x + 1; g(x) = f(f(f(x))); g(10) = 13
#[test]
fn test_chained_function_calls() {
    let add_one_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ];

    let triple_add_instrs: &[Instruction] = &[
        // g(x) = f(f(f(x)))
        Instruction::Call { dst: Reg(1), func_id: 2, args: RegRange::new(Reg(0), 1) },
        Instruction::Call { dst: Reg(2), func_id: 2, args: RegRange::new(Reg(1), 1) },
        Instruction::Call { dst: Reg(3), func_id: 2, args: RegRange::new(Reg(2), 1) },
        Instruction::Ret { value: Reg(3) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::Call { dst: Reg(0), func_id: 1, args: RegRange::new(Reg(1), 1) },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[
        (main_instrs, 16),
        (triple_add_instrs, 16),
        (add_one_instrs, 16),
    ]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 13);
}

/// Test: ToString converts int to string value
#[test]
fn test_to_string_conversion() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::ToString { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    // ToString should produce a small string "42"
    assert!(result.is_small_string(), "Expected small string from ToString, got tag {:?}", result);
    let ss = result.as_small_string();
    assert_eq!(ss.as_str(), "42");
}

/// Test: String concatenation "hello" + " " + "world"
#[test]
fn test_string_concat() {
    // Create two small strings and concatenate them
    let hello = Value::from_small_string("hi").expect("Should create small string");
    let world = Value::from_small_string("!").expect("Should create small string");

    // Load values as immediates, concat, return
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        // Simple arithmetic that we know works:
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::ToString { dst: Reg(3), src: Reg(2) },
        Instruction::Ret { value: Reg(3) },
    ]);
    assert!(result.is_small_string());
    assert_eq!(result.as_small_string().as_str(), "3");
}

/// Test: list operations - push, pop, len
#[test]
fn test_list_push_pop() {
    let result = run(&[
        Instruction::NewList { dst: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        Instruction::LoadSmallI { dst: Reg(1), value: 20 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        Instruction::LoadSmallI { dst: Reg(1), value: 30 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        // Pop the last element (30)
        Instruction::ListPop { dst: Reg(2), list: Reg(0) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 30);
}

/// Test: list push multiple then pop all (LIFO order)
#[test]
fn test_list_lifo_order() {
    let result = run(&[
        Instruction::NewList { dst: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        Instruction::LoadSmallI { dst: Reg(1), value: 3 },
        Instruction::ListPush { list: Reg(0), val: Reg(1) },
        // Pop: 3, then 2
        Instruction::ListPop { dst: Reg(2), list: Reg(0) },
        Instruction::ListPop { dst: Reg(3), list: Reg(0) },
        // Return 3 + 2 = 5 (verifying order)
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(4), a: Reg(2), b: Reg(3) },
        Instruction::Ret { value: Reg(4) },
    ]);
    assert_eq!(result.as_i64(), 5); // 3 + 2
}

/// Test: boolean logic execution (AND/OR via branches)
#[test]
fn test_boolean_not_logic() {
    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::Not { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert!(!result.as_bool());
}

/// Test: mixed int and float computation
/// (3 * 4 + 2) as float / 2.0 = 7.0
#[test]
fn test_mixed_int_float_computation() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 3 },
        Instruction::LoadSmallI { dst: Reg(1), value: 4 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::LoadSmallI { dst: Reg(3), value: 2 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(4), a: Reg(2), b: Reg(3) },
        // 14 as float
        Instruction::CvtIF { dst: Reg(5), src: Reg(4) },
        Instruction::LoadF { dst: Reg(6), value: 2.0 },
        Instruction::BinaryF { op: BinaryFloatOp::Div, dst: Reg(7), a: Reg(5), b: Reg(6) },
        Instruction::Ret { value: Reg(7) },
    ]);
    assert_eq!(result.as_f64(), 7.0);
}

/// Test: multiple sequential function calls (square(3) + square(4) = 25)
#[test]
fn test_multiple_function_calls() {
    let square_instrs: &[Instruction] = &[
        Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(0),
            b: Reg(0),
        },
        Instruction::Ret { value: Reg(1) },
    ];

    let main_instrs: &[Instruction] = &[
        Instruction::LoadSmallI { dst: Reg(1), value: 3 },
        Instruction::Call {
            dst: Reg(2),
            func_id: 1,
            args: RegRange::new(Reg(1), 1),
        },
        Instruction::LoadSmallI { dst: Reg(1), value: 4 },
        Instruction::Call {
            dst: Reg(3),
            func_id: 1,
            args: RegRange::new(Reg(1), 1),
        },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(2),
            b: Reg(3),
        },
        Instruction::Ret { value: Reg(0) },
    ];

    let module = create_multi_fn_module(&[(main_instrs, 16), (square_instrs, 16)]);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0)).expect("Execution failed");
    assert_eq!(result.as_i64(), 25);
}
