//! Generator Integration Tests
//!
//! Comprehensive tests for the VBC interpreter generator state machine.
//!
//! Generator state machine: fn* functions compile to a state enum with Yield/Return transitions.
//! Each yield point becomes a state; local variables are saved/restored on suspend/resume.
//! Generators implement the Iterator protocol (has_next/next) for for-in loop integration.
//!
//! These tests verify:
//! - Generator creation and lifecycle
//! - Yield suspension and resumption
//! - State preservation across yields
//! - Multiple independent generators
//! - Error handling and edge cases
//! - Memory safety during generator execution

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{BinaryIntOp, Instruction, Opcode, Reg, RegRange};
use verum_vbc::interpreter::{GeneratorId, Interpreter, InterpreterError};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

// =============================================================================
// Test Helpers
// =============================================================================

/// Creates a test module with the given bytecode.
fn create_module(bytecode: Vec<u8>) -> Arc<VbcModule> {
    let mut module = VbcModule::new("generator_test".to_string());

    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode.len() as u32;
    func.register_count = 16;

    module.functions.push(func);
    module.bytecode = bytecode;

    Arc::new(module)
}

/// Encodes instructions into bytecode.
fn encode_instructions(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

// =============================================================================
// Basic Generator Tests
// =============================================================================

#[test]
fn test_generator_creation() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Create generator - should succeed
    let gen_id = interp.create_generator(FunctionId(0));
    assert!(gen_id.is_ok());
}

#[test]
fn test_generator_creation_invalid_function() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Try to create generator from non-existent function
    let result = interp.create_generator(FunctionId(999));
    assert!(matches!(
        result,
        Err(InterpreterError::FunctionNotFound(_))
    ));
}

#[test]
fn test_generator_resume_invalid_id() {
    let bytecode = vec![Opcode::Ret as u8, 0];
    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Try to resume non-existent generator
    let result = interp.resume_generator(GeneratorId(12345));
    assert!(matches!(
        result,
        Err(InterpreterError::InvalidGeneratorId { .. })
    ));
}

// =============================================================================
// Yield and Resume Tests
// =============================================================================

#[test]
fn test_single_yield() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // First resume yields 42
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, Some(Value::from_i64(42)));

    // Second resume completes
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_multiple_yields() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 2 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 3 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // Yields 1, 2, 3, then completes
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(1))
    );
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(2))
    );
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(3))
    );
    assert_eq!(interp.resume_generator(gen_id).unwrap(), None);
}

#[test]
fn test_immediate_return() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 99 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // First resume completes immediately (no yield)
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, None);
}

// =============================================================================
// State Preservation Tests
// =============================================================================

#[test]
fn test_register_preservation() {
    // Generator that accumulates: r0 = 0; r0 += 10; yield; r0 += 20; yield
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::LoadSmallI { dst: Reg(1), value: 10 },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Yield { value: Reg(0) }, // yields 10
        Instruction::LoadSmallI { dst: Reg(1), value: 20 },
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(0),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Yield { value: Reg(0) }, // yields 30
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // First yield: 10
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(10))
    );

    // Second yield: 30 (10 + 20, r0 preserved)
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(30))
    );

    // Complete
    assert_eq!(interp.resume_generator(gen_id).unwrap(), None);
}

#[test]
fn test_multiple_registers() {
    // Use multiple registers and verify they're all preserved
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::LoadSmallI { dst: Reg(1), value: 2 },
        Instruction::LoadSmallI { dst: Reg(2), value: 3 },
        Instruction::Yield { value: Reg(0) }, // yields 1

        // All registers should still have their values
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(0),
            b: Reg(1),
        }, // r3 = 1 + 2 = 3
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(3),
            b: Reg(2),
        }, // r3 = 3 + 3 = 6
        Instruction::Yield { value: Reg(3) },  // yields 6
        Instruction::Ret { value: Reg(3) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(1))
    );
    assert_eq!(
        interp.resume_generator(gen_id).unwrap(),
        Some(Value::from_i64(6))
    );
}

// =============================================================================
// Multiple Generator Tests
// =============================================================================

#[test]
fn test_independent_generators() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 2 },
        Instruction::Yield { value: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Create two generators from the same function
    let gen1 = interp.create_generator(FunctionId(0)).unwrap();
    let gen2 = interp.create_generator(FunctionId(0)).unwrap();

    // Interleave resumes - each should maintain independent state
    assert_eq!(
        interp.resume_generator(gen1).unwrap(),
        Some(Value::from_i64(1))
    );
    assert_eq!(
        interp.resume_generator(gen2).unwrap(),
        Some(Value::from_i64(1))
    );
    assert_eq!(
        interp.resume_generator(gen1).unwrap(),
        Some(Value::from_i64(2))
    );
    assert_eq!(interp.resume_generator(gen1).unwrap(), None);
    assert_eq!(
        interp.resume_generator(gen2).unwrap(),
        Some(Value::from_i64(2))
    );
    assert_eq!(interp.resume_generator(gen2).unwrap(), None);
}

#[test]
fn test_many_generators() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 100 },
        Instruction::Yield { value: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Create many generators
    let generators: Vec<_> = (0..50)
        .map(|_| interp.create_generator(FunctionId(0)).unwrap())
        .collect();

    // Resume all, each should yield 100
    for gen_id in &generators {
        let result = interp.resume_generator(*gen_id).unwrap();
        assert_eq!(result, Some(Value::from_i64(100)));
    }

    // Complete all
    for gen_id in &generators {
        let result = interp.resume_generator(*gen_id).unwrap();
        assert_eq!(result, None);
    }
}

// =============================================================================
// Statistics Tests
// =============================================================================

#[test]
fn test_generator_stats() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Yield { value: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    // Initially no generators
    let stats = interp.generator_stats();
    assert_eq!(stats.total, 0);

    // Create generator
    let gen_id = interp.create_generator(FunctionId(0)).unwrap();
    let stats = interp.generator_stats();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.created, 1);

    // Resume (yields)
    let _ = interp.resume_generator(gen_id);
    let stats = interp.generator_stats();
    assert_eq!(stats.yielded, 1);

    // Resume (completes)
    let _ = interp.resume_generator(gen_id);
    let stats = interp.generator_stats();
    assert_eq!(stats.completed, 1);
}

#[test]
fn test_generator_has_next() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 1 },
        Instruction::Yield { value: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // Before first resume - can resume
    assert!(interp.generator_has_next(gen_id));

    // After yield - can resume
    let _ = interp.resume_generator(gen_id);
    assert!(interp.generator_has_next(gen_id));

    // After completion - cannot resume
    let _ = interp.resume_generator(gen_id);
    assert!(!interp.generator_has_next(gen_id));
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_resume_completed_generator() {
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // Complete the generator
    let _ = interp.resume_generator(gen_id);

    // Resuming completed generator should return None
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, None);

    // Should still return None on subsequent resumes
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_many_yields() {
    // Generator with 20 yields
    let mut instructions = Vec::new();
    for i in 0..20i8 {
        instructions.push(Instruction::LoadSmallI { dst: Reg(0), value: i });
        instructions.push(Instruction::Yield { value: Reg(0) });
    }
    instructions.push(Instruction::LoadSmallI { dst: Reg(0), value: 0 });
    instructions.push(Instruction::Ret { value: Reg(0) });

    let bytecode = encode_instructions(&instructions);
    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    // Should yield 0..19
    for expected in 0i64..20 {
        let result = interp.resume_generator(gen_id).unwrap();
        assert_eq!(result, Some(Value::from_i64(expected)));
    }

    // Then complete
    let result = interp.resume_generator(gen_id).unwrap();
    assert_eq!(result, None);
}

// =============================================================================
// Fibonacci Generator Test
// =============================================================================

#[test]
fn test_fibonacci_generator() {
    // Fibonacci: yield 0, 1, 1, 2, 3
    let bytecode = encode_instructions(&[
        // yield 0
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Yield { value: Reg(0) },
        // r1 = 1
        Instruction::LoadSmallI { dst: Reg(1), value: 1 },
        Instruction::Yield { value: Reg(1) }, // yield 1
        // r2 = r0 + r1 = 1
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Yield { value: Reg(2) },
        // r0 = r1, r1 = r2
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        // r2 = r0 + r1 = 2
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Yield { value: Reg(2) },
        Instruction::Mov { dst: Reg(0), src: Reg(1) },
        Instruction::Mov { dst: Reg(1), src: Reg(2) },
        // r2 = r0 + r1 = 3
        Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        },
        Instruction::Yield { value: Reg(2) },
        // return
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);

    let gen_id = interp.create_generator(FunctionId(0)).unwrap();

    let expected = [0i64, 1, 1, 2, 3];
    for exp in expected {
        let result = interp.resume_generator(gen_id).unwrap();
        assert_eq!(result, Some(Value::from_i64(exp)), "Expected {}", exp);
    }

    assert_eq!(interp.resume_generator(gen_id).unwrap(), None);
}

// =============================================================================
// Iterator Protocol Opcode Tests
// =============================================================================
//
// These tests verify the new GenCreate, GenNext, and GenHasNext opcodes that
// implement the Iterator protocol for generators.
//
// Generator-Iterator protocol bridge: generators auto-implement Iterator via state machine

#[test]
fn test_gen_create_opcode() {
    // Test GenCreate opcode creates a generator value
    //
    // Bytecode program:
    //   r0 = gen_create(func_id=1)  // Create generator from function 1
    //   ret r0
    //
    // Function 1:
    //   yield 42
    //   ret 0
    let main_bytecode = encode_instructions(&[
        Instruction::GenCreate { dst: Reg(0), func_id: 1, args: RegRange { start: Reg(0), count: 0 } },
        Instruction::Ret { value: Reg(0) },
    ]);

    let gen_bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let mut module = VbcModule::new("gen_create_test".to_string());

    // Main function (id=0)
    let mut main_func = FunctionDescriptor::new(StringId::EMPTY);
    main_func.id = FunctionId(0);
    main_func.bytecode_offset = 0;
    main_func.bytecode_length = main_bytecode.len() as u32;
    main_func.register_count = 16;
    module.functions.push(main_func);

    // Generator function (id=1)
    let mut gen_func = FunctionDescriptor::new(StringId::EMPTY);
    gen_func.id = FunctionId(1);
    gen_func.bytecode_offset = main_bytecode.len() as u32;
    gen_func.bytecode_length = gen_bytecode.len() as u32;
    gen_func.register_count = 16;
    gen_func.is_generator = true;
    module.functions.push(gen_func);

    let mut combined_bytecode = main_bytecode;
    combined_bytecode.extend(gen_bytecode);
    module.bytecode = combined_bytecode;

    let mut interp = Interpreter::new(Arc::new(module));
    let result = interp.execute_function(FunctionId(0)).unwrap();

    // Result should be a generator value
    assert!(result.is_generator(), "Expected generator value, got {:?}", result);
}

#[test]
fn test_gen_has_next_opcode() {
    // Test GenHasNext opcode checks generator status
    //
    // Bytecode program:
    //   r0 = gen_create(func_id=1)
    //   r1 = gen_has_next(r0)  // Should be true (Created)
    //   ret r1
    let main_bytecode = encode_instructions(&[
        Instruction::GenCreate { dst: Reg(0), func_id: 1, args: RegRange { start: Reg(0), count: 0 } },
        Instruction::GenHasNext { dst: Reg(1), generator: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);

    let gen_bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::Yield { value: Reg(0) },
        Instruction::LoadSmallI { dst: Reg(0), value: 0 },
        Instruction::Ret { value: Reg(0) },
    ]);

    let mut module = VbcModule::new("gen_has_next_test".to_string());

    let mut main_func = FunctionDescriptor::new(StringId::EMPTY);
    main_func.id = FunctionId(0);
    main_func.bytecode_offset = 0;
    main_func.bytecode_length = main_bytecode.len() as u32;
    main_func.register_count = 16;
    module.functions.push(main_func);

    let mut gen_func = FunctionDescriptor::new(StringId::EMPTY);
    gen_func.id = FunctionId(1);
    gen_func.bytecode_offset = main_bytecode.len() as u32;
    gen_func.bytecode_length = gen_bytecode.len() as u32;
    gen_func.register_count = 16;
    gen_func.is_generator = true;
    module.functions.push(gen_func);

    let mut combined_bytecode = main_bytecode;
    combined_bytecode.extend(gen_bytecode);
    module.bytecode = combined_bytecode;

    let mut interp = Interpreter::new(Arc::new(module));
    let result = interp.execute_function(FunctionId(0)).unwrap();

    // Should return true since generator is in Created state
    assert!(result.as_bool());
}

#[test]
fn test_gen_has_next_type_error() {
    // Test GenHasNext with non-generator value returns error
    //
    // Bytecode program:
    //   r0 = 42  // Not a generator
    //   r1 = gen_has_next(r0)  // Should fail
    //   ret r1
    let bytecode = encode_instructions(&[
        Instruction::LoadSmallI { dst: Reg(0), value: 42 },
        Instruction::GenHasNext { dst: Reg(1), generator: Reg(0) },
        Instruction::Ret { value: Reg(1) },
    ]);

    let module = create_module(bytecode);
    let mut interp = Interpreter::new(module);
    let result = interp.execute_function(FunctionId(0));

    // Should return a type mismatch error
    assert!(matches!(
        result,
        Err(InterpreterError::TypeMismatch { .. })
    ));
}

#[test]
fn test_bytecode_roundtrip_gen_create() {
    // Verify GenCreate bytecode encode/decode roundtrip
    let original = Instruction::GenCreate { dst: Reg(5), func_id: 42, args: RegRange { start: Reg(0), count: 2 } };
    let mut encoded = Vec::new();
    bytecode::encode_instruction(&original, &mut encoded);

    let mut offset = 0;
    let decoded = bytecode::decode_instruction(&encoded, &mut offset).unwrap();

    assert!(matches!(
        decoded,
        Instruction::GenCreate { dst: Reg(5), func_id: 42, args: RegRange { start: Reg(0), count: 2 } }
    ));
}

#[test]
fn test_bytecode_roundtrip_gen_next() {
    // Verify GenNext bytecode encode/decode roundtrip
    let original = Instruction::GenNext { dst: Reg(3), generator: Reg(7) };
    let mut encoded = Vec::new();
    bytecode::encode_instruction(&original, &mut encoded);

    let mut offset = 0;
    let decoded = bytecode::decode_instruction(&encoded, &mut offset).unwrap();

    assert!(matches!(
        decoded,
        Instruction::GenNext { dst: Reg(3), generator: Reg(7) }
    ));
}

#[test]
fn test_bytecode_roundtrip_gen_has_next() {
    // Verify GenHasNext bytecode encode/decode roundtrip
    let original = Instruction::GenHasNext { dst: Reg(0), generator: Reg(1) };
    let mut encoded = Vec::new();
    bytecode::encode_instruction(&original, &mut encoded);

    let mut offset = 0;
    let decoded = bytecode::decode_instruction(&encoded, &mut offset).unwrap();

    assert!(matches!(
        decoded,
        Instruction::GenHasNext { dst: Reg(0), generator: Reg(1) }
    ));
}

#[test]
fn test_opcode_values() {
    // Verify the opcode byte values match expected assignments
    assert_eq!(Opcode::GenCreate as u8, 0xC2);
    assert_eq!(Opcode::GenNext as u8, 0xC3);
    assert_eq!(Opcode::GenHasNext as u8, 0xC4);
}

#[test]
fn test_opcode_mnemonics() {
    // Verify opcode mnemonics are correct
    assert_eq!(Opcode::GenCreate.mnemonic(), "GEN_CREATE");
    assert_eq!(Opcode::GenNext.mnemonic(), "GEN_NEXT");
    assert_eq!(Opcode::GenHasNext.mnemonic(), "GEN_HAS_NEXT");
}
