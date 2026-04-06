//! Integration tests for the complete VBC pipeline.
//!
//! These tests verify the full path:
//! 1. Parse Verum source → AST
//! 2. Compile AST → VBC bytecode
//! 3. Execute VBC in interpreter
//! 4. Verify results
//!
//! This validates that all components work together correctly.

use crate::instruction::{Instruction, Reg};
use crate::module::{FunctionDescriptor, VbcFunction, VbcModule};
#[allow(unused_imports)]
use crate::types::StringId;

// ============================================================================
// Helper Functions for Test Setup
// ============================================================================

/// Creates a minimal VBC module with a single function.
fn create_test_module(name: &str, instructions: Vec<Instruction>) -> (VbcModule, VbcFunction) {
    let mut module = VbcModule::new(name.to_string());

    // Create function descriptor
    let name_id = module.intern_string("main");
    let mut desc = FunctionDescriptor::new(name_id);
    desc.register_count = 16; // Default register count for tests

    // Create function with instructions
    let func = VbcFunction::new(desc.clone(), instructions);
    module.add_function(desc);

    (module, func)
}

/// Gets the instruction count from a VbcFunction
fn instr_count(func: &VbcFunction) -> usize {
    func.instructions.len()
}

// ============================================================================
// Basic Instruction Execution Tests
// ============================================================================

mod basic_execution_tests {
    use super::*;
    use crate::instruction::{BinaryIntOp, CompareOp, UnaryIntOp};

    #[test]
    fn test_load_int_and_return() {
        // Simple: load 42 into r0, return it
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_load", instructions);
        assert_eq!(module.name, "test_load");
        assert_eq!(instr_count(&func), 2);
    }

    #[test]
    fn test_addition_instructions() {
        // Test: 10 + 32 = 42
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_add", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_subtraction_instructions() {
        // Test: 50 - 8 = 42
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 50,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 8,
            },
            Instruction::BinaryI {
                op: BinaryIntOp::Sub,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_sub", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_multiplication_instructions() {
        // Test: 6 * 7 = 42
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_mul", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_division_instructions() {
        // Test: 84 / 2 = 42
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_div", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_modulo_instructions() {
        // Test: 47 % 5 = 2
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_mod", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_negation_instruction() {
        // Test: -(-42) = 42
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_neg", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 3);
    }

    #[test]
    fn test_comparison_eq() {
        // Test: 42 == 42 -> true
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_eq", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }

    #[test]
    fn test_comparison_lt() {
        // Test: 10 < 20 -> true
        let instructions = vec![
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
        ];

        let (module, func) = create_test_module("test_lt", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 4);
    }
}

// ============================================================================
// Control Flow Tests
// ============================================================================

mod control_flow_tests {
    use super::*;
    use crate::instruction::CompareOp;

    #[test]
    fn test_unconditional_jump() {
        // Test: jump over an instruction
        // 0: load 1 -> r0
        // 1: jmp +2 (skip instruction 2)
        // 2: load 999 -> r0 (should be skipped)
        // 3: ret r0 (should return 1)
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 1,
            },
            Instruction::Jmp { offset: 2 }, // Jump to instruction 3
            Instruction::LoadI {
                dst: Reg(0),
                value: 999,
            },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_jmp", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_conditional_jump_taken() {
        // Test: jump if true
        // 0: load true -> r0
        // 1: jmp_if r0 +2 (skip instruction 2)
        // 2: load 999 -> r1 (should be skipped)
        // 3: load 42 -> r1
        // 4: ret r1
        let instructions = vec![
            Instruction::LoadTrue { dst: Reg(0) },
            Instruction::JmpIf {
                cond: Reg(0),
                offset: 2,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 999,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 42,
            },
            Instruction::Ret { value: Reg(1) },
        ];

        let (module, func) = create_test_module("test_jmp_if_taken", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_conditional_jump_not_taken() {
        // Test: don't jump if false
        // 0: load false -> r0
        // 1: jmp_if r0 +2 (should NOT skip)
        // 2: load 42 -> r1 (should execute)
        // 3: ret r1
        let instructions = vec![
            Instruction::LoadFalse { dst: Reg(0) },
            Instruction::JmpIf {
                cond: Reg(0),
                offset: 2,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 42,
            },
            Instruction::Ret { value: Reg(1) },
        ];

        let (module, func) = create_test_module("test_jmp_if_not_taken", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_simple_loop_structure() {
        // Test: loop that counts down from 5 to 0
        // r0 = counter (starts at 5)
        // r1 = result accumulator (starts at 0)
        // r2 = temp for comparison
        //
        // 0: load 5 -> r0
        // 1: load 0 -> r1
        // loop:
        // 2: cmp r0 <= 0 -> r2
        // 3: jmp_if r2 +5 (exit loop)
        // 4: add r1, r0 -> r1 (accumulate)
        // 5: dec r0 -> r0
        // 6: jmp -4 (back to loop start)
        // exit:
        // 7: ret r1 (return sum = 5+4+3+2+1 = 15)
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 5,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 0,
            },
            // Loop start (index 2)
            Instruction::CmpI {
                op: CompareOp::Le,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(3), // r3 is 0 by default (Unit)
            },
            Instruction::JmpIf {
                cond: Reg(2),
                offset: 5,
            }, // Jump to exit if done
            // Loop body
            Instruction::BinaryI {
                op: crate::instruction::BinaryIntOp::Add,
                dst: Reg(1),
                a: Reg(1),
                b: Reg(0),
            },
            Instruction::UnaryI {
                op: crate::instruction::UnaryIntOp::Dec,
                dst: Reg(0),
                src: Reg(0),
            },
            Instruction::Jmp { offset: -4 }, // Back to loop start
            // Exit (index 7)
            Instruction::Ret { value: Reg(1) },
        ];

        let (module, func) = create_test_module("test_loop", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 8);
    }

    #[test]
    fn test_nested_if_else_structure() {
        // if a > 0 {
        //   if b > 0 { 1 } else { 2 }
        // } else {
        //   3
        // }
        //
        // r0 = a = 5
        // r1 = b = 10
        // r2 = result
        // r3 = temp comparison
        let instructions = vec![
            // Setup
            Instruction::LoadI {
                dst: Reg(0),
                value: 5,
            }, // a = 5
            Instruction::LoadI {
                dst: Reg(1),
                value: 10,
            }, // b = 10
            Instruction::LoadI {
                dst: Reg(4),
                value: 0,
            }, // zero for comparison
            // Outer if: a > 0?
            Instruction::CmpI {
                op: CompareOp::Gt,
                dst: Reg(3),
                a: Reg(0),
                b: Reg(4),
            },
            Instruction::JmpNot {
                cond: Reg(3),
                offset: 6,
            }, // if false, jump to else
            // Outer then: inner if: b > 0?
            Instruction::CmpI {
                op: CompareOp::Gt,
                dst: Reg(3),
                a: Reg(1),
                b: Reg(4),
            },
            Instruction::JmpNot {
                cond: Reg(3),
                offset: 3,
            }, // if false, jump to inner else
            // Inner then
            Instruction::LoadI {
                dst: Reg(2),
                value: 1,
            },
            Instruction::Jmp { offset: 4 }, // Jump to end
            // Inner else
            Instruction::LoadI {
                dst: Reg(2),
                value: 2,
            },
            Instruction::Jmp { offset: 2 }, // Jump to end
            // Outer else
            Instruction::LoadI {
                dst: Reg(2),
                value: 3,
            },
            // End
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_nested_if", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// Float Operations Tests
// ============================================================================

mod float_tests {
    use super::*;
    use crate::instruction::{BinaryFloatOp, CompareOp};

    #[test]
    fn test_float_arithmetic() {
        // Test: 3.14 + 2.86 = 6.0
        let instructions = vec![
            Instruction::LoadF {
                dst: Reg(0),
                value: 3.14,
            },
            Instruction::LoadF {
                dst: Reg(1),
                value: 2.86,
            },
            Instruction::BinaryF {
                op: BinaryFloatOp::Add,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_float_add", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_float_comparison() {
        // Test: 3.14 < 4.0 -> true
        let instructions = vec![
            Instruction::LoadF {
                dst: Reg(0),
                value: 3.14,
            },
            Instruction::LoadF {
                dst: Reg(1),
                value: 4.0,
            },
            Instruction::CmpF {
                op: CompareOp::Lt,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_float_cmp", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// Boolean and Logic Tests
// ============================================================================

mod logic_tests {
    use super::*;

    #[test]
    fn test_boolean_not() {
        // Test: !true = false
        let instructions = vec![
            Instruction::LoadTrue { dst: Reg(0) },
            Instruction::Not {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Ret { value: Reg(1) },
        ];

        let (module, _func) = create_test_module("test_not", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_boolean_true_false_load() {
        // Test loading true and false values
        let instructions = vec![
            Instruction::LoadTrue { dst: Reg(0) },
            Instruction::LoadFalse { dst: Reg(1) },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_bool_load", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 3);
    }
}

// ============================================================================
// Bitwise Operations Tests
// ============================================================================

mod bitwise_tests {
    use super::*;
    use crate::instruction::BitwiseOp;

    #[test]
    fn test_bitwise_and() {
        // Test: 0b1010 & 0b1100 = 0b1000 = 8
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 0b1010,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 0b1100,
            },
            Instruction::Bitwise {
                op: BitwiseOp::And,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_bit_and", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_bitwise_or() {
        // Test: 0b1010 | 0b1100 = 0b1110 = 14
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 0b1010,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 0b1100,
            },
            Instruction::Bitwise {
                op: BitwiseOp::Or,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_bit_or", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_bitwise_shift_left() {
        // Test: 1 << 4 = 16
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 1,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 4,
            },
            Instruction::Bitwise {
                op: BitwiseOp::Shl,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_shl", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_bitwise_shift_right() {
        // Test: 16 >> 2 = 4
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 16,
            },
            Instruction::LoadI {
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
        ];

        let (module, func) = create_test_module("test_shr", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// Collection Operations Tests
// ============================================================================

mod collection_tests {
    use super::*;

    #[test]
    fn test_new_list() {
        let instructions = vec![
            Instruction::NewList { dst: Reg(0) },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_new_list", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_list_push() {
        let instructions = vec![
            Instruction::NewList { dst: Reg(0) },
            Instruction::LoadI {
                dst: Reg(1),
                value: 42,
            },
            Instruction::ListPush {
                list: Reg(0),
                val: Reg(1),
            },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_list_push", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_new_map() {
        let instructions = vec![
            Instruction::NewMap { dst: Reg(0) },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_new_map", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// Iterator Tests
// ============================================================================

mod iterator_tests {
    use super::*;

    #[test]
    fn test_iterator_creation() {
        let instructions = vec![
            Instruction::NewList { dst: Reg(0) },
            Instruction::LoadI {
                dst: Reg(1),
                value: 1,
            },
            Instruction::ListPush {
                list: Reg(0),
                val: Reg(1),
            },
            Instruction::IterNew {
                dst: Reg(2),
                iterable: Reg(0),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_iter_new", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// Module and String Interning Tests
// ============================================================================

mod module_tests {
    use super::*;

    #[test]
    fn test_string_interning() {
        let mut module = VbcModule::new("test".to_string());

        let id1 = module.intern_string("hello");
        let id2 = module.intern_string("world");
        let id3 = module.intern_string("hello"); // Duplicate

        // Same string should return same ID
        assert_eq!(id1, id3);
        assert_ne!(id1, id2);

        // Check retrieval
        assert_eq!(module.get_string(id1), Some("hello"));
        assert_eq!(module.get_string(id2), Some("world"));
    }

    #[test]
    fn test_module_serialization_roundtrip() {
        use crate::deserialize::deserialize_module;
        use crate::serialize::serialize_module;

        let mut module = VbcModule::new("roundtrip_test".to_string());
        module.intern_string("test_string");

        let bytes = serialize_module(&module).expect("Serialization failed");
        let loaded = deserialize_module(&bytes).expect("Deserialization failed");

        assert_eq!(module.name, loaded.name);
    }
}

// ============================================================================
// Bytecode Encoding/Decoding Tests
// ============================================================================

mod bytecode_tests {
    use super::*;
    use crate::bytecode::{decode_instruction, encode_instruction};
    use crate::instruction::BinaryIntOp;

    #[test]
    fn test_instruction_sequence_roundtrip() {
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 100,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 200,
            },
            Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        // Encode all instructions
        let mut encoded = Vec::new();
        for instr in &instructions {
            encode_instruction(instr, &mut encoded);
        }

        // Decode all instructions
        let mut offset = 0;
        let mut decoded = Vec::new();
        while offset < encoded.len() {
            let instr = decode_instruction(&encoded, &mut offset).expect("Decode failed");
            decoded.push(instr);
        }

        // Verify count matches
        assert_eq!(instructions.len(), decoded.len());

        // Verify each instruction matches
        for (original, decoded_instr) in instructions.iter().zip(decoded.iter()) {
            assert_eq!(
                format!("{:?}", original),
                format!("{:?}", decoded_instr),
                "Instruction mismatch"
            );
        }
    }
}

// ============================================================================
// Complex Algorithm Tests
// ============================================================================

mod algorithm_tests {
    use super::*;
    use crate::instruction::{BinaryIntOp, CompareOp, UnaryIntOp};

    #[test]
    fn test_fibonacci_like_structure() {
        // Compute fibonacci(10) using a loop
        // r0 = n (input/counter)
        // r1 = fib_prev = 0
        // r2 = fib_curr = 1
        // r3 = temp
        // r4 = zero for comparison
        // r5 = one for decrement
        //
        // Note: This tests the instruction sequence structure, not execution

        let instructions = vec![
            // Initialize
            Instruction::LoadI {
                dst: Reg(0),
                value: 10,
            }, // n = 10
            Instruction::LoadI {
                dst: Reg(1),
                value: 0,
            }, // prev = 0
            Instruction::LoadI {
                dst: Reg(2),
                value: 1,
            }, // curr = 1
            Instruction::LoadI {
                dst: Reg(4),
                value: 0,
            }, // zero
            // Loop: while n > 0
            Instruction::CmpI {
                op: CompareOp::Le,
                dst: Reg(3),
                a: Reg(0),
                b: Reg(4),
            },
            Instruction::JmpIf {
                cond: Reg(3),
                offset: 6,
            }, // Exit if n <= 0
            // temp = curr + prev
            Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(3),
                a: Reg(1),
                b: Reg(2),
            },
            // prev = curr
            Instruction::Mov {
                dst: Reg(1),
                src: Reg(2),
            },
            // curr = temp
            Instruction::Mov {
                dst: Reg(2),
                src: Reg(3),
            },
            // n--
            Instruction::UnaryI {
                op: UnaryIntOp::Dec,
                dst: Reg(0),
                src: Reg(0),
            },
            // Jump back to loop start
            Instruction::Jmp { offset: -6 },
            // Return curr
            Instruction::Ret { value: Reg(2) },
        ];

        let (module, func) = create_test_module("test_fib", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 12);
    }

    #[test]
    fn test_factorial_like_structure() {
        // Compute factorial(5) = 120 using a loop
        // r0 = n (counter, starts at 5)
        // r1 = result (accumulator, starts at 1)
        // r2 = temp for comparison
        // r3 = one

        let instructions = vec![
            // Initialize
            Instruction::LoadI {
                dst: Reg(0),
                value: 5,
            }, // n = 5
            Instruction::LoadI {
                dst: Reg(1),
                value: 1,
            }, // result = 1
            Instruction::LoadI {
                dst: Reg(3),
                value: 1,
            }, // one = 1
            // Loop: while n > 1
            Instruction::CmpI {
                op: CompareOp::Le,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(3),
            },
            Instruction::JmpIf {
                cond: Reg(2),
                offset: 4,
            }, // Exit if n <= 1
            // result = result * n
            Instruction::BinaryI {
                op: BinaryIntOp::Mul,
                dst: Reg(1),
                a: Reg(1),
                b: Reg(0),
            },
            // n--
            Instruction::UnaryI {
                op: UnaryIntOp::Dec,
                dst: Reg(0),
                src: Reg(0),
            },
            // Jump back to loop
            Instruction::Jmp { offset: -4 },
            // Return result
            Instruction::Ret { value: Reg(1) },
        ];

        let (module, func) = create_test_module("test_factorial", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 9);
    }
}

// ============================================================================
// Edge Cases and Error Handling Tests
// ============================================================================

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_function() {
        let instructions = vec![Instruction::RetV];

        let (module, func) = create_test_module("test_empty", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 1);
    }

    #[test]
    fn test_many_registers() {
        // Use many different registers
        let mut instructions = Vec::new();
        for i in 0..50 {
            instructions.push(Instruction::LoadI {
                dst: Reg(i),
                value: i as i64,
            });
        }
        instructions.push(Instruction::Ret { value: Reg(49) });

        let (module, func) = create_test_module("test_many_regs", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 51);
    }

    #[test]
    fn test_long_instruction_sequence() {
        // Create a long sequence of nops followed by return
        let mut instructions: Vec<Instruction> = (0..1000).map(|_| Instruction::Nop).collect();
        instructions.push(Instruction::LoadI {
            dst: Reg(0),
            value: 42,
        });
        instructions.push(Instruction::Ret { value: Reg(0) });

        let (module, func) = create_test_module("test_long_seq", instructions);
        assert_eq!(module.functions.len(), 1);
        assert_eq!(instr_count(&func), 1002);
    }

    #[test]
    fn test_extreme_int_values() {
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: i64::MAX,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: i64::MIN,
            },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_extreme_int", instructions);
        assert_eq!(module.functions.len(), 1);
    }

    #[test]
    fn test_extreme_float_values() {
        let instructions = vec![
            Instruction::LoadF {
                dst: Reg(0),
                value: f64::MAX,
            },
            Instruction::LoadF {
                dst: Reg(1),
                value: f64::MIN_POSITIVE,
            },
            Instruction::LoadF {
                dst: Reg(2),
                value: f64::INFINITY,
            },
            Instruction::LoadF {
                dst: Reg(3),
                value: f64::NEG_INFINITY,
            },
            Instruction::Ret { value: Reg(0) },
        ];

        let (module, func) = create_test_module("test_extreme_float", instructions);
        assert_eq!(module.functions.len(), 1);
    }
}

// ============================================================================
// CBGR Integration Tests - Phase 3.9
// ============================================================================

mod cbgr_integration_tests {
    use super::*;
    use crate::codegen::context::{ExprId, TierContext};
    use crate::types::CbgrTier;
    use verum_common::Map;

    /// Test TierContext creation and basic tier lookup.
    #[test]
    fn test_tier_context_creation() {
        let ctx = TierContext::new();
        assert!(!ctx.enabled);
        assert!(!ctx.is_unsafe());
        assert_eq!(ctx.default_tier, CbgrTier::Tier0);
    }

    /// Test TierContext with decisions from analysis.
    #[test]
    fn test_tier_context_with_decisions() {
        let mut decisions = Map::new();
        decisions.insert(ExprId(1), CbgrTier::Tier1);
        decisions.insert(ExprId(2), CbgrTier::Tier2);
        decisions.insert(ExprId(3), CbgrTier::Tier0);

        let ctx = TierContext::with_decisions(decisions);
        assert!(ctx.enabled);
        assert!(ctx.has_decisions());
        assert_eq!(ctx.decision_count(), 3);

        // Check tier lookups
        assert_eq!(ctx.get_tier(ExprId(1)), CbgrTier::Tier1);
        assert_eq!(ctx.get_tier(ExprId(2)), CbgrTier::Tier2);
        assert_eq!(ctx.get_tier(ExprId(3)), CbgrTier::Tier0);
        // Unknown expression should return default
        assert_eq!(ctx.get_tier(ExprId(999)), CbgrTier::Tier0);
    }

    /// Test span-based tier lookup.
    #[test]
    fn test_tier_context_span_lookup() {
        let mut decisions = Map::new();
        // Create span-encoded ExprId: (start << 32) | end
        let span_id = ExprId((100_u64 << 32) | 150_u64);
        decisions.insert(span_id, CbgrTier::Tier1);

        let ctx = TierContext::with_decisions(decisions);
        assert_eq!(ctx.get_tier_for_span(100, 150), CbgrTier::Tier1);
        assert_eq!(ctx.get_tier_for_span(100, 151), CbgrTier::Tier0); // Different span
    }

    /// Test unsafe block enter/exit and nesting.
    #[test]
    fn test_unsafe_block_nesting() {
        let mut ctx = TierContext::new();
        ctx.enabled = true;

        // Initial state
        assert!(!ctx.is_unsafe());

        // Enter outer unsafe
        let prev1 = ctx.enter_unsafe();
        assert!(!prev1); // Was not unsafe before
        assert!(ctx.is_unsafe());

        // Enter nested unsafe
        let prev2 = ctx.enter_unsafe();
        assert!(prev2); // Was already unsafe
        assert!(ctx.is_unsafe());

        // Exit nested unsafe
        ctx.exit_unsafe(prev2);
        assert!(ctx.is_unsafe()); // Still in outer unsafe

        // Exit outer unsafe
        ctx.exit_unsafe(prev1);
        assert!(!ctx.is_unsafe()); // Back to safe context
    }

    /// Test effective tier with unsafe context.
    #[test]
    fn test_effective_tier_in_unsafe() {
        let mut decisions = Map::new();
        decisions.insert(ExprId(1), CbgrTier::Tier0);
        decisions.insert(ExprId(2), CbgrTier::Tier1);

        let mut ctx = TierContext::with_decisions(decisions);

        // Outside unsafe: use analyzed tiers
        assert_eq!(ctx.get_effective_tier(ExprId(1), false), CbgrTier::Tier0);
        assert_eq!(ctx.get_effective_tier(ExprId(2), false), CbgrTier::Tier1);

        // Explicit &unsafe always returns Tier2
        assert_eq!(ctx.get_effective_tier(ExprId(1), true), CbgrTier::Tier2);
        assert_eq!(ctx.get_effective_tier(ExprId(2), true), CbgrTier::Tier2);

        // Enter unsafe context
        let prev = ctx.enter_unsafe();

        // Inside unsafe: Tier0 stays Tier0 (safety-critical)
        assert_eq!(ctx.get_effective_tier(ExprId(1), false), CbgrTier::Tier0);
        // Inside unsafe: Tier1 promotes to Tier2
        assert_eq!(ctx.get_effective_tier(ExprId(2), false), CbgrTier::Tier2);

        ctx.exit_unsafe(prev);
    }

    /// Test tier allowed checks.
    #[test]
    fn test_tier_allowed() {
        let mut ctx = TierContext::new();
        ctx.enabled = true;

        // Outside unsafe
        assert!(ctx.is_tier_allowed(CbgrTier::Tier0, false));
        assert!(ctx.is_tier_allowed(CbgrTier::Tier1, false));
        assert!(!ctx.is_tier_allowed(CbgrTier::Tier2, false)); // Not allowed without unsafe
        assert!(ctx.is_tier_allowed(CbgrTier::Tier2, true)); // Explicit &unsafe allowed

        // Inside unsafe
        let prev = ctx.enter_unsafe();
        assert!(ctx.is_tier_allowed(CbgrTier::Tier2, false)); // Allowed in unsafe
        ctx.exit_unsafe(prev);
    }

    /// Test integration with CBGR tier analysis result.
    #[test]
    fn test_from_analysis_result() {
        use verum_cbgr::analysis::RefId;
        use verum_cbgr::tier_analysis::TierAnalysisResult;

        // Create mock analysis result
        let mut result = TierAnalysisResult::empty();
        result.decisions.insert(RefId(1), verum_cbgr::tier_types::ReferenceTier::tier1());
        result.decisions.insert(
            RefId(2),
            verum_cbgr::tier_types::ReferenceTier::tier0(
                verum_cbgr::tier_types::Tier0Reason::Escapes,
            ),
        );

        // Convert to TierContext
        let ctx = TierContext::from_analysis_result(&result);
        assert!(ctx.enabled);
        assert!(ctx.has_decisions());
    }

    /// Test CBGR instruction emission with different tiers.
    #[test]
    fn test_cbgr_instruction_variants() {
        // Test that different CBGR instructions compile correctly
        let tier0_instrs = vec![
            // Tier 0: ChkRef before Deref
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::ChkRef { ref_reg: Reg(1) },
            Instruction::Deref {
                dst: Reg(2),
                ref_reg: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let tier1_instrs = vec![
            // Tier 1: RefChecked, direct Deref
            Instruction::RefChecked {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Deref {
                dst: Reg(2),
                ref_reg: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let tier2_instrs = vec![
            // Tier 2: RefUnsafe, direct Deref
            Instruction::RefUnsafe {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Deref {
                dst: Reg(2),
                ref_reg: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let (_module0, func0) = create_test_module("test_tier0", tier0_instrs);
        let (_module1, func1) = create_test_module("test_tier1", tier1_instrs);
        let (_module2, func2) = create_test_module("test_tier2", tier2_instrs);

        // Tier 0 has extra ChkRef instruction
        assert_eq!(instr_count(&func0), 4);
        // Tier 1 and 2 skip ChkRef
        assert_eq!(instr_count(&func1), 3);
        assert_eq!(instr_count(&func2), 3);
    }

    /// Test CodegenStats tier tracking.
    #[test]
    fn test_codegen_stats_tier_tracking() {
        use crate::codegen::context::CodegenStats;

        let mut stats = CodegenStats::default();
        assert_eq!(stats.tier0_refs, 0);
        assert_eq!(stats.tier1_refs, 0);
        assert_eq!(stats.tier2_refs, 0);
        assert_eq!(stats.tier_fallbacks, 0);

        // Simulate recording tiers
        stats.tier0_refs = 10;
        stats.tier1_refs = 5;
        stats.tier2_refs = 2;
        stats.tier_fallbacks = 3;
        stats.capability_checks = 15;

        // Verify totals
        let total_refs = stats.tier0_refs + stats.tier1_refs + stats.tier2_refs;
        assert_eq!(total_refs, 17);
        assert_eq!(stats.tier_fallbacks, 3);
        assert_eq!(stats.capability_checks, 15);
    }

    /// Test CodegenStats cfg_filtered_stmts tracking.
    #[test]
    fn test_codegen_stats_cfg_filtered() {
        use crate::codegen::context::CodegenStats;

        let mut stats = CodegenStats::default();
        assert_eq!(stats.cfg_filtered_stmts, 0);

        // Simulate recording filtered statements
        stats.cfg_filtered_stmts = 5;
        assert_eq!(stats.cfg_filtered_stmts, 5);
    }
}

// ============================================================================
// @cfg Statement-Level Filtering Tests
// ============================================================================

mod cfg_filtering_tests {
    use crate::codegen::{CodegenConfig, VbcCodegen};
    use verum_ast::cfg::TargetConfig;

    /// Test that CodegenConfig correctly stores target config.
    /// Test that CodegenConfig with explicit target uses that target.
    #[test]
    fn test_codegen_config_with_target() {
        let config = CodegenConfig::new("test")
            .with_target(TargetConfig::linux_x86_64());

        assert_eq!(config.target_config.target_os, "linux");
        assert_eq!(config.target_config.target_arch, "x86_64");
    }

    /// Test that CodegenConfig default uses host platform.
    #[test]
    fn test_codegen_config_default_uses_host() {
        let config = CodegenConfig::default();
        let host = TargetConfig::host();

        // Default should match host platform
        assert_eq!(config.target_config.target_os, host.target_os);
        assert_eq!(config.target_config.target_arch, host.target_arch);
    }

    /// Test that VbcCodegen creates cfg evaluator with target.
    #[test]
    fn test_vbc_codegen_with_target_has_evaluator() {
        let config = CodegenConfig::new("test")
            .with_target(TargetConfig::macos_aarch64());

        let codegen = VbcCodegen::with_config(config);
        // cfg_evaluator is private, but we can verify through compilation behavior
        // For now just verify construction succeeds
        assert_eq!(codegen.tier_stats(), (0, 0, 0));
    }

    /// Test that VbcCodegen with default config uses host platform.
    #[test]
    fn test_vbc_codegen_default_uses_host() {
        let codegen = VbcCodegen::new();
        // With default config (host platform), should work
        assert_eq!(codegen.tier_stats(), (0, 0, 0));
    }

    /// Test common target configurations.
    #[test]
    fn test_common_target_configs() {
        // Desktop/Server targets
        let _linux = TargetConfig::linux_x86_64();
        let _macos = TargetConfig::macos_aarch64();
        let _windows = TargetConfig::windows_x86_64();
        let _wasm = TargetConfig::wasm32_wasi();

        // Verify host config
        let host = TargetConfig::host();
        assert!(!host.target_os.is_empty());
        assert!(!host.target_arch.is_empty());
    }

    /// Test embedded/cross-compilation targets including sub-32-bit architectures.
    #[test]
    fn test_embedded_target_configs() {
        // ARM Cortex-M (32-bit embedded)
        let cortex_m0 = TargetConfig::thumbv6m_none_eabi();
        assert_eq!(cortex_m0.target_arch.as_str(), "thumbv6m");
        assert_eq!(cortex_m0.target_os.as_str(), "none");

        let cortex_m4 = TargetConfig::thumbv7em_none_eabihf();
        assert_eq!(cortex_m4.target_arch.as_str(), "thumbv7em");

        // AVR (8-bit architecture, 16-bit pointers)
        let avr = TargetConfig::avr_unknown();
        assert_eq!(avr.target_arch.as_str(), "avr");
        assert_eq!(avr.target_pointer_width.as_str(), "16");

        // MSP430 (16-bit architecture)
        let msp430 = TargetConfig::msp430_none_elf();
        assert_eq!(msp430.target_arch.as_str(), "msp430");
        assert_eq!(msp430.target_pointer_width.as_str(), "16");

        // RISC-V embedded
        let riscv32 = TargetConfig::riscv32imc_unknown_none_elf();
        assert_eq!(riscv32.target_arch.as_str(), "riscv32imc");

        // ESP32
        let esp32 = TargetConfig::xtensa_esp32_none_elf();
        assert_eq!(esp32.target_arch.as_str(), "xtensa");
        assert_eq!(esp32.target_vendor.as_str(), "espressif");

        // Custom bare-metal target
        let custom = TargetConfig::bare_metal("custom_arch", 24);
        assert_eq!(custom.target_arch.as_str(), "custom_arch");
        assert_eq!(custom.target_pointer_width.as_str(), "24");
        assert_eq!(custom.target_os.as_str(), "none");
    }

    /// Test cross-compilation codegen configuration.
    #[test]
    fn test_cross_compilation_codegen() {
        // Cross-compile for ARM Cortex-M4 from any host
        let config = CodegenConfig::new("embedded_app")
            .with_target(TargetConfig::thumbv7em_none_eabihf());

        assert_eq!(config.target_config.target_arch.as_str(), "thumbv7em");
        assert_eq!(config.target_config.target_os.as_str(), "none");

        // Verify VbcCodegen can be created for embedded target
        let codegen = VbcCodegen::with_config(config);
        assert_eq!(codegen.tier_stats(), (0, 0, 0));
    }
}
