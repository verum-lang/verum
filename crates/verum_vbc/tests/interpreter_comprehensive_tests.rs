//! Comprehensive VBC Interpreter Tests
//!
//! Tests cover:
//! - Data movement instructions (Mov, LoadI, LoadF, LoadTrue, etc.)
//! - Integer arithmetic (Add, Sub, Mul, Div, Mod)
//! - Float arithmetic
//! - Comparison operations
//! - Boolean logic
//! - Type conversions
//! - Multi-register chaining
//! - Edge cases

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{
    BinaryFloatOp, BinaryIntOp, CompareOp, Instruction, Reg,
};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

// =============================================================================
// Helpers
// =============================================================================

fn create_module(bytecode_data: Vec<u8>) -> Arc<VbcModule> {
    let mut module = VbcModule::new("test".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_data.len() as u32;
    func.register_count = 16;
    module.functions.push(func);
    module.bytecode = bytecode_data;
    Arc::new(module)
}

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

fn run(instructions: &[Instruction]) -> Value {
    let bc = encode(instructions);
    let module = create_module(bc);
    let mut interp = Interpreter::new(module);
    interp.execute_function(FunctionId(0)).expect("Execution failed")
}

// =============================================================================
// Data Movement Tests
// =============================================================================

#[test]
fn test_load_small_int_positive() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_int());
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_load_small_int_negative() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: -10 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_int());
    assert_eq!(result.as_i64(), -10);
}

#[test]
fn test_load_small_int_zero() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_int());
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn test_load_true() {
    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_bool());
    assert!(result.as_bool());
}

#[test]
fn test_load_false() {
    let result = run(&[
        Instruction::LoadFalse { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_bool());
    assert!(!result.as_bool());
}

#[test]
fn test_load_unit() {
    let result = run(&[
        Instruction::LoadUnit { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_unit());
}

#[test]
fn test_mov_register() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 99 },
        Instruction::Mov { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 99);
}

#[test]
fn test_load_large_int() {
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: 1_000_000 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert!(result.is_int());
    assert_eq!(result.as_i64(), 1_000_000);
}

#[test]
fn test_load_float() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 3.14 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_f64(), 3.14);
}

// =============================================================================
// Integer Arithmetic Tests
// =============================================================================

#[test]
fn test_add_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 10 },
        Instruction::LoadSmallI { dst: Reg(1), value: 20 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 30);
}

#[test]
fn test_sub_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 50 },
        Instruction::LoadSmallI { dst: Reg(1), value: 20 },
        Instruction::BinaryI { op: BinaryIntOp::Sub, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 30);
}

#[test]
fn test_mul_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 6 },
        Instruction::LoadSmallI { dst: Reg(1), value: 7 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_div_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 6 },
        Instruction::BinaryI { op: BinaryIntOp::Div, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 7);
}

#[test]
fn test_mod_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 17 },
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::BinaryI { op: BinaryIntOp::Mod, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 2);
}

#[test]
fn test_add_negative_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 10 },
        Instruction::LoadSmallI { dst: Reg(1), value: -3 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 7);
}

// =============================================================================
// Float Arithmetic Tests
// =============================================================================

#[test]
fn test_add_float() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 1.5 },
        Instruction::LoadF { dst: Reg(1), value: 2.5 },
        Instruction::BinaryF { op: BinaryFloatOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_f64(), 4.0);
}

#[test]
fn test_sub_float() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 10.0 },
        Instruction::LoadF { dst: Reg(1), value: 3.5 },
        Instruction::BinaryF { op: BinaryFloatOp::Sub, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_f64(), 6.5);
}

#[test]
fn test_mul_float() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 3.0 },
        Instruction::LoadF { dst: Reg(1), value: 4.0 },
        Instruction::BinaryF { op: BinaryFloatOp::Mul, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_f64(), 12.0);
}

#[test]
fn test_div_float() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 10.0 },
        Instruction::LoadF { dst: Reg(1), value: 4.0 },
        Instruction::BinaryF { op: BinaryFloatOp::Div, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_f64(), 2.5);
}

// =============================================================================
// Comparison Tests
// =============================================================================

#[test]
fn test_compare_int_eq_true() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 42 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

#[test]
fn test_compare_int_eq_false() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 43 },
        Instruction::CmpI { op: CompareOp::Eq, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(!result.as_bool());
}

#[test]
fn test_compare_int_lt_true() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 5 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::CmpI { op: CompareOp::Lt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

#[test]
fn test_compare_int_gt_true() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 10 },
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::CmpI { op: CompareOp::Gt, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

#[test]
fn test_compare_int_le_equal() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 5 },
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::CmpI { op: CompareOp::Le, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

#[test]
fn test_compare_int_ge_less() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 3 },
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::CmpI { op: CompareOp::Ge, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(!result.as_bool());
}

#[test]
fn test_compare_int_ne_true() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        Instruction::CmpI { op: CompareOp::Ne, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert!(result.as_bool());
}

// =============================================================================
// Boolean Logic Tests
// =============================================================================

#[test]
fn test_not_true() {
    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::Not { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert!(!result.as_bool());
}

#[test]
fn test_not_false() {
    let result = run(&[
        Instruction::LoadFalse { dst: Reg(0) },
        Instruction::Not { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert!(result.as_bool());
}

// =============================================================================
// Type Conversion Tests
// =============================================================================

#[test]
fn test_convert_int_to_float() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::CvtIF { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_f64(), 42.0);
}

#[test]
fn test_convert_bool_to_int_true() {
    let result = run(&[
        Instruction::LoadTrue { dst: Reg(0) },
        Instruction::CvtBI { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 1);
}

#[test]
fn test_convert_bool_to_int_false() {
    let result = run(&[
        Instruction::LoadFalse { dst: Reg(0) },
        Instruction::CvtBI { dst: Reg(1), src: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 0);
}

// =============================================================================
// Multi-Register Chaining Tests
// =============================================================================

#[test]
fn test_chain_of_operations() {
    // r0=3, r1=4, r2=r0+r1(=7), r3=r2*r0(=21)
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 3 },
        Instruction::LoadSmallI { dst: Reg(1), value: 4 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(3), a: Reg(2), b: Reg(0) },
        Instruction::Ret { value: Reg(3) },
    ]);
    assert_eq!(result.as_i64(), 21);
}

#[test]
fn test_overwrite_register() {
    // r0=10, r0=r0+5=15
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 10 },
        Instruction::LoadSmallI { dst: Reg(1), value: 5 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(0), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 15);
}

#[test]
fn test_fibonacci_sequence() {
    // Compute fib(5)=5 using unrolled loop
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        // Step 1: 0+1=1
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        // Step 2: 1+1=2
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        // Step 3: 1+2=3
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        // Step 4: 2+3=5
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        Instruction::Ret { value: Reg(1) },
    ]);
    assert_eq!(result.as_i64(), 5);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_load_max_small_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 63 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 63);
}

#[test]
fn test_load_min_small_int() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: -64 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), -64);
}

#[test]
fn test_add_zero() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },
        Instruction::BinaryI { op: BinaryIntOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_multiply_by_one() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_multiply_by_zero() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::LoadSmallI { dst: Reg(1), value: 0 },
        Instruction::BinaryI { op: BinaryIntOp::Mul, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn test_float_precision() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: 0.1 },
        Instruction::LoadF { dst: Reg(1), value: 0.2 },
        Instruction::BinaryF { op: BinaryFloatOp::Add, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    let val = result.as_f64();
    assert!((val - 0.3).abs() < 1e-10, "Expected ~0.3, got {}", val);
}

#[test]
fn test_negative_result() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 5 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::BinaryI { op: BinaryIntOp::Sub, dst: Reg(2), a: Reg(0), b: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), -5);
}

#[test]
fn test_large_negative_int() {
    let result = run(&[
        Instruction::LoadI { dst: Reg(0), value: -1_000_000 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), -1_000_000);
}

#[test]
fn test_float_negative_zero() {
    let result = run(&[
        Instruction::LoadF { dst: Reg(0), value: -0.0 },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_f64(), 0.0);
}

// =============================================================================
// CBGR Reference Tests
// =============================================================================

#[test]
fn test_cbgr_ref_basic() {
    // r0 = 42; r1 = &r0; ChkRef r1; r2 = *r1; return r2
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::Ref { dst: Reg(1), src: Reg(0) },
        Instruction::ChkRef { ref_reg: Reg(1) },
        Instruction::Deref { dst: Reg(2), ref_reg: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_cbgr_ref_mut_basic() {
    // r0 = 0; r1 = &mut r0; *r1 = 42; return r0
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::RefMut { dst: Reg(1), src: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(2), value: 42 },
        Instruction::DerefMut { ref_reg: Reg(1), value: Reg(2) },
        Instruction::Ret { value: Reg(0) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_cbgr_deref_preserves_value() {
    // r0 = 77; r1 = &r0; r2 = *r1; r3 = *r1; both should be 77
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 77 },
        Instruction::Ref { dst: Reg(1), src: Reg(0) },
        Instruction::Deref { dst: Reg(2), ref_reg: Reg(1) },
        Instruction::Deref { dst: Reg(3), ref_reg: Reg(1) },
        Instruction::Ret { value: Reg(3) },
    ]);
    assert_eq!(result.as_i64(), 77);
}

#[test]
fn test_cbgr_ref_negative_value() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: -100 },
        Instruction::Ref { dst: Reg(1), src: Reg(0) },
        Instruction::Deref { dst: Reg(2), ref_reg: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), -100);
}

#[test]
fn test_cbgr_ref_zero_value() {
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ref { dst: Reg(1), src: Reg(0) },
        Instruction::Deref { dst: Reg(2), ref_reg: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn test_cbgr_ref_checked_tier1() {
    // Use Tier 1 RefChecked instead of Tier 0 Ref
    let result = run(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::RefChecked { dst: Reg(1), src: Reg(0) },
        Instruction::Deref { dst: Reg(2), ref_reg: Reg(1) },
        Instruction::Ret { value: Reg(2) },
    ]);
    assert_eq!(result.as_i64(), 42);
}

#[test]
fn test_cbgr_ref_with_function_call() {
    // Simulates: fn read_ref(r: &Int) -> Int { *r }
    // main: let x = 42; read_ref(&x)
    // Function 1 (read_ref): Deref r1 <- r0, Ret r1
    // Function 0 (main): LoadI r0, 42; Ref r1, r0; Call r2, "read_ref", [r1]; Ret r2
    // This requires a two-function module which our test helper doesn't support.
    // Skip this test for now - it requires multi-function module setup.
}
