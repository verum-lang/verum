//! VBC Execution Tests - End-to-end tests that actually run VBC bytecode.
//!

//! These tests:
//! 1. Create VBC instructions programmatically
//! 2. Encode them to bytecode
//! 3. Execute in the VBC interpreter
//! 4. Verify results
//!

//! This validates that the full VBC pipeline works correctly.

#![cfg(test)]

use crate::bytecode::encode_instruction;
use crate::instruction::{BinaryIntOp, CompareOp, Instruction, Reg, UnaryIntOp};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::module::{FunctionDescriptor, VbcModule};
#[allow(unused_imports)]
use crate::types::StringId;
use crate::value::Value;

use std::sync::Arc;

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a VBC module with a single function that can be executed.
fn create_executable_module(
    name: &str,
    instructions: Vec<Instruction>,
    register_count: u16,
) -> Arc<VbcModule> {
    let mut module = VbcModule::new(name.to_string());

    // Encode instructions to bytecode
    let mut bytecode = Vec::new();
    for instr in &instructions {
        encode_instruction(instr, &mut bytecode);
    }

    // Store bytecode in module
    let bytecode_offset = module.bytecode.len() as u32;
    let bytecode_length = bytecode.len() as u32;
    module.bytecode.extend(bytecode);

    // Create function descriptor
    let name_id = module.intern_string("main");
    let mut desc = FunctionDescriptor::new(name_id);
    desc.register_count = register_count;
    desc.bytecode_offset = bytecode_offset;
    desc.bytecode_length = bytecode_length;

    module.add_function(desc);

    Arc::new(module)
}

/// Executes a module and returns the result.
fn execute_module(module: Arc<VbcModule>) -> Result<Value, InterpreterError> {
    let mut interp = Interpreter::new(module);
    interp.run_main()
}

/// Helper to assert integer result.
fn assert_int_result(module: Arc<VbcModule>, expected: i64) {
    let result = execute_module(module).expect("Execution failed");
    match result.try_as_i64() {
        Some(actual) => assert_eq!(actual, expected, "Expected {}, got {}", expected, actual),
        None => panic!("Expected Int, got {:?}", result),
    }
}

/// Helper to assert boolean result.
fn assert_bool_result(module: Arc<VbcModule>, expected: bool) {
    let result = execute_module(module).expect("Execution failed");
    match result.try_as_bool() {
        Some(actual) => assert_eq!(actual, expected, "Expected {}, got {}", expected, actual),
        None => panic!("Expected Bool, got {:?}", result),
    }
}

/// Helper to assert unit result.
fn assert_unit_result(module: Arc<VbcModule>) {
    let result = execute_module(module).expect("Execution failed");
    assert!(result.is_unit(), "Expected Unit, got {:?}", result);
}

/// Calculates the byte size of an instruction when encoded.
fn instruction_byte_size(instr: &Instruction) -> i32 {
    let mut buf = Vec::new();
    encode_instruction(instr, &mut buf);
    buf.len() as i32
}

// ============================================================================
// Basic Arithmetic Execution Tests
// ============================================================================

mod arithmetic_execution_tests {
    use super::*;

    #[test]
    fn test_execute_load_int() {
        let module = create_executable_module(
            "load_int",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 42,
                },
                Instruction::Ret { value: Reg(0) },
            ],
            1,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_addition() {
        let module = create_executable_module(
            "addition",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 10,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 32,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_subtraction() {
        let module = create_executable_module(
            "subtraction",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 100,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 58,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Sub,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_multiplication() {
        let module = create_executable_module(
            "multiplication",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 6,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 7,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Mul,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_division() {
        let module = create_executable_module(
            "division",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 84,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 2,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Div,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_modulo() {
        let module = create_executable_module(
            "modulo",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 47,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 5,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Mod,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 2);
    }

    #[test]
    fn test_execute_negation() {
        let module = create_executable_module(
            "negation",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: -42,
                },
                Instruction::UnaryI {
                    op: UnaryIntOp::Neg,
                    dst: Reg(1),
                    src: Reg(0),
                },
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_complex_expression() {
        // (10 + 20) * 2 - 18 = 42
        let module = create_executable_module(
            "complex",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 10,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 20,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                }, // r2 = 30
                Instruction::LoadI {
                    dst: Reg(3),
                    value: 2,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Mul,
                    dst: Reg(4),
                    a: Reg(2),
                    b: Reg(3),
                }, // r4 = 60
                Instruction::LoadI {
                    dst: Reg(5),
                    value: 18,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Sub,
                    dst: Reg(6),
                    a: Reg(4),
                    b: Reg(5),
                }, // r6 = 42
                Instruction::Ret { value: Reg(6) },
            ],
            7,
        );

        assert_int_result(module, 42);
    }
}

// ============================================================================
// Comparison Execution Tests
// ============================================================================

mod comparison_execution_tests {
    use super::*;

    #[test]
    fn test_execute_eq_true() {
        let module = create_executable_module(
            "eq_true",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 42,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 42,
                },
                Instruction::CmpI {
                    op: CompareOp::Eq,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_bool_result(module, true);
    }

    #[test]
    fn test_execute_eq_false() {
        let module = create_executable_module(
            "eq_false",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 42,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 43,
                },
                Instruction::CmpI {
                    op: CompareOp::Eq,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_bool_result(module, false);
    }

    #[test]
    fn test_execute_lt_true() {
        let module = create_executable_module(
            "lt_true",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 10,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 20,
                },
                Instruction::CmpI {
                    op: CompareOp::Lt,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_bool_result(module, true);
    }

    #[test]
    fn test_execute_gt_true() {
        let module = create_executable_module(
            "gt_true",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 20,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 10,
                },
                Instruction::CmpI {
                    op: CompareOp::Gt,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_bool_result(module, true);
    }
}

// ============================================================================
// Control Flow Execution Tests
// ============================================================================

mod control_flow_execution_tests {
    use super::*;

    #[test]
    fn test_execute_unconditional_jump() {
        // Jump over the "wrong" value
        // Calculate byte size of the instruction to skip
        let skip_instr = Instruction::LoadI {
            dst: Reg(0),
            value: 999,
        };
        let skip_bytes = instruction_byte_size(&skip_instr);

        let module = create_executable_module(
            "jmp",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 42,
                },
                Instruction::Jmp { offset: skip_bytes }, // Skip next instruction
                skip_instr.clone(),                      // Should be skipped
                Instruction::Ret { value: Reg(0) },
            ],
            1,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_conditional_jump_taken() {
        // Jump when condition is true
        // Calculate byte sizes
        let skip_instr = Instruction::LoadI {
            dst: Reg(1),
            value: 999,
        };
        let skip_bytes = instruction_byte_size(&skip_instr);

        let module = create_executable_module(
            "jmp_if_taken",
            vec![
                Instruction::LoadTrue { dst: Reg(0) },
                Instruction::JmpIf {
                    cond: Reg(0),
                    offset: skip_bytes,
                },
                skip_instr.clone(), // Should be skipped
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 42,
                },
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_conditional_jump_not_taken() {
        // Don't jump when condition is false - use valid offset even though not taken
        let skip_instr = Instruction::LoadI {
            dst: Reg(1),
            value: 42,
        };
        let skip_bytes = instruction_byte_size(&skip_instr);

        let module = create_executable_module(
            "jmp_if_not_taken",
            vec![
                Instruction::LoadFalse { dst: Reg(0) },
                Instruction::JmpIf {
                    cond: Reg(0),
                    offset: skip_bytes, // Won't be taken anyway
                },
                skip_instr.clone(), // Should execute
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_simple_if_else() {
        // if true { 42 } else { 999 }
        // Calculate byte sizes for each instruction
        let then_load = Instruction::LoadI {
            dst: Reg(1),
            value: 42,
        };
        let else_load = Instruction::LoadI {
            dst: Reg(1),
            value: 999,
        };

        // JmpNot needs to skip: then_load + jmp_to_end
        // But we don't know jmp size yet - use placeholder and recalculate
        let jmp_placeholder = Instruction::Jmp { offset: 0 };
        let then_size = instruction_byte_size(&then_load);
        let jmp_size = instruction_byte_size(&jmp_placeholder);
        let else_size = instruction_byte_size(&else_load);

        let module = create_executable_module(
            "if_else",
            vec![
                Instruction::LoadTrue { dst: Reg(0) }, // condition
                Instruction::JmpNot {
                    cond: Reg(0),
                    offset: then_size + jmp_size,
                }, // if false, goto else
                // Then branch
                then_load,
                Instruction::Jmp { offset: else_size }, // goto end (skip else)
                // Else branch
                else_load,
                // End
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_int_result(module, 42);
    }
}

// ============================================================================
// Loop Execution Tests
// ============================================================================

mod loop_execution_tests {
    use super::*;

    #[test]
    fn test_execute_countdown_loop() {
        // Sum of 1 to 5 = 15
        // r0 = counter (5 to 0)
        // r1 = sum
        // r2 = temp comparison
        // r3 = zero constant

        // First, define all the instructions to calculate their sizes
        let init1 = Instruction::LoadI {
            dst: Reg(0),
            value: 5,
        };
        let init2 = Instruction::LoadI {
            dst: Reg(1),
            value: 0,
        };
        let init3 = Instruction::LoadI {
            dst: Reg(3),
            value: 0,
        };
        let cmp = Instruction::CmpI {
            op: CompareOp::Le,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(3),
        };
        // JmpIf will be created with calculated offset
        let body_add = Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        };
        let body_dec = Instruction::UnaryI {
            op: UnaryIntOp::Dec,
            dst: Reg(0),
            src: Reg(0),
        };
        // Back jump will be created with calculated offset
        let ret = Instruction::Ret { value: Reg(1) };

        // Calculate sizes (init/ret sizes only documented — not consumed
        // directly; the body sizes below drive the forward/back jump
        // offsets).
        let _init1_size = instruction_byte_size(&init1);
        let _init2_size = instruction_byte_size(&init2);
        let _init3_size = instruction_byte_size(&init3);
        let cmp_size = instruction_byte_size(&cmp);
        let body_add_size = instruction_byte_size(&body_add);
        let body_dec_size = instruction_byte_size(&body_dec);
        let _ret_size = instruction_byte_size(&ret);

        // JmpIf needs to skip: body_add + body_dec + back_jmp to reach ret
        // Back jump placeholder to estimate size
        let back_jmp_placeholder = Instruction::Jmp { offset: 0 };
        let back_jmp_size = instruction_byte_size(&back_jmp_placeholder);

        let forward_jmp_offset = body_add_size + body_dec_size + back_jmp_size;

        // Back jump needs to go backwards to cmp instruction
        // From after back_jmp to start of cmp: -(body_add + body_dec + back_jmp + jmpif + cmp)
        let jmpif_placeholder = Instruction::JmpIf {
            cond: Reg(2),
            offset: 0,
        };
        let jmpif_size = instruction_byte_size(&jmpif_placeholder);
        let back_offset = -(cmp_size + jmpif_size + body_add_size + body_dec_size + back_jmp_size);

        let module = create_executable_module(
            "countdown",
            vec![
                init1,
                init2,
                init3,
                cmp.clone(),
                Instruction::JmpIf {
                    cond: Reg(2),
                    offset: forward_jmp_offset,
                },
                body_add,
                body_dec,
                Instruction::Jmp {
                    offset: back_offset,
                },
                ret,
            ],
            4,
        );

        assert_int_result(module, 15); // 5 + 4 + 3 + 2 + 1 = 15
    }

    #[test]
    fn test_execute_factorial() {
        // factorial(5) = 120
        // r0 = n (5 to 1)
        // r1 = result
        // r2 = temp comparison
        // r3 = one constant

        let init1 = Instruction::LoadI {
            dst: Reg(0),
            value: 5,
        };
        let init2 = Instruction::LoadI {
            dst: Reg(1),
            value: 1,
        };
        let init3 = Instruction::LoadI {
            dst: Reg(3),
            value: 1,
        };
        let cmp = Instruction::CmpI {
            op: CompareOp::Lt,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(3),
        };
        let body_mul = Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(1),
            a: Reg(1),
            b: Reg(0),
        };
        let body_dec = Instruction::UnaryI {
            op: UnaryIntOp::Dec,
            dst: Reg(0),
            src: Reg(0),
        };
        let ret = Instruction::Ret { value: Reg(1) };

        let cmp_size = instruction_byte_size(&cmp);
        let body_mul_size = instruction_byte_size(&body_mul);
        let body_dec_size = instruction_byte_size(&body_dec);

        let back_jmp_placeholder = Instruction::Jmp { offset: 0 };
        let back_jmp_size = instruction_byte_size(&back_jmp_placeholder);
        let jmpif_placeholder = Instruction::JmpIf {
            cond: Reg(2),
            offset: 0,
        };
        let jmpif_size = instruction_byte_size(&jmpif_placeholder);

        let forward_jmp_offset = body_mul_size + body_dec_size + back_jmp_size;
        let back_offset = -(cmp_size + jmpif_size + body_mul_size + body_dec_size + back_jmp_size);

        let module = create_executable_module(
            "factorial",
            vec![
                init1,
                init2,
                init3,
                cmp.clone(),
                Instruction::JmpIf {
                    cond: Reg(2),
                    offset: forward_jmp_offset,
                },
                body_mul,
                body_dec,
                Instruction::Jmp {
                    offset: back_offset,
                },
                ret,
            ],
            4,
        );

        assert_int_result(module, 120); // 5! = 120
    }

    #[test]
    fn test_execute_fibonacci() {
        // fibonacci(10) = 55 (using 0-indexed: fib(0)=0, fib(1)=1, ...)
        // r0 = n (counter, starts at 10)
        // r1 = fib_prev (starts at 0)
        // r2 = fib_curr (starts at 1)
        // r3 = temp (for swapping)
        // r4 = comparison result
        // r5 = zero constant

        let init_n = Instruction::LoadI {
            dst: Reg(0),
            value: 10,
        };
        let init_prev = Instruction::LoadI {
            dst: Reg(1),
            value: 0,
        };
        let init_curr = Instruction::LoadI {
            dst: Reg(2),
            value: 1,
        };
        let init_zero = Instruction::LoadI {
            dst: Reg(5),
            value: 0,
        };
        let cmp = Instruction::CmpI {
            op: CompareOp::Le,
            dst: Reg(4),
            a: Reg(0),
            b: Reg(5),
        };
        // body: temp = prev + curr, prev = curr, curr = temp, n--
        let body_add = Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(1),
            b: Reg(2),
        };
        let body_mov1 = Instruction::Mov {
            dst: Reg(1),
            src: Reg(2),
        };
        let body_mov2 = Instruction::Mov {
            dst: Reg(2),
            src: Reg(3),
        };
        let body_dec = Instruction::UnaryI {
            op: UnaryIntOp::Dec,
            dst: Reg(0),
            src: Reg(0),
        };
        let ret = Instruction::Ret { value: Reg(1) };

        let cmp_size = instruction_byte_size(&cmp);
        let body_add_size = instruction_byte_size(&body_add);
        let body_mov1_size = instruction_byte_size(&body_mov1);
        let body_mov2_size = instruction_byte_size(&body_mov2);
        let body_dec_size = instruction_byte_size(&body_dec);

        let back_jmp_placeholder = Instruction::Jmp { offset: 0 };
        let back_jmp_size = instruction_byte_size(&back_jmp_placeholder);
        let jmpif_placeholder = Instruction::JmpIf {
            cond: Reg(4),
            offset: 0,
        };
        let jmpif_size = instruction_byte_size(&jmpif_placeholder);

        let body_size = body_add_size + body_mov1_size + body_mov2_size + body_dec_size;
        let forward_jmp_offset = body_size + back_jmp_size;
        let back_offset = -(cmp_size + jmpif_size + body_size + back_jmp_size);

        let module = create_executable_module(
            "fibonacci",
            vec![
                init_n,
                init_prev,
                init_curr,
                init_zero,
                cmp.clone(),
                Instruction::JmpIf {
                    cond: Reg(4),
                    offset: forward_jmp_offset,
                },
                body_add,
                body_mov1,
                body_mov2,
                body_dec,
                Instruction::Jmp {
                    offset: back_offset,
                },
                ret,
            ],
            6,
        );

        assert_int_result(module, 55); // fib(10) = 55
    }
}

// ============================================================================
// Boolean Logic Execution Tests
// ============================================================================

mod boolean_execution_tests {
    use super::*;

    #[test]
    fn test_execute_not_true() {
        let module = create_executable_module(
            "not_true",
            vec![
                Instruction::LoadTrue { dst: Reg(0) },
                Instruction::Not {
                    dst: Reg(1),
                    src: Reg(0),
                },
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_bool_result(module, false);
    }

    #[test]
    fn test_execute_not_false() {
        let module = create_executable_module(
            "not_false",
            vec![
                Instruction::LoadFalse { dst: Reg(0) },
                Instruction::Not {
                    dst: Reg(1),
                    src: Reg(0),
                },
                Instruction::Ret { value: Reg(1) },
            ],
            2,
        );

        assert_bool_result(module, true);
    }

    #[test]
    fn test_execute_double_negation() {
        let module = create_executable_module(
            "double_not",
            vec![
                Instruction::LoadTrue { dst: Reg(0) },
                Instruction::Not {
                    dst: Reg(1),
                    src: Reg(0),
                },
                Instruction::Not {
                    dst: Reg(2),
                    src: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_bool_result(module, true);
    }
}

// ============================================================================
// Return Value Tests
// ============================================================================

mod return_tests {
    use super::*;

    #[test]
    fn test_execute_return_unit() {
        let module = create_executable_module("ret_unit", vec![Instruction::RetV], 0);

        assert_unit_result(module);
    }

    #[test]
    fn test_execute_return_early() {
        let module = create_executable_module(
            "ret_early",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 42,
                },
                Instruction::Ret { value: Reg(0) }, // Early return
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 999,
                }, // Never executed
                Instruction::Ret { value: Reg(0) },
            ],
            1,
        );

        assert_int_result(module, 42);
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

mod edge_case_execution_tests {
    use super::*;

    #[test]
    fn test_execute_zero_operations() {
        let module = create_executable_module(
            "zero_ops",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 0,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 42,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                }, // 0 + 42 = 42
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_negative_numbers() {
        let module = create_executable_module(
            "negative",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: -10,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 52,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                }, // -10 + 52 = 42
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 42);
    }

    #[test]
    fn test_execute_large_numbers() {
        let module = create_executable_module(
            "large",
            vec![
                Instruction::LoadI {
                    dst: Reg(0),
                    value: 1_000_000_000,
                },
                Instruction::LoadI {
                    dst: Reg(1),
                    value: 2_000_000_000,
                },
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ],
            3,
        );

        assert_int_result(module, 3_000_000_000);
    }

    #[test]
    fn test_execute_many_operations() {
        // Chain of operations: 1 + 1 + 1 + ... + 1 = 42
        let mut instructions = vec![Instruction::LoadI {
            dst: Reg(0),
            value: 0,
        }];

        for _ in 0..42 {
            instructions.push(Instruction::LoadI {
                dst: Reg(1),
                value: 1,
            });
            instructions.push(Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(0),
                a: Reg(0),
                b: Reg(1),
            });
        }
        instructions.push(Instruction::Ret { value: Reg(0) });

        let module = create_executable_module("many_ops", instructions, 2);

        assert_int_result(module, 42);
    }
}

// ============================================================================
// Try Operator Tag Tests
//
// These tests pin the runtime behavior of IsVar against the canonical variant
// tags from MAYBE_VARIANT_LAYOUT and RESULT_VARIANT_LAYOUT.
//
// compile_try emits:
//   MakeVariant { tag: X, field_count }   -- build the variant
//   IsVar { dst, value, tag: success_tag } -- check for success variant
//
// For Maybe: success_tag = maybe_success_tag() = 1  (Some)
// For Result: success_tag = result_success_tag() = 0 (Ok)
//
// Each test creates a minimal program that returns 1 (success) or 0 (failure)
// depending on whether IsVar fires, verifying the interpreter honors the tag.
// ============================================================================

mod try_operator_tag_tests {
    use super::*;
    use verum_common::well_known_types::{maybe_success_tag, result_success_tag};

    /// Helper: build a program that:
    ///   r0 = MakeVariant { tag: variant_tag, field_count: 0 }
    ///   r1 = IsVar(r0, check_tag)
    ///   Ret r1   -- returns Bool (1 if tags match, 0 otherwise)
    fn is_var_result(variant_tag: u32, check_tag: u32) -> bool {
        let instructions = vec![
            Instruction::MakeVariant {
                dst: Reg(0),
                tag: variant_tag,
                field_count: 0,
            },
            Instruction::IsVar {
                dst: Reg(1),
                value: Reg(0),
                tag: check_tag,
            },
            Instruction::Ret { value: Reg(1) },
        ];
        let module = create_executable_module("is_var_test", instructions, 2);
        execute_module(module)
            .expect("execution failed")
            .try_as_bool()
            .expect("expected Bool result")
    }

    /// `IsVar { tag: maybe_success_tag() }` must be true for `Some` (tag 1).
    #[test]
    fn maybe_success_tag_detects_some() {
        let some_tag = 1u32; // MAYBE_VARIANT_LAYOUT: Some = 1
        assert!(
            is_var_result(some_tag, maybe_success_tag()),
            "IsVar(tag={}) must be true when variant has tag {} (Some)",
            maybe_success_tag(),
            some_tag,
        );
    }

    /// `IsVar { tag: maybe_success_tag() }` must be false for `None` (tag 0).
    /// This was the critical bug: success_tag.unwrap_or(0) tested for None, not Some.
    #[test]
    fn maybe_success_tag_rejects_none() {
        let none_tag = 0u32; // MAYBE_VARIANT_LAYOUT: None = 0
        assert!(
            !is_var_result(none_tag, maybe_success_tag()),
            "IsVar(tag={}) must be false when variant has tag {} (None) — \
             success tag must be Some ({}), not None ({})",
            maybe_success_tag(),
            none_tag,
            maybe_success_tag(),
            none_tag,
        );
    }

    /// `IsVar { tag: result_success_tag() }` must be true for `Ok` (tag 0).
    #[test]
    fn result_success_tag_detects_ok() {
        let ok_tag = 0u32; // RESULT_VARIANT_LAYOUT: Ok = 0
        assert!(
            is_var_result(ok_tag, result_success_tag()),
            "IsVar(tag={}) must be true when variant has tag {} (Ok)",
            result_success_tag(),
            ok_tag,
        );
    }

    /// `IsVar { tag: result_success_tag() }` must be false for `Err` (tag 1).
    #[test]
    fn result_success_tag_rejects_err() {
        let err_tag = 1u32; // RESULT_VARIANT_LAYOUT: Err = 1
        assert!(
            !is_var_result(err_tag, result_success_tag()),
            "IsVar(tag={}) must be false when variant has tag {} (Err)",
            result_success_tag(),
            err_tag,
        );
    }

    /// Cross-check: maybe_success_tag != result_failure_tag.
    /// Ensures the two canonical tags don't accidentally coincide in a way
    /// that would mask the Maybe-tag bug.
    #[test]
    fn maybe_and_result_success_tags_are_correct() {
        assert_eq!(maybe_success_tag(), 1, "Some must be tag 1");
        assert_eq!(result_success_tag(), 0, "Ok must be tag 0");
        assert_ne!(
            maybe_success_tag(),
            result_success_tag(),
            "Maybe and Result have different success tag positions by design",
        );
    }
}

// ============================================================================
// FN-LOCAL-STATIC-ONCE-1 (task #16) — fn-local `static` hoist + once-init
// ============================================================================
//
// A `static` declared inside a fn body is hoisted at declaration-collection
// time to a module-level synthetic cell `<fn>$static$<name>` wired through
// the same `__tls_init_*` / global-ctor machinery as module-level
// `static mut`.  These tests compile REAL Verum source through the full
// parse → VBC codegen → interpreter pipeline and pin:
//
//   1. once-init: the initializer runs exactly once (at ctor time), so a
//      counter static advances 1, 2, 3 across calls instead of resetting;
//   2. per-declaration cell identity: two fns each declaring `static mut N`
//      get DISTINCT cells (pre-fix both registered the bare name "N" and
//      silently shared one slot);
//   3. scope-first shadowing: a fn-local static shadows a module-level
//      static of the same name inside its declaring fn only;
//   4. the hoist is structural: the module carries a
//      `__tls_init_<fn>$static$<name>` synthetic registered as a global
//      ctor (the same path Tier-1 AOT lowers to `@llvm.global_ctors`).

mod fn_local_static_once_tests {
    use super::*;
    use crate::codegen::{CodegenConfig, VbcCodegen};

    fn compile_source(source: &str) -> Arc<VbcModule> {
        let file_id = verum_ast::FileId::new(0);
        let lexer = verum_lexer::Lexer::new(source, file_id);
        let parser = verum_fast_parser::VerumParser::new();
        let ast = parser
            .parse_module(lexer, file_id)
            .unwrap_or_else(|e| panic!("parse failed: {:?}", e));
        // Module name "main" mirrors the production single-file user run:
        // any other name promotes descriptor names to `<module>.<fn>`
        // qualified form and `find_function_by_name("main")` misses.
        let config = CodegenConfig::new("main");
        let mut codegen = VbcCodegen::with_config(config);
        Arc::new(codegen.compile_module(&ast).expect("codegen failed"))
    }

    /// Runs global ctors (TLS inits) then the named function, mirroring
    /// the production `run_main` sequencing.
    fn run_named(module: Arc<VbcModule>, name: &str) -> Value {
        let fid = module.find_function_by_name(name).unwrap_or_else(|| {
            let names: Vec<&str> = module
                .functions
                .iter()
                .filter_map(|d| module.get_string(d.name))
                .collect();
            panic!(
                "function '{}' not found in module; functions present: {:?}",
                name, names
            )
        });
        let mut interp = Interpreter::new(module);
        interp.run_global_ctors().expect("global ctors failed");
        interp.execute_function(fid).expect("execution failed")
    }

    fn run_named_int(source: &str, name: &str) -> i64 {
        let result = run_named(compile_source(source), name);
        result
            .try_as_i64()
            .unwrap_or_else(|| panic!("expected Int result, got {:?}", result))
    }

    /// The task-#16 repro probe: a fn-local `static mut` counter must
    /// advance 1, 2, 3 across consecutive calls.  Broken once-init
    /// (per-call re-initialization) yields 1, 1, 1.
    #[test]
    fn fn_local_static_mut_counter_advances_across_calls() {
        let source = r#"
fn counter() -> Int {
    static mut N: Int = 0;
    unsafe { N = N + 1; N }
}
fn main() -> Int {
    let a = counter();
    let b = counter();
    let c = counter();
    a * 100 + b * 10 + c
}
"#;
        assert_eq!(run_named_int(source, "main"), 123);
    }

    /// Two fns each declaring `static mut N` must get DISTINCT hoisted
    /// cells.  Pre-fix both registered bare "N": one shared slot, and
    /// the last `__tls_init_N` won the initial value (f: 101, g: 111,
    /// f: 112 → 324 instead of 1 + 110 + 2 = 113).
    #[test]
    fn fn_local_statics_do_not_collide_across_fns() {
        let source = r#"
fn f() -> Int {
    static mut N: Int = 0;
    unsafe { N = N + 1; N }
}
fn g() -> Int {
    static mut N: Int = 100;
    unsafe { N = N + 10; N }
}
fn main() -> Int {
    let a = f();
    let b = g();
    let c = f();
    a + b + c
}
"#;
        assert_eq!(run_named_int(source, "main"), 113);
    }

    /// A NON-mut fn-local static must also be once-init (ctor-time), not
    /// a re-evaluated constant-function.  This is the recovery.vr:900
    /// `static SEED: AtomicU64 = AtomicU64.new(...)` defect class in its
    /// minimal form; the structural pin below covers the hoist itself.
    #[test]
    fn fn_local_plain_static_reads_hoisted_cell() {
        let source = r#"
fn base() -> Int {
    static K: Int = 41;
    K + 1
}
fn main() -> Int { base() }
"#;
        assert_eq!(run_named_int(source, "main"), 42);
    }

    /// Structural pin: the hoist emits a `__tls_init_<fn>$static$<name>`
    /// synthetic function AND registers it in `module.global_ctors` —
    /// the exact channel Tier-1 AOT lowers to `@llvm.global_ctors`, so
    /// both tiers share ONE init authority.
    #[test]
    fn fn_local_static_hoist_registers_tls_init_global_ctor() {
        let source = r#"
fn counter() -> Int {
    static mut N: Int = 0;
    unsafe { N = N + 1; N }
}
fn main() -> Int { counter() }
"#;
        let module = compile_source(source);
        let init_fid = module
            .find_function_by_name("__tls_init_counter$static$N")
            .expect("hoisted fn-local static must emit __tls_init_counter$static$N");
        assert!(
            module.global_ctors.iter().any(|(fid, _prio)| *fid == init_fid),
            "__tls_init_counter$static$N must be registered as a global ctor; \
             global_ctors = {:?}",
            module.global_ctors,
        );
    }

    /// Scope-first shadowing: inside `f`, `N` is the fn-local hoisted
    /// cell; inside `main`, `N` is the module-level static.  The two
    /// must never alias.
    #[test]
    fn fn_local_static_shadows_module_static_in_declaring_fn_only() {
        let source = r#"
static mut N: Int = 1000;
fn f() -> Int {
    static mut N: Int = 0;
    unsafe { N = N + 1; N }
}
fn main() -> Int {
    let a = f();
    let b = f();
    unsafe { a + b + N }
}
"#;
        assert_eq!(run_named_int(source, "main"), 1003);
    }
}
